//! axum handlers for the Projects tab.
//!
//! Routes (declared in `routes::build_router`):
//! - `GET  /projects`                      — list
//! - `GET  /projects/new`                  — register form
//! - `GET  /projects/new/autodetect?url=`  — repo autodetect (HTMX partial)
//! - `POST /projects`                      — create
//! - `GET  /projects/:slug`               — detail
//! - `POST /projects/:slug/deploy`         — deploy now
//! - `POST /projects/:slug/stop`           — docker compose stop
//! - `POST /projects/:slug/restart`        — docker compose restart
//! - `GET  /projects/:slug/rollback`       — rollback picker (HTMX partial)
//! - `POST /projects/:slug/rollback`       — enqueue rollback
//! - `POST /projects/:slug/remove`         — compose down -v, delete folder + row
//! - `POST /projects/:slug/webhook-toggle` — flip webhook_enabled (HTMX partial)
//! - `GET  /projects/:slug/compose`        — compose editor
//! - `POST /projects/:slug/compose`        — save / reset compose
//! - `GET  /projects/:slug/env`            — .env editor
//! - `POST /projects/:slug/env`            — save .env
//! - `GET  /projects/:slug/env/example`    — .env.example contents

use std::path::PathBuf;

use axum::extract::{Form, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth::session_from_extensions;
use crate::crypto;
use crate::deploy::{git::GitOps, DeployManager, DeployRequest};
use crate::docker::DockerClient;
use crate::models::project::{NewProject, Project};
use crate::templates::projects as tpl;
use crate::AppState;

// ============================================================================
// Form / query types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AutodetectParams {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectForm {
    pub source_url: String,
    pub display_name: String,
    pub folder_name: String,
    pub branch: String,
    pub compose_path: String,
    pub compose_command: String,
    /// HTML checkboxes submit "on" when checked and are absent otherwise.
    pub auto_start: Option<String>,
    pub health_timeout_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RollbackForm {
    pub commit_sha: String,
}

#[derive(Debug, Deserialize)]
pub struct EditorForm {
    pub content: String,
    /// When present, the compose editor's "Reset to repo" button asks the
    /// server to discard local edits and return the repo-tracked version.
    pub reset: Option<String>,
}

// ============================================================================
// Helpers
// ============================================================================

/// Resolve the projects root: prefer the DB setting, fall back to config.
async fn projects_root(state: &AppState) -> String {
    state
        .db
        .get_setting("projects_root")
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.config.projects_root.clone())
}

/// Extract the CSRF token from the request's session.
fn csrf(req: &Request) -> &str {
    session_from_extensions(req)
        .map(|s| s.csrf_token.as_str())
        .unwrap_or("")
}

/// Build the public webhook URL for a project, e.g.
/// `http://host.example/hook/my-slug`. Falls back to a relative path
/// (`/hook/<slug>`) when the Host header is missing or unparsable.
fn webhook_url_for(req: &Request, slug: &str) -> String {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host.is_empty() {
        return format!("/hook/{}", slug);
    }
    let scheme = if host.starts_with("localhost")
        || host.starts_with("127.0.0.1")
        || host.starts_with("0.0.0.0")
    {
        "http"
    } else {
        "https"
    };
    format!("{}://{}/hook/{}", scheme, host, slug)
}

/// Resolve a project by slug or return a 404 response.
async fn require_project(state: &AppState, slug: &str) -> Result<Project, Box<Response>> {
    match state.db.get_project(slug).await {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(Box::new(
            (StatusCode::NOT_FOUND, "project not found").into_response(),
        )),
        Err(e) => {
            tracing::error!(slug, "get_project failed: {e:#}");
            Err(Box::new(
                (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response(),
            ))
        }
    }
}

/// Run a docker compose command, logging the result. The output is surfaced to
/// the tracing layer rather than the UI since these handlers redirect back.
async fn run_compose<F, Fut>(_state: &AppState, project: &Project, op: F)
where
    F: FnOnce(DockerClient, PathBuf, String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<std::process::Output>>,
{
    match DockerClient::new().await {
        Ok(client) => {
            let folder = PathBuf::from(&project.folder_path);
            let compose = project.compose_path.clone();
            match op(client, folder, compose).await {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!(
                            project = %project.slug,
                            "docker compose command exited non-zero: {stderr}"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(project = %project.slug, "docker compose command failed: {e:#}");
                }
            }
        }
        Err(e) => {
            tracing::error!(project = %project.slug, "docker unavailable: {e:#}");
        }
    }
}

/// Read a file relative to the project folder. Returns `None` when the file
/// does not exist (treated as empty in the UI).
async fn read_project_file(project: &Project, rel: &str) -> Option<String> {
    let path = PathBuf::from(&project.folder_path).join(rel);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!(path = %path.display(), "failed to read file: {e}");
            None
        }
    }
}

