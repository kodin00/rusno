use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::Db;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub folder_name: String,
    pub folder_path: String,
    pub source_type: String,
    pub source_url: String,
    pub branch: String,
    pub compose_path: String,
    pub compose_command: String,
    pub auto_start: bool,
    pub webhook_secret: String,
    pub webhook_enabled: bool,
    pub branch_filter: bool,
    pub health_timeout_secs: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Project {
    /// Derive a URL-safe slug from a folder name: lowercase, [a-z0-9-].
    pub fn derive_slug(folder_name: &str) -> String {
        folder_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    }

    /// Detect source type from a git URL.
    pub fn detect_source_type(url: &str) -> String {
        if url.starts_with("git@") || url.starts_with("ssh://") {
            "git_ssh".to_string()
        } else {
            "git_https".to_string()
        }
    }

    /// Extract the repo name from a git URL (last segment minus .git).
    pub fn repo_name(url: &str) -> String {
        let trimmed = url.trim_end_matches(".git");
        let last = trimmed
            .rsplit(|c| c == '/' || c == ':')
            .next()
            .unwrap_or(trimmed);
        last.to_string()
    }
}

impl Db {
    pub async fn create_project(&self, p: &NewProject) -> anyhow::Result<Project> {
        let now = crate::db::now_rfc3339();
        let slug = self.unique_slug(&p.folder_name).await?;

        let project = sqlx::query_as::<_, Project>(
            r#"INSERT INTO projects
            (slug, display_name, folder_name, folder_path, source_type, source_url,
             branch, compose_path, compose_command, auto_start, webhook_secret,
             webhook_enabled, branch_filter, health_timeout_secs, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *"#,
        )
        .bind(&slug)
        .bind(&p.display_name)
        .bind(&p.folder_name)
        .bind(&p.folder_path)
        .bind(&p.source_type)
        .bind(&p.source_url)
        .bind(&p.branch)
        .bind(&p.compose_path)
        .bind(&p.compose_command)
        .bind(p.auto_start)
        .bind(&p.webhook_secret)
        .bind(p.webhook_enabled)
        .bind(p.branch_filter)
        .bind(p.health_timeout_secs)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await?;

        Ok(project)
    }

    /// Generate a unique slug, appending -2, -3, etc. on collision.
    async fn unique_slug(&self, folder_name: &str) -> anyhow::Result<String> {
        let base = Project::derive_slug(folder_name);
        let mut slug = base.clone();
        let mut suffix = 2u32;
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE slug = ?)")
                    .bind(&slug)
                    .fetch_one(&self.pool)
                    .await?;
            if !exists {
                return Ok(slug);
            }
            slug = format!("{}-{}", base, suffix);
            suffix += 1;
        }
    }

    pub async fn get_project(&self, slug: &str) -> anyhow::Result<Option<Project>> {
        let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?;
        Ok(project)
    }

    pub async fn get_project_by_id(&self, id: i64) -> anyhow::Result<Option<Project>> {
        let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(project)
    }

    pub async fn list_projects(&self) -> anyhow::Result<Vec<Project>> {
        let projects =
            sqlx::query_as::<_, Project>("SELECT * FROM projects ORDER BY display_name ASC")
                .fetch_all(&self.pool)
                .await?;
        Ok(projects)
    }

    pub async fn delete_project(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_project(&self, id: i64, p: &Project) -> anyhow::Result<Project> {
        let now = crate::db::now_rfc3339();
        let project = sqlx::query_as::<_, Project>(
            r#"UPDATE projects SET
            display_name = ?, folder_name = ?, folder_path = ?, branch = ?,
            compose_path = ?, compose_command = ?, auto_start = ?,
            webhook_enabled = ?, branch_filter = ?, health_timeout_secs = ?,
            updated_at = ?
            WHERE id = ? RETURNING *"#,
        )
        .bind(&p.display_name)
        .bind(&p.folder_name)
        .bind(&p.folder_path)
        .bind(&p.branch)
        .bind(&p.compose_path)
        .bind(&p.compose_command)
        .bind(p.auto_start)
        .bind(p.webhook_enabled)
        .bind(p.branch_filter)
        .bind(p.health_timeout_secs)
        .bind(&now)
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(project)
    }
}

/// Fields for creating a new project (used by the register form).
#[derive(Debug, Clone)]
pub struct NewProject {
    pub display_name: String,
    pub folder_name: String,
    pub folder_path: String,
    pub source_type: String,
    pub source_url: String,
    pub branch: String,
    pub compose_path: String,
    pub compose_command: String,
    pub auto_start: bool,
    pub webhook_secret: String,
    pub webhook_enabled: bool,
    pub branch_filter: bool,
    pub health_timeout_secs: i64,
}
