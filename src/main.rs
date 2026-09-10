mod auth;
mod config;
mod crypto;
mod db;
mod deploy;
mod docker;
mod models;
mod routes;
mod templates;
mod webhooks;

use std::net::SocketAddr;
use std::path::PathBuf;
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
    /// Install / manage the systemd service (auto-start on boot)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Write the systemd unit, enable it, and start rusno now + on boot
    Install {
        #[arg(long, default_value = "6967")]
        port: u16,
    },
    /// Stop, disable, and remove the systemd unit
    Uninstall,
    /// Show `systemctl status rusno`
    Status,
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
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("rusno=debug,info")),
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
        Commands::Service { action } => {
            cmd_service(action).await?;
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

// ---------------------------------------------------------------------------
// `rusno service` — systemd management
// ---------------------------------------------------------------------------

/// Resolve the numeric UID of the current process via `id -u`.
///
/// Returns `None` if `id` is missing or its output is not a number — neither
/// should happen on a normal Linux system.
fn current_uid() -> Option<u32> {
    let output = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

/// The user that *invoked* rusno: `SUDO_USER` when run under sudo, else
/// `$USER`, else the output of `id -un`.
fn invoking_user() -> String {
    if let Ok(u) = std::env::var("SUDO_USER") {
        if !u.is_empty() {
            return u;
        }
    }
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    match std::process::Command::new("id").arg("-un").output() {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => "root".to_string(),
    }
}

/// Resolve a user's home directory from the passwd database via
/// `getent passwd <user>`. Returns `None` if `getent` is missing or the
/// user is unknown. Falls back to `dirs::home_dir()` when the requested
/// user is the invoking user (covers `sudo rusno service install` on hosts
/// without `getent`).
fn user_home(user: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("getent")
        .args(["passwd", user])
        .output()
        .ok()?;
    if !output.status.success() {
        if user == invoking_user().as_str() {
            return dirs::home_dir();
        }
        return None;
    }
    // passwd entry: name:passwd:uid:gid:gecos:home:shell
    String::from_utf8_lossy(&output.stdout)
        .split(':')
        .nth(5)
        .map(PathBuf::from)
}

/// Absolute path to the running rusno binary, following symlinks.
fn current_binary() -> Result<PathBuf> {
    std::env::current_exe().context("resolving rusno binary path")
}

/// Run `systemctl <args>` and return the exit status.
fn systemctl(args: &[&str]) -> Result<std::process::ExitStatus> {
    std::process::Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("running systemctl {}", args.join(" ")))
}

/// `rusno service install|uninstall|status`.
async fn cmd_service(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install { port } => service_install(port).await,
        ServiceAction::Uninstall => service_uninstall().await,
        ServiceAction::Status => service_status().await,
    }
}

/// Write the systemd unit, enable it, and start rusno now + on boot.
async fn service_install(port: u16) -> Result<()> {
    // systemd must be present.
    let has_systemctl = std::process::Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_systemctl {
        anyhow::bail!(
            "systemd not found; `rusno serve` must be run under your own \
             process supervisor (e.g. nohup, tmux, or systemd on a Linux VPS)."
        );
    }

    // Writing under /etc/systemd and calling enable/restart needs root.
    if current_uid() != Some(0) {
        anyhow::bail!(
            "rusno service install must run as root. Try: sudo rusno service install"
        );
    }

    let user = invoking_user();
    let home = user_home(&user)
        .with_context(|| format!("could not resolve home directory for user '{user}'"))?;
    let rusno_home = home.join(".rusno");
    let bin = current_binary()?;

    // Best-effort projects_root from config; falls back to <rusno_home>/projects.
    let projects_root = AppConfig::load(&rusno_home)
        .ok()
        .map(|c| c.projects_root)
        .unwrap_or_else(|| rusno_home.join("projects").to_string_lossy().into_owned());

    let unit = format!(
        r#"[Unit]
Description=rusno — self-hosted deployment manager
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
Type=simple
User={user}
Group=docker
ExecStart={bin} serve --port {port}
Restart=on-failure
RestartSec=5
Environment=RUSNO_HOME={rusno_home}
WorkingDirectory={home}

# Hardening (deliberately no ProtectHome so host-existing SSH keys in ~/.ssh remain readable)
NoNewPrivileges=true
ProtectSystem=full
PrivateTmp=true
ReadWritePaths={rusno_home} {projects_root}

[Install]
WantedBy=multi-user.target
"#,
        bin = bin.display(),
        rusno_home = rusno_home.display(),
        home = home.display(),
    );

    let unit_path = PathBuf::from("/etc/systemd/system/rusno.service");
    std::fs::write(&unit_path, &unit)
        .with_context(|| format!("writing {}", unit_path.display()))?;
    tracing::info!(path = %unit_path.display(), "wrote systemd unit");

    for step in &["daemon-reload", "enable rusno", "restart rusno"] {
        let args: Vec<&str> = step.split_whitespace().collect();
        let status = systemctl(&args)?;
        if !status.success() {
            anyhow::bail!("`systemctl {step}` failed (exit {status})");
        }
    }

    println!("rusno service installed and started.");
    println!("Listening on http://localhost:{port} (check: sudo systemctl status rusno)");
    println!(
        "IMPORTANT: Back up your master key at {}/keys/master.key",
        rusno_home.display()
    );
    println!("           Losing it means all encrypted secrets are unrecoverable.");
    Ok(())
}

/// Stop, disable, and remove the systemd unit.
async fn service_uninstall() -> Result<()> {
    if current_uid() != Some(0) {
        anyhow::bail!(
            "rusno service uninstall must run as root. Try: sudo rusno service uninstall"
        );
    }
    // stop/disable are fine if the service was never installed.
    for step in &["stop rusno", "disable rusno"] {
        let args: Vec<&str> = step.split_whitespace().collect();
        let _ = systemctl(&args);
    }
    let unit_path = PathBuf::from("/etc/systemd/system/rusno.service");
    match std::fs::remove_file(&unit_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => anyhow::bail!("removing {}: {e}", unit_path.display()),
    }
    let _ = systemctl(&["daemon-reload"]);
    println!("rusno service removed.");
    Ok(())
}

/// Show `systemctl status rusno` (exit status is intentionally ignored).
async fn service_status() -> Result<()> {
    let _ = std::process::Command::new("systemctl")
        .args(["status", "rusno"])
        .status();
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
