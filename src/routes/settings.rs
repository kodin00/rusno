//! Settings routes: service config, SSH keys, GitHub token, Docker
//! maintenance, and admin account.
//!
//! All handlers sit behind the auth middleware (see `routes::build_router`),
//! so a valid session is guaranteed and the CSRF token is checked on POSTs.

use axum::extract::{Form, Request, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};
use serde::Deserialize;
use tracing::warn;

use crate::auth::session_from_extensions;
use crate::config::AppConfig;
use crate::crypto;
use crate::docker::DockerClient;
use crate::templates::settings::{
    docker_prune_result, docker_summary_fragment, settings_page as settings_page_tpl,
    ssh_key_fragment, SettingsValues,
};
use crate::AppState;

/// SSH key path used in rusno-managed mode: `<data_dir>/ssh/rusno_ed25519`.
fn rusno_ssh_key_path(config: &AppConfig) -> std::path::PathBuf {
    std::path::PathBuf::from(&config.data_dir)
        .join("ssh")
        .join("rusno_ed25519")
}

/// Read the rusno-managed public key, if present.
fn read_rusno_pubkey(config: &AppConfig) -> Option<String> {
    let pub_path = rusno_ssh_key_path(config).with_file_name("rusno_ed25519.pub");
    std::fs::read_to_string(&pub_path).ok()
}

// ----------------------------------------------------------------------------
// Form payloads
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SaveSettingsForm {
    pub rusno_url: String,
    pub projects_root: String,
    pub deploy_concurrency: i64,
    pub default_health_timeout_secs: i64,
    pub ssh_mode: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveTokenForm {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveSshModeForm {
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct NuclearConfirmForm {
    pub confirm: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordForm {
    pub new_password: String,
    pub confirm_password: String,
}

// ----------------------------------------------------------------------------
// Handlers
// ----------------------------------------------------------------------------

/// GET /settings — render the full settings page.
pub async fn settings_page(State(state): State<AppState>, req: Request) -> Response {
    let csrf = session_from_extensions(&req)
        .map(|s| s.csrf_token.clone())
        .unwrap_or_default();

    // Load current settings values from the DB, falling back to config/defaults.
    let rusno_url = state
        .db
        .get_setting("rusno_url")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("http://localhost:{}", state.config.port));
    let projects_root = state
        .db
        .get_setting("projects_root")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| state.config.projects_root.clone());
    let deploy_concurrency = state
        .db
        .get_setting("deploy_concurrency")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "2".to_string());
    let default_health_timeout_secs = state
        .db
        .get_setting("default_health_timeout_secs")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "60".to_string());
    let ssh_mode = state
        .db
        .get_setting("ssh_mode")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| state.config.ssh_mode.clone());
    let has_github_token = state
        .db
        .get_setting("github_token_encrypted")
        .await
        .map(|v| v.is_some())
        .unwrap_or(false);

    let values = SettingsValues {
        rusno_url,
        projects_root,
        deploy_concurrency,
        default_health_timeout_secs,
        ssh_mode,
        has_github_token,
    };

    // In rusno-managed mode, surface the existing pubkey (if any) so the
    // page can show it without a round-trip. host-existing mode discovers
    // keys from ~/.ssh/ inside the template itself.
    let ssh_pubkey = if values.ssh_mode == "rusno-managed" {
        read_rusno_pubkey(&state.config)
    } else {
        None
    };

    // Docker disk usage is loaded lazily via the /settings/docker HTMX
    // endpoint (hx-trigger="load"), so it's None on the initial render.
    settings_page_tpl(&csrf, &state.config, &values, ssh_pubkey.as_deref(), None).into_response()
}

/// POST /settings — save service configuration values.
pub async fn save_settings(
    State(state): State<AppState>,
    Form(form): Form<SaveSettingsForm>,
) -> Response {
    let _ = state.db.set_setting("rusno_url", &form.rusno_url).await;
    let _ = state
        .db
        .set_setting("projects_root", &form.projects_root)
        .await;
    let _ = state
        .db
        .set_setting("deploy_concurrency", &form.deploy_concurrency.to_string())
        .await;
    let _ = state
        .db
        .set_setting(
            "default_health_timeout_secs",
            &form.default_health_timeout_secs.to_string(),
        )
        .await;
    // ssh_mode is validated against the known set; the toggle is also exposed
    // via a dedicated endpoint, but the settings form carries it too.
    if form.ssh_mode == "rusno-managed" || form.ssh_mode == "host-existing" {
        let _ = state.db.set_setting("ssh_mode", &form.ssh_mode).await;
    }
    Redirect::to("/settings").into_response()
}

/// POST /settings/ssh-mode — toggle SSH handling mode.
pub async fn save_ssh_mode(
    State(state): State<AppState>,
    Form(form): Form<SaveSshModeForm>,
) -> Response {
    if form.mode == "rusno-managed" || form.mode == "host-existing" {
        let _ = state.db.set_setting("ssh_mode", &form.mode).await;
    }
    Redirect::to("/settings").into_response()
}

