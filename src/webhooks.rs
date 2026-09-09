//! Webhook endpoint: `POST /hook/<slug>`.
//!
//! Auto-detects GitHub HMAC vs plain shared-secret, verifies in constant
//! time, applies the branch filter (GitHub mode), and enqueues a deploy.
//! Always returns fast — GitHub retries webhooks that don't 2xx within 10s.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;

use crate::AppState;

/// Max request body size for webhooks (GitHub payloads are small JSON).
const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB

/// Rate limit: max triggers per minute per slug.
const RATE_LIMIT: u32 = 10;

/// Per-slug rate limiter state: (window_start, count).
static RATE_LIMITS: Mutex<Option<HashMap<String, (Instant, u32)>>> = Mutex::new(None);

fn rate_limited(slug: &str) -> bool {
    let mut guards = RATE_LIMITS.lock().unwrap();
    let map = guards.get_or_insert_with(HashMap::new);
    let now = Instant::now();

    let entry = map
        .entry(slug.to_string())
        .or_insert((now, 0));

    if now.duration_since(entry.0).as_secs() >= 60 {
        // New window
        *entry = (now, 1);
        false
    } else {
        entry.1 += 1;
        entry.1 > RATE_LIMIT
    }
}

#[derive(Debug)]
enum VerifyMode {
    /// X-Hub-Signature-256 present — GitHub HMAC mode.
    GitHub { signature: String },
    /// X-Rusno-Secret or X-Webhook-Secret present — plain mode.
    Plain { secret: String },
    /// Neither header — reject.
    None,
}

fn detect_mode(headers: &HeaderMap) -> VerifyMode {
    if let Some(sig) = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        return VerifyMode::GitHub {
            signature: sig.to_string(),
        };
    }
    if let Some(secret) = headers
        .get("x-rusno-secret")
        .or_else(|| headers.get("x-webhook-secret"))
        .and_then(|v| v.to_str().ok())
    {
        return VerifyMode::Plain {
            secret: secret.to_string(),
        };
    }
    VerifyMode::None
}

pub async fn handle_webhook(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Body size cap → 413
    if body.len() > MAX_BODY {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"status": "error", "reason": "body too large"})),
        )
            .into_response();
    }

    // Look up the project. Unknown slug AND disabled webhooks both return 404
    // so rusno doesn't reveal which slugs are registered.
    let project = match state.db.get_project(&slug).await {
        Ok(Some(p)) if p.webhook_enabled => p,
        _ => {
            return (StatusCode::NOT_FOUND, Json(json!({"status": "error", "reason": "not found"})))
                .into_response()
        }
    };

    // Rate limit per slug → 429
    if rate_limited(&slug) {
        tracing::warn!(slug, "webhook rate limited");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"status": "error", "reason": "rate limited"})),
        )
            .into_response();
    }

    let mode = detect_mode(&headers);

    match mode {
        VerifyMode::GitHub { signature } => {
            // GitHub mode: HMAC-SHA256 of body with webhook_secret
            if !crate::crypto::verify_hmac_sha256(
                project.webhook_secret.as_bytes(),
                &body,
                &signature,
            ) {
                tracing::warn!(slug, "webhook bad signature");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"status": "error", "reason": "bad signature"})),
                )
                    .into_response();
            }

            // Non-push events → 200 ignored (prevents GitHub retries)
            let event = headers
                .get("x-github-event")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if event != "push" {
                return (
                    StatusCode::OK,
                    Json(json!({"status": "ignored", "reason": "event not push"})),
                )
                    .into_response();
            }

            // Branch filter: only deploy when ref matches the configured branch
            if project.branch_filter {
                let pushed_branch = parse_push_branch(&body);
                match pushed_branch {
                    Some(branch) if branch == project.branch => {
                        // proceed to deploy
                    }
                    Some(branch) => {
                        tracing::info!(slug, branch, "webhook branch mismatch, ignoring");
                        return (
                            StatusCode::OK,
                            Json(json!({"status": "ignored", "reason": "branch mismatch"})),
                        )
                            .into_response();
                    }
                    None => {
                        // Couldn't parse ref — be lenient and deploy
                        tracing::warn!(slug, "webhook push payload missing ref, deploying anyway");
                    }
                }
            }

            enqueue_and_respond(&state, project.id, "webhook_github", &slug).await
        }
        VerifyMode::Plain { secret } => {
            // Plain mode: constant-time compare of the header secret.
            // No branch filter — plain mode always deploys.
            if !crate::crypto::constant_time_eq(&secret, &project.webhook_secret) {
                tracing::warn!(slug, "webhook bad secret");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"status": "error", "reason": "bad secret"})),
                )
                    .into_response();
            }

            enqueue_and_respond(&state, project.id, "webhook_plain", &slug).await
        }
        VerifyMode::None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"status": "error", "reason": "no verification header"})),
        )
            .into_response(),
    }
}

async fn enqueue_and_respond(
    state: &AppState,
    project_id: i64,
    trigger: &str,
    slug: &str,
) -> Response {
    match state
        .deploy_manager
        .enqueue(crate::deploy::DeployRequest::Normal {
            project_id,
            trigger: trigger.to_string(),
        })
        .await
    {
        Ok(deploy_id) => {
            tracing::info!(slug, trigger, deploy_id, "webhook deploy queued");
            (
                StatusCode::OK,
                Json(json!({"status": "queued", "deploy_id": deploy_id})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(slug, "webhook enqueue failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"status": "error", "reason": "internal error"})),
            )
                .into_response()
        }
    }
}

/// Parse `ref` from a GitHub push payload: `refs/heads/<branch>` → `<branch>`.
fn parse_push_branch(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let r = v.get("ref")?.as_str()?;
    r.strip_prefix("refs/heads/").map(|s| s.to_string())
}
