mod config;
mod db;
mod crypto;
mod auth;
mod models;
mod docker;
mod deploy;
mod webhooks;
mod routes;
mod templates;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::db::Db;

#[derive(Parser)]
#[command(name = "rusno", version, about = "Self-hosted deployment manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize rusno: create ~/.rusno/, config, and database
    Init {
        #[arg(long)]
        admin_password: Option<String>,
        #[arg(long)]
        projects_root: Option<String>,
        #[arg(long, default_value = "6967")]
        port: u16,
    },
    /// Run database migrations
    Migrate,
    /// Start the web server
    Serve {
        #[arg(long, default_value = "6967")]
        port: u16,
    },
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<AppConfig>,
    pub master_key: Arc<crypto::MasterKey>,
    pub deploy_manager: Arc<deploy::DeployManager>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("rusno=debug,info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            admin_password,
            projects_root,
            port,
        } => {
            cmd_init(admin_password, projects_root, port).await?;
        }
        Commands::Migrate => {
            cmd_migrate().await?;
        }
        Commands::Serve { port } => {
            cmd_serve(port).await?;
        }
    }

    Ok(())
}

async fn cmd_init(
    admin_password: Option<String>,
    projects_root: Option<String>,
    port: u16,
) -> Result<()> {
    let data_dir = config::data_dir();
    let rusno_home = data_dir.join(".rusno");

    // Create directory structure
    for dir in &["keys", "ssh", "projects"] {
        let p = rusno_home.join(dir);
        if !p.exists() {
            tokio::fs::create_dir_all(&p)
                .await
                .with_context(|| format!("creating dir {}", p.display()))?;
        }
    }

    // Write config.toml if missing
    let config_path = rusno_home.join("config.toml");
    if !config_path.exists() {
        let projects_root = projects_root
            .unwrap_or_else(|| rusno_home.join("projects").to_string_lossy().to_string());
        let toml = format!(
            r#"port = {port}
projects_root = "{projects_root}"
ssh_mode = "rusno-managed"
data_dir = "{}"
"#,
            rusno_home.display()
        );
        tokio::fs::write(&config_path, toml).await?;
    }

    // Generate master key if missing
    let key_path = rusno_home.join("keys").join("master.key");
    if !key_path.exists() {
        let key = crypto::MasterKey::generate();
        tokio::fs::write(&key_path, key.as_bytes()).await?;
        set_file_mode_0600(&key_path).await?;
    }

    // Run migrations
    let db = Db::connect(&rusno_home.join("rusno.db")).await?;
    db.run_migrations().await?;

    // Seed default settings
    db.seed_defaults().await?;

    // Set admin password if provided
    if let Some(pw) = admin_password {
        let hash = crypto::hash_password(&pw)?;
        db.set_setting("admin_password_hash", &hash).await?;
    }

    // Generate session secret if missing
    let existing = db.get_setting("session_secret").await?;
    if existing.is_none() {
        let secret = crypto::random_base64url(32);
        db.set_setting("session_secret", &secret).await?;
    }

    println!("rusno initialized at {}", rusno_home.display());
    println!("config: {}", config_path.display());
    println!("database: {}", rusno_home.join("rusno.db").display());
    println!("master key: {}", key_path.display());
    println!();
    println!("Run `rusno serve` to start the server.");

    Ok(())
}

async fn cmd_migrate() -> Result<()> {
    let data_dir = config::data_dir();
    let rusno_home = data_dir.join(".rusno");
    if !rusno_home.exists() {
        anyhow::bail!(
            "rusno home not found at {}. Run `rusno init` first.",
            rusno_home.display()
        );
    }

    let db = Db::connect(&rusno_home.join("rusno.db")).await?;
    db.run_migrations().await?;
    println!("Migrations complete.");
    Ok(())
}

async fn cmd_serve(port: u16) -> Result<()> {
    // Ensure initialized
    let data_dir = config::data_dir();
    let rusno_home = data_dir.join(".rusno");
    if !rusno_home.exists() {
        anyhow::bail!(
            "rusno home not found at {}. Run `rusno init` first.",
            rusno_home.display()
        );
    }

    // Load config
    let mut app_config = AppConfig::load(&rusno_home)?;
    // CLI port overrides
    app_config.port = port;

    // Connect DB + migrate
    let db = Db::connect(&rusno_home.join("rusno.db")).await?;
    db.run_migrations().await?;
    db.seed_defaults().await?;

    // Crash recovery: mark non-terminal deploys as failed
    db.recover_crashed_deploys().await?;

    // Load master key
    let key_path = rusno_home.join("keys").join("master.key");
    let master_key = if key_path.exists() {
        let raw = tokio::fs::read(&key_path).await?;
        crypto::MasterKey::from_bytes(&raw)
    } else {
        let key = crypto::MasterKey::generate();
        tokio::fs::write(&key_path, key.as_bytes()).await?;
        set_file_mode_0600(&key_path).await?;
        key
    };

    // Start deploy manager
    let deploy_manager = deploy::DeployManager::new(db.clone(), app_config.clone()).await?;

    // Enqueue auto-start projects
    deploy_manager.enqueue_auto_start().await?;

    let state = AppState {
        db,
        config: Arc::new(app_config),
        master_key: Arc::new(master_key),
        deploy_manager: Arc::new(deploy_manager),
    };

    // Build router
    let app = routes::build_router(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("rusno listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(unix)]
async fn set_file_mode_0600(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = tokio::fs::metadata(path).await?.permissions();
    perms.set_mode(0o600);
    tokio::fs::set_permissions(path, perms).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn set_file_mode_0600(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
