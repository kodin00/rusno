//! Dashboard HTTP handlers.
//!
//! [`dashboard`] renders the full page; the other three handlers return
//! HTMX partials that the dashboard polls every 5 s. Each partial returns
//! only the fragment it owns, so a refresh is cheap and never re-renders
//! the whole page.
//!
//! Telemetry and Docker state are gathered on demand. `Telemetry::new()`
//! is cheap (a sysinfo refresh) and `DockerClient::new()` pings the daemon
//! once — if Docker is unreachable the container count degrades to `0`
//! rather than erroring the whole page.

use std::path::Path;

use axum::extract::{Request, State};
use axum::response::{Html, IntoResponse, Response};

use crate::auth::session_from_extensions;
use crate::docker::{DockerClient, Telemetry};
use crate::templates::dashboard as tpl;
use crate::AppState;

/// `GET /` — the main dashboard page.
///
/// Gathers project count, running container count, host telemetry, and the
/// five most recent deploys, then renders the full page via the base
/// layout. The CSRF token is pulled from the session in request
/// extensions (set by the auth middleware).
pub async fn dashboard(State(state): State<AppState>, req: Request) -> Response {
    let csrf = session_from_extensions(&req)
        .map(|s| s.csrf_token.as_str())
        .unwrap_or("");

    let project_count = match state.db.list_projects().await {
        Ok(projects) => projects.len(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to list projects for dashboard");
            0
        }
    };

    let container_count = match DockerClient::new().await {
        Ok(client) => match client.count_rusno_containers().await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "failed to count rusno containers");
                0
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "docker unavailable; container count is 0");
            0
        }
    };

    let projects_root = Path::new(&state.config.projects_root);
    let mut telemetry = Telemetry::new();
    let snapshot = telemetry.snapshot(projects_root);

    let deploys = match state.db.list_recent_deploys(5).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list recent deploys for dashboard");
            Vec::new()
        }
    };

    Html(
        tpl::dashboard_page(csrf, project_count, container_count, &snapshot, &deploys)
            .into_string(),
    )
    .into_response()
}

/// `GET /dashboard/stats` — the four stat cards, polled every 5 s.
///
/// Returns the [`tpl::stats_fragment`] markup. The fragment carries its own
/// `hx-get`/`hx-trigger`/`hx-swap="outerHTML"` so polling re-arms after
/// each swap.
pub async fn dashboard_stats(State(state): State<AppState>) -> Response {
    let project_count = match state.db.list_projects().await {
        Ok(projects) => projects.len(),
        Err(e) => {
            tracing::warn!(error = %e, "stats: failed to list projects");
            0
        }
    };

    let container_count = match DockerClient::new().await {
        Ok(client) => match client.count_rusno_containers().await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "stats: failed to count containers");
                0
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "stats: docker unavailable");
            0
        }
    };

    let projects_root = Path::new(&state.config.projects_root);
    let mut telemetry = Telemetry::new();
    let snapshot = telemetry.snapshot(projects_root);

    Html(tpl::stats_fragment(project_count, container_count, &snapshot).into_string())
        .into_response()
}

/// `GET /dashboard/telemetry` — the CPU/memory/storage progress bars,
/// polled every 5 s. The fragment re-arms itself via `hx-swap="outerHTML"`.
pub async fn dashboard_telemetry(State(state): State<AppState>) -> Response {
    let projects_root = Path::new(&state.config.projects_root);
    let mut telemetry = Telemetry::new();
    let snapshot = telemetry.snapshot(projects_root);

    Html(tpl::telemetry_fragment(&snapshot).into_string()).into_response()
}

/// `GET /dashboard/recent-deploys` — the last five deploys as table rows,
/// swapped into `#recent-deploys` (`hx-swap="innerHTML"`).
pub async fn dashboard_recent_deploys(State(state): State<AppState>) -> Response {
    let deploys = match state.db.list_recent_deploys(5).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "recent-deploys: failed to list deploys");
            Vec::new()
        }
    };

    Html(tpl::recent_deploys_fragment(&deploys).into_string()).into_response()
}
