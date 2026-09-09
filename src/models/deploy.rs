use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::Db;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Deploy {
    pub id: i64,
    pub project_id: i64,
    pub commit_sha: Option<String>,
    pub commit_msg: Option<String>,
    pub trigger: String,
    pub status: String,
    pub log_tail: Option<String>,
    pub log_path: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub is_rollback: bool,
    pub error: Option<String>,
}

impl Deploy {
    pub fn is_terminal(&self) -> bool {
        self.status == "succeeded" || self.status == "failed"
    }

    pub fn is_in_flight(&self) -> bool {
        !self.is_terminal()
    }
}

impl Db {
    pub async fn create_deploy(
        &self,
        project_id: i64,
        trigger: &str,
        is_rollback: bool,
    ) -> anyhow::Result<i64> {
        let now = crate::db::now_rfc3339();
        let result = sqlx::query(
            r#"INSERT INTO deploys (project_id, trigger, status, started_at, is_rollback)
            VALUES (?, ?, 'queued', ?, ?)"#,
        )
        .bind(project_id)
        .bind(trigger)
        .bind(&now)
        .bind(is_rollback)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_deploy(&self, id: i64) -> anyhow::Result<Option<Deploy>> {
        let deploy = sqlx::query_as::<_, Deploy>("SELECT * FROM deploys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(deploy)
    }

    pub async fn update_deploy_status(
        &self,
        id: i64,
        status: &str,
        log_tail: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE deploys SET status = ?, log_tail = ?, error = ? WHERE id = ?",
        )
        .bind(status)
        .bind(log_tail)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_deploy_log_path(&self, id: i64, path: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE deploys SET log_path = ? WHERE id = ?")
            .bind(path)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_deploy_commit(
        &self,
        id: i64,
        sha: &str,
        msg: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE deploys SET commit_sha = ?, commit_msg = ? WHERE id = ?")
            .bind(sha)
            .bind(msg)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn finish_deploy(
        &self,
        id: i64,
        status: &str,
        log_tail: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = crate::db::now_rfc3339();
        sqlx::query(
            "UPDATE deploys SET status = ?, log_tail = ?, error = ?, finished_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(log_tail)
        .bind(error)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Latest deploys across all projects, paginated.
    pub async fn list_deploys_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<DeployWithProject>> {
        let rows = sqlx::query_as::<_, DeployWithProject>(
            r#"SELECT d.*, p.display_name as project_name, p.slug as project_slug
            FROM deploys d
            JOIN projects p ON p.id = d.project_id
            ORDER BY d.started_at DESC
            LIMIT ? OFFSET ?"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count total deploys for pagination.
    pub async fn count_deploys(&self) -> anyhow::Result<i64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deploys")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Last N deploys for the dashboard.
    pub async fn list_recent_deploys(&self, limit: i64) -> anyhow::Result<Vec<DeployWithProject>> {
        let rows = sqlx::query_as::<_, DeployWithProject>(
            r#"SELECT d.*, p.display_name as project_name, p.slug as project_slug
            FROM deploys d
            JOIN projects p ON p.id = d.project_id
            ORDER BY d.started_at DESC
            LIMIT ?"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Deploys for a specific project, paginated.
    pub async fn list_project_deploys(
        &self,
        project_id: i64,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<Deploy>> {
        let rows = sqlx::query_as::<_, Deploy>(
            r#"SELECT * FROM deploys
            WHERE project_id = ?
            ORDER BY started_at DESC
            LIMIT ? OFFSET ?"#,
        )
        .bind(project_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Succeeded deploys with distinct commit SHAs — for the rollback picker.
    pub async fn list_rollback_targets(
        &self,
        project_id: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<Deploy>> {
        let rows = sqlx::query_as::<_, Deploy>(
            r#"SELECT * FROM deploys
            WHERE project_id = ? AND status = 'succeeded' AND commit_sha IS NOT NULL
            GROUP BY commit_sha
            ORDER BY started_at DESC
            LIMIT ?"#,
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get the latest succeeded deploy's commit_sha for a project (auto-start optimization).
    pub async fn last_succeeded_commit(&self, project_id: i64) -> anyhow::Result<Option<String>> {
        let sha: Option<String> = sqlx::query_scalar(
            r#"SELECT commit_sha FROM deploys
            WHERE project_id = ? AND status = 'succeeded' AND commit_sha IS NOT NULL
            ORDER BY started_at DESC LIMIT 1"#,
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(sha)
    }
}

/// Deploy row joined with project info for list views.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DeployWithProject {
    pub id: i64,
    pub project_id: i64,
    pub commit_sha: Option<String>,
    pub commit_msg: Option<String>,
    pub trigger: String,
    pub status: String,
    pub log_tail: Option<String>,
    pub log_path: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub is_rollback: bool,
    pub error: Option<String>,
    pub project_name: String,
    pub project_slug: String,
}
