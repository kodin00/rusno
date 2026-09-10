//! SQLite database connection, migrations, and settings access.
//!
//! The database is a single SQLite file at `~/.rusno/rusno.db`, managed by
//! `sqlx`. WAL mode is enabled for concurrent reads during deploys. All
//! timestamps are stored as RFC 3339 strings in UTC.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tracing::{info, warn};

/// Wrapper around a `sqlx::SqlitePool`. Cheaply cloneable (the pool is
/// internally an `Arc`), so it can live in `AppState`.
#[derive(Clone)]
pub struct Db {
    pub(crate) pool: SqlitePool,
}

impl Db {
    /// Connect to the SQLite database at `db_path`, enabling WAL mode for
    /// concurrent reads during deploys.
    pub async fn connect(db_path: &Path) -> Result<Db> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            // WAL allows readers to coexist with a writer (e.g. deploys).
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // `busy_timeout` smooths over brief write contention.
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .with_context(|| format!("connecting to sqlite database at {}", db_path.display()))?;

        // Belt-and-suspenders: ensure WAL is on for this connection's writes
        // (the connect option above handles new pools, but an existing file's
        // journal mode persists from prior sessions; this forces it).
        sqlx::query("PRAGMA journal_mode=WAL;")
            .execute(&pool)
            .await
            .context("enabling WAL journal mode")?;

        // Foreign keys are off by default in SQLite; the schema uses ON DELETE
        // CASCADE, so enforce them per-connection.
        sqlx::query("PRAGMA foreign_keys=ON;")
            .execute(&pool)
            .await
            .context("enabling foreign keys")?;

        Ok(Db { pool })
    }

    /// Run pending sqlx migrations from the `migrations/` directory.
    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("running database migrations")?;
        info!("database migrations complete");
        Ok(())
    }

    /// Insert default settings if missing. Idempotent — safe to call on every
    /// startup.
    pub async fn seed_defaults(&self) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)")
            .bind("deploy_concurrency")
            .bind("2")
            .execute(&self.pool)
            .await
            .context("seeding default deploy_concurrency")?;

        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)")
            .bind("default_health_timeout_secs")
            .bind("60")
            .execute(&self.pool)
            .await
            .context("seeding default default_health_timeout_secs")?;

        Ok(())
    }

    /// Read a setting value by key. Returns `None` if the key is absent.
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("reading setting {:?}", key))?;
        Ok(row.map(|(v,)| v))
    }

    /// Upsert a setting (INSERT OR REPLACE on the primary key).
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .with_context(|| format!("writing setting {:?}", key))?;
        Ok(())
    }

    /// Mark all deploys left in a non-terminal state (queued/pulling/building/
    /// starting) as `failed` with an error indicating rusno restarted
    /// mid-deploy. Runs on startup so the UI never shows a stale "in-progress"
    /// row after a crash or restart.
    pub async fn recover_crashed_deploys(&self) -> Result<()> {
        let now = now_rfc3339();
        let result = sqlx::query(
            "UPDATE deploys
             SET status = 'failed',
                 error = 'rusno restarted mid-deploy',
                 finished_at = ?1
             WHERE status IN ('queued', 'pulling', 'building', 'starting')",
        )
        .bind(&now)
        .execute(&self.pool)
        .await
        .context("recovering crashed deploys")?;

        let affected = result.rows_affected();
        if affected > 0 {
            warn!("recovered {affected} crashed deploy(s) marked as failed");
        } else {
            info!("no crashed deploys to recover");
        }
        Ok(())
    }
}

/// Return the current time as an RFC 3339 string in UTC, e.g.
/// `2026-09-09T12:34:56.789+00:00`. All timestamps in the schema are stored
/// in this format (TEXT columns).
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}
