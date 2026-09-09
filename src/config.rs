//! Bootstrap configuration for rusno.
//!
//! Nearly all rusno state lives in SQLite; the only file-based config is the
//! minimal `config.toml` in the rusno home directory (`~/.rusno/`), read
//! exactly once on startup:
//!
//! ```toml
//! port = 6967
//! projects_root = "~/rusno/projects"
//! ssh_mode = "rusno-managed"   # or "host-existing"
//! data_dir = "~/.rusno"
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use config::{Config, File, FileFormat};
use serde::Deserialize;

/// Default port the web server listens on.
const DEFAULT_PORT: u16 = 6967;

/// Default SSH mode: rusno generates and manages its own keys.
const DEFAULT_SSH_MODE: &str = "rusno-managed";

/// Return the user's home directory — the parent of the `.rusno/` directory.
///
/// Falls back to the current directory in the (practically impossible) case
/// that the home directory cannot be determined.
pub fn data_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The minimal bootstrap configuration loaded from `~/.rusno/config.toml`.
///
/// Only the handful of values needed to bring the server up live here; any
/// setting that can change at runtime belongs in the database instead.
///
/// Every field is a plain owned value, so `AppConfig` is automatically
/// `Send + Sync` and can be freely shared behind an `Arc`.
#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    /// Port the web server listens on.
    pub port: u16,
    /// Root directory under which project repositories are cloned.
    /// A leading `~/` is expanded to the home directory on load.
    pub projects_root: String,
    /// SSH handling mode: `"rusno-managed"` or `"host-existing"`.
    pub ssh_mode: String,
    /// The rusno home directory (e.g. `/home/user/.rusno`).
    pub data_dir: String,
}

impl AppConfig {
    /// Load the configuration from `config.toml` inside `rusno_home`.
    ///
    /// If the file does not exist (e.g. `rusno serve` runs before
    /// `rusno init`), defaults derived from `rusno_home` are returned so the
    /// server can still boot. A partial file is also fine: any key missing
    /// from the file falls back to its default. Leading `~/` segments in
    /// `projects_root` and `data_dir` are expanded to the real home directory.
    pub fn load(rusno_home: &Path) -> Result<AppConfig> {
        let config_path = rusno_home.join("config.toml");

        // Seed the builder with defaults so a missing or partial file still
        // yields a complete, usable config.
        let mut builder = Config::builder()
            .set_default("port", DEFAULT_PORT)
            .context("setting default port")?
            .set_default(
                "projects_root",
                rusno_home.join("projects").to_string_lossy().into_owned(),
            )
            .context("setting default projects_root")?
            .set_default("ssh_mode", DEFAULT_SSH_MODE)
            .context("setting default ssh_mode")?
            .set_default("data_dir", rusno_home.to_string_lossy().into_owned())
            .context("setting default data_dir")?;

        if config_path.exists() {
            tracing::debug!(path = %config_path.display(), "loading config.toml");
            // Read the file ourselves and parse the string so we control
            // exactly which file is used (avoids `File::with_name`'s
            // extension-searching behaviour).
            let contents = std::fs::read_to_string(&config_path)
                .with_context(|| format!("reading {}", config_path.display()))?;
            builder = builder.add_source(File::from_str(&contents, FileFormat::Toml));
        } else {
            tracing::debug!(
                path = %config_path.display(),
                "config.toml not found, using defaults"
            );
        }

        let mut config: AppConfig = builder
            .build()
            .with_context(|| format!("building config from {}", config_path.display()))?
            .try_deserialize()
            .with_context(|| format!("invalid config in {}", config_path.display()))?;

        // Expand `~/` in path-valued fields to the real home directory.
        config.projects_root = expand_tilde(&config.projects_root);
        config.data_dir = expand_tilde(&config.data_dir);

        Ok(config)
    }
}

/// Replace a leading `~/` in `path` with the user's home directory.
///
/// Paths that do not start with `~/` are returned unchanged.
fn expand_tilde(path: &str) -> String {
    path.strip_prefix("~/")
        .and_then(|rest| dirs::home_dir().map(|home| home.join(rest)))
        .map(|expanded| expanded.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

// `AppConfig` is shared behind an `Arc` in `AppState` and must stay
// thread-safe. All fields are plain owned values, so `Send + Sync` hold
// automatically — assert it so a future field cannot silently regress this.
#[allow(dead_code)]
fn _assert_app_config_thread_safe() {
    fn check<T: Send + Sync>() {}
    check::<AppConfig>();
}