/// Atomically write `contents` to `target`: write to `<target>.tmp` then
/// rename over the original so a crash or concurrent read never sees a
/// half-written file.
async fn atomic_write(folder: &std::path::Path, rel: &str, contents: &str) -> Response {
    let target = folder.join(rel);
    let tmp = folder.join(format!("{}.tmp", rel));

    // Ensure the parent directory exists (compose_path may be nested).
    if let Some(parent) = target.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("mkdir failed: {e}"),
            )
                .into_response();
        }
    }

    if let Err(e) = tokio::fs::write(&tmp, contents).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write failed: {e}"),
        )
            .into_response();
    }
    if let Err(e) = tokio::fs::rename(&tmp, &target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("rename failed: {e}"),
        )
            .into_response();
    }

    maud::html! { "Saved." }.into_response()
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /projects — project list.
pub async fn list_projects(State(state): State<AppState>, req: Request) -> Response {
    let projects = match state.db.list_projects().await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("list_projects failed: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    tpl::projects_page(csrf(&req), &projects).into_response()
}

/// GET /projects/new — register form.
pub async fn new_project_form(State(_state): State<AppState>, req: Request) -> Response {
    // The form supports an `?error=<code>` query param so a failed POST can
    // redirect back here without losing the error message. Codes are plain
    // kebab-case strings, so no percent-decoding is needed.
    let error = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("error=")))
        .map(str::to_string);
    tpl::new_project_form_page(csrf(&req), error.as_deref()).into_response()
}

/// GET /projects/new/autodetect?url=... — repo autodetect (HTMX partial).
pub async fn autodetect(
    State(state): State<AppState>,
    Query(params): Query<AutodetectParams>,
) -> Response {
    let url = params.url.trim();
    if url.is_empty() {
        return maud::html! {}.into_response();
    }

    let repo_name = Project::repo_name(url);
    let git = GitOps::new(&state.config);
    let default_branch = match git.detect_default_branch(url).await {
        Some(b) => b,
        None => {
            // Could not detect (private repo, network issue). Still surface
            // the repo name so the user can fill the rest manually.
            tracing::debug!(url, "autodetect: could not detect default branch");
            "main".to_string()
        }
    };

    tpl::autodetect_response(&repo_name, &default_branch).into_response()
}

