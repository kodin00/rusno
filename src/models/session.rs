use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::Db;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
    pub csrf_token: String,
}

impl Db {
    pub async fn create_session(
        &self,
        session_id: &str,
        csrf_token: &str,
        ttl_secs: i64,
    ) -> anyhow::Result<()> {
        let now = crate::db::now_rfc3339();
        let expires = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::seconds(ttl_secs))
            .map(|t| t.to_rfc3339())
            .unwrap_or(now.clone());

        sqlx::query(
            "INSERT INTO sessions (id, created_at, expires_at, csrf_token) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(&now)
        .bind(&expires)
        .bind(csrf_token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let session = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(session)
    }

    pub async fn touch_session(&self, session_id: &str, ttl_secs: i64) -> anyhow::Result<()> {
        let expires = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::seconds(ttl_secs))
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| crate::db::now_rfc3339());

        sqlx::query("UPDATE sessions SET expires_at = ? WHERE id = ?")
            .bind(&expires)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_sessions(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
