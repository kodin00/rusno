use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tracing::{error, info, warn};

use crate::config::AppConfig;
use crate::db::Db;
use crate::deploy::git::GitOps;
use crate::deploy::DeployRequest;
use crate::docker::DockerClient;

/// Everything a worker needs to run a deploy.
#[derive(Clone)]
pub struct WorkerContext {
    pub db: Db,
    pub config: Arc<AppConfig>,
    pub semaphore: Arc<Semaphore>,
}

/// Handle to a per-project worker. Each project gets one of these —
/// a single-worker mpsc channel that coalesces pending requests.
pub struct ProjectWorkerHandle {
    tx: mpsc::Sender<(DeployRequest, i64)>,
    /// Track whether this worker is currently running a deploy.
    busy: Arc<Mutex<bool>>,
}

impl ProjectWorkerHandle {
    pub fn new(project_id: i64, ctx: WorkerContext) -> Self {
        let (tx, rx) = mpsc::channel::<(DeployRequest, i64)>(8);
        let busy = Arc::new(Mutex::new(false));

        tokio::spawn(worker_loop(project_id, ctx, rx, busy.clone()));

        Self { tx, busy }
    }

    /// Enqueue a deploy request. The worker coalesces: if busy and
    /// multiple requests arrive, only the latest survives.
    pub async fn enqueue(&self, request: DeployRequest, deploy_id: i64) {
        // Channel capacity is small; if it's full the request is dropped —
        // acceptable since coalescing means only the latest matters anyway.
        let _ = self.tx.try_send((request, deploy_id));
    }

    pub fn is_busy(&self) -> bool {
        self.busy.try_lock().map(|g| *g).unwrap_or(true)
    }
}

/// The worker loop: receives requests, coalesces if busy, runs the deploy.
async fn worker_loop(
    project_id: i64,
    ctx: WorkerContext,
    mut rx: mpsc::Receiver<(DeployRequest, i64)>,
    busy: Arc<Mutex<bool>>,
) {
    while let Some((request, deploy_id)) = rx.recv().await {
        // Drain any additional pending requests — keep only the latest.
        let mut latest = (request, deploy_id);
        while let Ok(extra) = rx.try_recv() {
            let (_, dropped_id) = std::mem::replace(&mut latest, extra);
            warn!(project_id, dropped_id, "deploy coalesced (dropped)");
            // The dropped deploy row is marked failed so it doesn't linger as queued.
            let _ = ctx
                .db
                .finish_deploy(
                    dropped_id,
                    "failed",
                    None,
                    Some("coalesced into a newer deploy"),
                )
                .await;
        }

        {
            let mut b = busy.lock().await;
            *b = true;
        }

        let (req, id) = latest;
        if let Err(e) = run_deploy(&ctx, project_id, id, req).await {
            error!(project_id, deploy_id = id, "deploy worker error: {e:#}");
        }

        {
            let mut b = busy.lock().await;
            *b = false;
        }
    }
}