/// POST /projects — create a new project.
pub async fn create_project(
    State(state): State<AppState>,
    Form(form): Form<CreateProjectForm>,
) -> Response {
    // ---- Validation -------------------------------------------------------
    if form.source_url.trim().is_empty() {
        return Redirect::to("/projects/new?error=empty-source-url").into_response();
    }
    if form.folder_name.trim().is_empty() {
        return Redirect::to("/projects/new?error=empty-folder-name").into_response();
    }

    // ---- Resolve the folder path -----------------------------------------
    let root = projects_root(&state).await;
    let folder_name = form.folder_name.trim().to_string();
    let folder_path = PathBuf::from(&root)
        .join(&folder_name)
        .to_string_lossy()
        .to_string();

    // ---- Derive the remaining fields --------------------------------------
    let source_type = Project::detect_source_type(&form.source_url);
    let webhook_secret = crypto::random_base64url(32);
    let auto_start = form.auto_start.as_deref() == Some("on");
    let health_timeout = form.health_timeout_secs.unwrap_or(60).max(1);

    let new = NewProject {
        display_name: form.display_name.trim().to_string(),
        folder_name,
        folder_path: folder_path.clone(),
        source_type,
        source_url: form.source_url.trim().to_string(),
        branch: form.branch.trim().to_string(),
        compose_path: form.compose_path.trim().to_string(),
        compose_command: form.compose_command.trim().to_string(),
        auto_start,
        webhook_secret,
        // New projects start with webhooks enabled and the branch filter on so
        // pushes to the configured branch trigger deploys out of the box.
        webhook_enabled: true,
        branch_filter: true,
        health_timeout_secs: health_timeout,
    };

    // ---- Insert ----------------------------------------------------------
    let project = match state.db.create_project(&new).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("create_project failed: {e:#}");
            return Redirect::to("/projects/new?error=create-failed").into_response();
        }
    };

    // Ensure the project folder exists before the first deploy tries to clone
    // into it.
    let _ = tokio::fs::create_dir_all(&folder_path).await;

    // ---- Enqueue the first deploy ----------------------------------------
    if let Err(e) = enqueue_normal(&state.deploy_manager, project.id, "initial").await {
        tracing::error!(project = %project.slug, "enqueue initial deploy failed: {e:#}");
    }

    Redirect::to(&format!("/projects/{}", project.slug)).into_response()
}

/// GET /projects/:slug — project detail page.
pub async fn project_detail(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    req: Request,
) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    let deploys = match state.db.list_project_deploys(project.id, 50, 0).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("list_project_deploys failed: {e:#}");
            Vec::new()
        }
    };

    let is_deploying = state
        .deploy_manager
        .is_deploying(project.id)
        .await
        .unwrap_or(false);

    let webhook_url = webhook_url_for(&req, &project.slug);

    tpl::project_detail_page(csrf(&req), &project, &webhook_url, &deploys, is_deploying)
        .into_response()
}

/// POST /projects/:slug/deploy — enqueue a manual deploy.
pub async fn deploy_now(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    if let Err(e) = enqueue_normal(&state.deploy_manager, project.id, "manual").await {
        tracing::error!(project = %slug, "enqueue manual deploy failed: {e:#}");
    }
    Redirect::to(&format!("/projects/{}", slug)).into_response()
}

/// POST /projects/:slug/stop — docker compose stop.
pub async fn stop_project(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    run_compose(&state, &project, |c, f, p| async move {
        c.compose_stop(&f, &p).await
    })
    .await;
    Redirect::to(&format!("/projects/{}", slug)).into_response()
}

/// POST /projects/:slug/restart — docker compose restart.
pub async fn restart_project(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    run_compose(&state, &project, |c, f, p| async move {
        c.compose_restart(&f, &p).await
    })
    .await;
    Redirect::to(&format!("/projects/{}", slug)).into_response()
}

/// GET /projects/:slug/rollback — rollback picker (HTMX partial).
pub async fn rollback_picker(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let targets = match state.db.list_rollback_targets(project.id, 25).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("list_rollback_targets failed: {e:#}");
            Vec::new()
        }
    };
    tpl::rollback_picker_fragment(&targets).into_response()
}

/// POST /projects/:slug/rollback — enqueue a rollback deploy.
pub async fn rollback_to(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<RollbackForm>,
) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let sha = form.commit_sha.trim().to_string();
    if sha.is_empty() {
        return (StatusCode::BAD_REQUEST, "no commit selected").into_response();
    }
    if let Err(e) = state
        .deploy_manager
        .enqueue(DeployRequest::Rollback {
            project_id: project.id,
            commit_sha: sha,
        })
        .await
    {
        tracing::error!(project = %slug, "enqueue rollback failed: {e:#}");
    }
    Redirect::to(&format!("/projects/{}", slug)).into_response()
}

