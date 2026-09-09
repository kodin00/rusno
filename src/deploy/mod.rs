pub mod git;
pub mod worker;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, Semaphore};

use crate::config::AppConfig;
use crate::db::Db;
use crate::deploy::worker::WorkerContext;

/// What kind of deploy to run.
#[derive(Debug, Clone)]
pub enum DeployRequest {
    /// Normal deploy: fetch + checkout branch + pull + compose up.
    Normal {
        project_id: i64,
        trigger: String,
    },
    /// Rollback: fetch + checkout specific commit + compose up.
    Rollback {
        project_id: i64,
        commit_sha: String,
    },
}

/// Global deploy scheduler + per-project single-worker dispatch.
pub struct DeployManager {
    ctx: WorkerContext,
    /// Per-project worker handles. Each project gets a single-worker mpsc channel.
    workers: Mutex<HashMap<i64, worker::ProjectWorkerHandle>>,
    /// Cached concurrency cap so we can detect setting changes.
    concurrency: Mutex<usize>,
}

impl DeployManager {
    pub async fn new(db: Db, config: AppConfig) -> Result<Self> {
        let concurrency = db
            .get_setting("deploy_concurrency")
            .await?
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);

        let semaphore = Arc::new(Semaphore::new(concurrency));
        let config = Arc::new(config);

        Ok(Self {
            ctx: WorkerContext {
                db: db.clone(),
                config: config.clone(),
                semaphore: semaphore.clone(),
            },
            workers: Mutex::new(HashMap::new()),
            concurrency: Mutex::new(concurrency),
        })
    }

    /// Refresh the semaphore size if the setting changed. Grows only (shrinking
    /// a live semaphore doesn't reclaim permits; a restart applies reductions).
    pub async fn refresh_concurrency(&self) -> Result<()> {
        let target = self
            .ctx
            .db
            .get_setting("deploy_concurrency")
            .await?
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);

        let mut current = self.concurrency.lock().await;
        if target > *current {
            self.ctx.semaphore.add_permits(target - *current);
            *current = target;
            tracing::info!("deploy concurrency raised to {target}");
        }
        Ok(())
    }

    /// Enqueue a deploy for a project. Creates the worker if it doesn't exist.
    /// If a deploy is already running for the project, this coalesces into the
    /// single pending slot (never cancels the running one).
    pub async fn enqueue(&self, request: DeployRequest) -> Result<i64> {
        let project_id = match &request {
            DeployRequest::Normal { project_id, .. } => *project_id,
            DeployRequest::Rollback { project_id, .. } => *project_id,
        };

        // Create the deploy row first (queued state)
        let (trigger, is_rollback) = match &request {
            DeployRequest::Normal { trigger, .. } => (trigger.clone(), false),
            DeployRequest::Rollback { .. } => ("rollback".to_string(), true),
        };
        let deploy_id = self
            .ctx
            .db
            .create_deploy(project_id, &trigger, is_rollback)
            .await?;

        // Get or create the worker
        let mut workers = self.workers.lock().await;
        let worker = workers
            .entry(project_id)
            .or_insert_with(|| worker::ProjectWorkerHandle::new(project_id, self.ctx.clone()));

        worker.enqueue(request, deploy_id).await;

        tracing::info!(project_id, deploy_id, "deploy enqueued (trigger={trigger})");
        Ok(deploy_id)
    }

    /// On startup: enqueue restart deploys for auto_start projects.
    pub async fn enqueue_auto_start(&self) -> Result<()> {
        let projects = self.ctx.db.list_projects().await?;
        let mut count = 0;
        for project in projects {
            if project.auto_start {
                self.enqueue(DeployRequest::Normal {
                    project_id: project.id,
                    trigger: "restart".to_string(),
                })
                .await?;
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!("enqueued {count} auto-start project(s)");
        }
        Ok(())
    }

    /// Check whether a project currently has a deploy in flight.
    pub async fn is_deploying(&self, project_id: i64) -> Result<bool> {
        let workers = self.workers.lock().await;
        Ok(workers
            .get(&project_id)
            .map(|w| w.is_busy())
            .unwrap_or(false))
    }

    pub fn db(&self) -> &Db {
        &self.ctx.db
    }

    pub fn config(&self) -> &AppConfig {
        &self.ctx.config
    }

    pub fn semaphore(&self) -> &Arc<Semaphore> {
        &self.ctx.semaphore
    }
}