/// Run a single deploy through the full state machine:
/// queued → pulling → building → starting → healthy → succeeded (or failed).
async fn run_deploy(
    ctx: &WorkerContext,
    project_id: i64,
    deploy_id: i64,
    request: DeployRequest,
) -> Result<()> {
    let db = &ctx.db;

    let project = match db.get_project_by_id(project_id).await? {
        Some(p) => p,
        None => {
            db.finish_deploy(deploy_id, "failed", None, Some("project not found"))
                .await?;
            return Ok(());
        }
    };

    let git = GitOps::new(&ctx.config);

    // 1. Acquire global permit (waits here while status stays 'queued')
    let _permit = ctx
        .semaphore
        .acquire()
        .await
        .map_err(|e| anyhow::anyhow!("semaphore closed: {e}"))?;

    // Create log directory
    let log_dir = PathBuf::from(&project.folder_path).join("logs");
    tokio::fs::create_dir_all(&log_dir).await.ok();

    let log_path = log_dir.join(format!("{deploy_id}.log"));
    let mut log_file = tokio::fs::File::create(&log_path).await?;
    db.set_deploy_log_path(deploy_id, &log_path.to_string_lossy())
        .await?;

    let mut ring = RingBuffer::new(200);

    // 2. Git sync (pulling)
    db.update_deploy_status(deploy_id, "pulling", None, None)
        .await?;

    let folder_path = PathBuf::from(&project.folder_path);

    match &request {
        DeployRequest::Normal { trigger, .. } => {
            if !folder_path.exists() || !GitOps::is_repo(&folder_path) {
                match git.clone(&project.source_url, &folder_path).await {
                    Ok(out) => write_log(&mut log_file, &out).await,
                    Err(e) => return fail_deploy(db, deploy_id, &mut log_file, &mut ring, e).await,
                }
            } else {
                // restart optimization: skip pull when HEAD matches last succeeded deploy
                let skip_pull = if trigger == "restart" {
                    let head_ok = git.head_sha(&folder_path).await.ok();
                    if let Some(head) = head_ok {
                        db.last_succeeded_commit(project_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|l| l == head)
                            .unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !skip_pull {
                    match git.fetch_checkout_pull(&folder_path, &project.branch).await {
                        Ok(out) => write_log(&mut log_file, &out).await,
                        Err(e) => {
                            return fail_deploy(db, deploy_id, &mut log_file, &mut ring, e).await
                        }
                    }
                } else {
                    write_log(&mut log_file, "[restart] HEAD unchanged, skipping git pull").await;
                }
            }
        }
        DeployRequest::Rollback { commit_sha, .. } => {
            write_log(
                &mut log_file,
                &format!("[rollback] target commit {commit_sha}"),
            )
            .await;
            match git.fetch_checkout_commit(&folder_path, commit_sha).await {
                Ok(out) => write_log(&mut log_file, &out).await,
                Err(e) => return fail_deploy(db, deploy_id, &mut log_file, &mut ring, e).await,
            }
        }
    }

    // 3. Record commit
    let sha = git.head_sha(&folder_path).await.unwrap_or_default();
    let msg = git.commit_message(&folder_path).await.unwrap_or_default();
    db.set_deploy_commit(deploy_id, &sha, &msg).await?;

    // 4. Compose up (building)
    db.update_deploy_status(deploy_id, "building", None, None)
        .await?;

    let docker = match DockerClient::new().await {
        Ok(d) => d,
        Err(e) => return fail_deploy(db, deploy_id, &mut log_file, &mut ring, e).await,
    };

    match docker
        .compose_up(
            &folder_path,
            &project.compose_path,
            &project.compose_command,
        )
        .await
    {
        Ok(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            write_log(&mut log_file, &combined).await;

            if !output.status.success() {
                let err = anyhow::anyhow!(
                    "docker compose exited with code {}",
                    output.status.code().unwrap_or(-1)
                );
                return fail_deploy(db, deploy_id, &mut log_file, &mut ring, err).await;
            }
        }
        Err(e) => return fail_deploy(db, deploy_id, &mut log_file, &mut ring, e).await,
    }

    // 5. Health check (starting → healthy)
    db.update_deploy_status(deploy_id, "starting", None, None)
        .await?;

    let timeout_secs = project.health_timeout_secs.max(1) as u64;
    let poll_interval = 2u64;
    let mut elapsed = 0u64;
    let mut healthy = false;

    while elapsed < timeout_secs {
        tokio::time::sleep(Duration::from_secs(poll_interval)).await;
        elapsed += poll_interval;

        match docker
            .compose_ps_healthy(&folder_path, &project.compose_path)
            .await
        {
            Ok(true) => {
                healthy = true;
                break;
            }
            Ok(false) => {}
            Err(e) => warn!("compose ps error: {e}"),
        }
    }

    if !healthy {
        let logs = docker
            .compose_logs_tail(&folder_path, &project.compose_path, 200)
            .await
            .unwrap_or_default();
        write_log(&mut log_file, &logs).await;
        ring.push_many(&logs);

        let err = anyhow::anyhow!("health check timed out after {timeout_secs}s");
        return fail_deploy(db, deploy_id, &mut log_file, &mut ring, err).await;
    }

    // Healthy (brief transition state) → succeeded
    db.update_deploy_status(deploy_id, "healthy", None, None)
        .await?;
    write_log(&mut log_file, "\n[deploy succeeded]").await;

    db.finish_deploy(
        deploy_id,
        "succeeded",
        Some(ring.as_string().as_str()),
        None,
    )
    .await?;

    info!(project_id, deploy_id, "deploy succeeded");
    Ok(())
}

async fn fail_deploy(
    db: &Db,
    deploy_id: i64,
    log_file: &mut tokio::fs::File,
    ring: &mut RingBuffer,
    e: anyhow::Error,
) -> Result<()> {
    let err = format!("{e:#}");
    write_log(log_file, &format!("\n[deploy failed] {err}")).await;
    ring.push(&err);
    db.finish_deploy(
        deploy_id,
        "failed",
        Some(ring.as_string().as_str()),
        Some(&err),
    )
    .await?;
    Ok(())
}

async fn write_log(file: &mut tokio::fs::File, text: &str) {
    let _ = file.write_all(text.as_bytes()).await;
    let _ = file.write_all(b"\n").await;
    let _ = file.flush().await;
}

// ============================================================
// Ring buffer for last N lines (→ deploys.log_tail)
// ============================================================

pub struct RingBuffer {
    lines: Vec<String>,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, line: &str) {
        for l in line.lines() {
            self.lines.push(l.to_string());
            if self.lines.len() > self.capacity {
                self.lines.remove(0);
            }
        }
    }

    pub fn push_many(&mut self, text: &str) {
        self.push(text);
    }

    pub fn as_string(&self) -> String {
        self.lines.join("\n")
    }
}