/// POST /projects/:slug/remove — compose down -v, delete folder + row.
pub async fn remove_project(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    // Tear down containers + volumes first, then remove the folder, then the
    // row. Order matters: the row is only deleted once the on-disk state is
    // gone, so a crash mid-teardown leaves a recoverable project behind.
    run_compose(&state, &project, |c, f, p| async move {
        c.compose_down(&f, &p).await
    })
    .await;

    let folder = PathBuf::from(&project.folder_path);
    if let Err(e) = tokio::fs::remove_dir_all(&folder).await {
        // NotFound is fine (already gone); anything else is logged.
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %folder.display(), "remove_dir_all failed: {e}");
        }
    }

    if let Err(e) = state.db.delete_project(project.id).await {
        tracing::error!(project = %slug, "delete_project failed: {e:#}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to delete project",
        )
            .into_response();
    }

    Redirect::to("/projects").into_response()
}

/// POST /projects/:slug/webhook-toggle — flip webhook_enabled (HTMX partial).
pub async fn toggle_webhook(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let mut project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    project.webhook_enabled = !project.webhook_enabled;
    let new_enabled = project.webhook_enabled;

    if let Err(e) = state.db.update_project(project.id, &project).await {
        tracing::error!(project = %slug, "update_project (webhook toggle) failed: {e:#}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to update project",
        )
            .into_response();
    }

    tpl::webhook_toggle_fragment(&slug, new_enabled).into_response()
}

// ============================================================================
// Compose & env editors
// ============================================================================

/// GET /projects/:slug/compose — compose file editor.
pub async fn get_compose(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let content = read_project_file(&project, &project.compose_path)
        .await
        .unwrap_or_default();
    tpl::compose_editor(&slug, &content).into_response()
}

/// POST /projects/:slug/compose — save (or reset) the compose file.
pub async fn save_compose(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<EditorForm>,
) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    // "Reset to repo": discard local edits by checking out the repo-tracked
    // version of the compose file, then re-serve the editor.
    if form.reset.is_some() {
        let folder = PathBuf::from(&project.folder_path);
        let git = GitOps::new(&state.config);
        if let Err(e) = git.checkout_path(&folder, &project.compose_path).await {
            tracing::error!(project = %slug, "checkout_path failed: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "git checkout failed").into_response();
        }
        let content = read_project_file(&project, &project.compose_path)
            .await
            .unwrap_or_default();
        return tpl::compose_editor(&slug, &content).into_response();
    }

    let folder = PathBuf::from(&project.folder_path);
    atomic_write(&folder, &project.compose_path, &form.content).await
}

/// GET /projects/:slug/env — .env file editor.
pub async fn get_env(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let content = read_project_file(&project, ".env")
        .await
        .unwrap_or_default();
    // Whether a .env.example exists determines whether the "copy from example"
    // button is shown.
    let has_example = read_project_file(&project, ".env.example").await.is_some();
    tpl::env_editor(&slug, &content, has_example).into_response()
}

/// POST /projects/:slug/env — save the .env file.
pub async fn save_env(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<EditorForm>,
) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let folder = PathBuf::from(&project.folder_path);
    atomic_write(&folder, ".env", &form.content).await
}

/// GET /projects/:slug/env/example — .env.example contents (for the copy button).
pub async fn get_env_example(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let project = match require_project(&state, &slug).await {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    match read_project_file(&project, ".env.example").await {
        Some(contents) => {
            maud::html! {
                div class="card" style="margin-top:0.5rem" {
                    h4 style="margin-top:0" { ".env.example" }
                    pre style="white-space:pre-wrap;font-family:var(--mono);font-size:0.85rem;border:1px solid var(--border);border-radius:var(--radius-sm);padding:0.75rem;overflow:auto;background:#f8fafc;color:#1f2937" { (contents) }
                    button type="button"
                        onclick=(r#"const ed=document.getElementById('env-editor'); ed.value = this.previousElementSibling.textContent; ed.dispatchEvent(new Event('input'))"#)
                    { "Copy into editor" }
                }
            }
            .into_response()
        }
        None => (StatusCode::NOT_FOUND, "no .env.example found").into_response(),
    }
}

// ============================================================================
// Small enqueue wrapper (keeps the call sites tidy + logs failures once)
// ============================================================================

async fn enqueue_normal(
    dm: &DeployManager,
    project_id: i64,
    trigger: &str,
) -> Result<i64, anyhow::Error> {
    dm.enqueue(DeployRequest::Normal {
        project_id,
        trigger: trigger.to_string(),
    })
    .await
}