/// POST /settings/ssh/generate — generate a new rusno-managed ed25519 key
/// and return the pubkey as an HTMX partial.
pub async fn generate_ssh_key(State(state): State<AppState>) -> Response {
    let key_path = rusno_ssh_key_path(&state.config);
    // Ensure the parent directory exists.
    if let Some(parent) = key_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Remove any existing key so ssh-keygen doesn't refuse to overwrite.
    let _ = std::fs::remove_file(&key_path);
    let _ = std::fs::remove_file(key_path.with_file_name("rusno_ed25519.pub"));

    let output = match tokio::process::Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            &key_path.to_string_lossy(),
            "-N",
            "",
            "-C",
            "rusno",
        ])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            warn!(error = %e, "failed to run ssh-keygen");
            return error_fragment(&format!("Failed to generate SSH key: {e}")).into_response();
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return error_fragment(&format!("ssh-keygen failed: {stderr}")).into_response();
    }

    let pub_path = key_path.with_file_name("rusno_ed25519.pub");
    let pubkey = match std::fs::read_to_string(&pub_path) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "generated key but pubkey unreadable");
            return error_fragment(&format!("Key generated but unreadable: {e}")).into_response();
        }
    };

    ssh_key_fragment(&pubkey).into_response()
}

/// POST /settings/github-token — encrypt and store the GitHub token.
pub async fn save_github_token(
    State(state): State<AppState>,
    Form(form): Form<SaveTokenForm>,
) -> Response {
    if form.token.trim().is_empty() {
        // An empty submission is treated as a removal.
        let _ = state.db.set_setting("github_token_encrypted", "").await;
        return Redirect::to("/settings").into_response();
    }
    match state.master_key.encrypt(form.token.as_bytes()) {
        Ok(enc) => {
            let _ = state.db.set_setting("github_token_encrypted", &enc).await;
        }
        Err(e) => {
            warn!(error = %e, "failed to encrypt github token");
            return error_fragment(&format!("Failed to store token: {e}")).into_response();
        }
    }
    Redirect::to("/settings").into_response()
}

/// POST /settings/github-token/remove — delete the stored GitHub token.
pub async fn remove_github_token(State(state): State<AppState>) -> Response {
    let _ = state.db.set_setting("github_token_encrypted", "").await;
    Redirect::to("/settings").into_response()
}

/// GET /settings/docker — return an HTMX partial with docker system df.
pub async fn docker_maintain(State(_state): State<AppState>) -> Response {
    let docker = match DockerClient::new().await {
        Ok(d) => d,
        Err(e) => {
            return error_fragment(&format!("Docker unavailable: {e}")).into_response();
        }
    };
    match docker.system_df().await {
        Ok(df) => docker_summary_fragment(&df).into_response(),
        Err(e) => error_fragment(&format!("docker system df failed: {e}")).into_response(),
    }
}

/// POST /settings/docker/prune-safe — run the safe prune and return output.
pub async fn docker_prune_safe(State(_state): State<AppState>) -> Response {
    let docker = match DockerClient::new().await {
        Ok(d) => d,
        Err(e) => {
            return error_fragment(&format!("Docker unavailable: {e}")).into_response();
        }
    };
    match docker.prune_safe().await {
        Ok(out) => docker_prune_result(&out).into_response(),
        Err(e) => error_fragment(&format!("safe prune failed: {e}")).into_response(),
    }
}

/// POST /settings/docker/prune-nuclear — require typed confirmation, then
/// run `docker system prune -a --volumes`.
pub async fn docker_prune_nuclear(
    State(_state): State<AppState>,
    Form(form): Form<NuclearConfirmForm>,
) -> Response {
    if form.confirm.trim() != "prune" {
        return error_fragment("Confirmation failed: type 'prune' to confirm.").into_response();
    }
    let docker = match DockerClient::new().await {
        Ok(d) => d,
        Err(e) => {
            return error_fragment(&format!("Docker unavailable: {e}")).into_response();
        }
    };
    match docker.prune_nuclear().await {
        Ok(out) => docker_prune_result(&out).into_response(),
        Err(e) => error_fragment(&format!("nuclear prune failed: {e}")).into_response(),
    }
}

/// POST /settings/change-password — validate, hash, and store the new admin
/// password.
pub async fn change_password(
    State(state): State<AppState>,
    Form(form): Form<ChangePasswordForm>,
) -> Response {
    if form.new_password.is_empty() {
        return error_fragment("New password must not be empty.").into_response();
    }
    if form.new_password != form.confirm_password {
        return error_fragment("Passwords do not match.").into_response();
    }
    match crypto::hash_password(&form.new_password) {
        Ok(hash) => {
            let _ = state.db.set_setting("admin_password_hash", &hash).await;
        }
        Err(e) => {
            warn!(error = %e, "failed to hash new password");
            return error_fragment(&format!("Failed to set password: {e}")).into_response();
        }
    }
    Redirect::to("/settings").into_response()
}

/// POST /settings/sign-out-everywhere — invalidate every session, clear the
/// caller's cookie, and bounce to /login.
pub async fn sign_out_everywhere(State(state): State<AppState>) -> Response {
    let _ = state.db.delete_all_sessions().await;
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("rusno_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    resp
}

/// A tiny HTMX partial used to surface an error from an inline action
/// (key generation, prune, token save). Renders into the same swap target
/// as the success fragments.
fn error_fragment(msg: &str) -> Markup {
    html! {
        div class="card" style="border:1px solid rgba(220,38,38,0.3);background:var(--danger-soft);" {
            p style="color:var(--danger-text); font-weight:600;" { "Error" }
            pre style="white-space:pre-wrap; word-break:break-word;" { (msg) }
        }
    }
}
