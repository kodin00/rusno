//! Deployments routes: paginated deploy list, deploy detail with live log
//! polling, and raw log file serving.
//!
//! All routes here sit behind the auth middleware (see `routes::mod`), so the
//! session is already in the request extensions by the time a handler runs.

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use crate::auth::session_from_extensions;
use crate::templates::deployments::{deploy_detail_page, deployments_page};
use crate::AppState;

/// Page size for the deployments list.
const PER_PAGE: i64 = 25;

/// Query parameters for the deployments list.
#[derive(Deserialize)]
pub struct DeployListParams {
    /// 1-indexed page number; defaults to 1.
    pub page: Option<u32>,
}

/// GET /deployments — paginated deploy list across all projects.
pub async fn deployments(
    State(state): State<AppState>,
    Query(params): Query<DeployListParams>,
    req: Request,
) -> Response {
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page as i64 - 1) * PER_PAGE;

    let deploys = state
        .db
        .list_deploys_paginated(PER_PAGE, offset)
        .await
        .unwrap_or_default();
    let total = state.db.count_deploys().await.unwrap_or(0);

    let csrf = session_from_extensions(&req)
        .map(|s| s.csrf_token.as_str())
        .unwrap_or("");

    Html(deployments_page(csrf, &deploys, page, total).into_string()).into_response()
}

/// GET /deployments/:id — deploy detail page with header and live log view.
pub async fn deploy_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    req: Request,
) -> Response {
    let deploy = match state.db.get_deploy(id).await {
        Ok(Some(d)) => d,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    // The deploy row does not carry project fields; join them back in. A
    // deploy may outlive its project, so fall back to a placeholder.
    let project = state
        .db
        .get_project_by_id(deploy.project_id)
        .await
        .ok()
        .flatten();
    let (project_name, project_slug) = match &project {
        Some(p) => (p.display_name.clone(), p.slug.clone()),
        None => ("(deleted project)".to_string(), String::new()),
    };

    let csrf = session_from_extensions(&req)
        .map(|s| s.csrf_token.as_str())
        .unwrap_or("");

    Html(deploy_detail_page(csrf, &deploy, &project_name, &project_slug).into_string())
        .into_response()
}

/// GET /deployments/:id/log — HTMX partial returning the `log_tail` content,
/// swapped into `#deploy-log` every 5 seconds while a deploy is in flight.
pub async fn deploy_log_fragment(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let deploy = match state.db.get_deploy(id).await {
        Ok(Some(d)) => d,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let log_tail = deploy.log_tail.unwrap_or_default();
    Html(crate::templates::deployments::deploy_log_fragment(&log_tail).into_string())
        .into_response()
}

/// GET /deployments/:id/log/full — raw log file served as text/plain.
/// Auth is enforced by the middleware; this handler just streams the file.
pub async fn deploy_log_full(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let deploy = match state.db.get_deploy(id).await {
        Ok(Some(d)) => d,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let log_path = match deploy.log_path.as_deref() {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    match tokio::fs::read(log_path).await {
        Ok(contents) => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            contents,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
