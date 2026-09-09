//! Auth routes: first-run setup wizard, login, and logout.
//!
//! These handlers are wired up in [`crate::routes`]: `/setup` and `/login`
//! live in the public router (no auth middleware), while `/logout` is in the
//! protected router so the session is available for cleanup.
//!
//! Flow:
//! 1. `GET /setup` — shown only when `admin_password_hash` is unset.
//! 2. `POST /setup` — stores the argon2 hash and ensures a `session_secret`.
//! 3. `GET /login` — shown only when `admin_password_hash` is set.
//! 4. `POST /login` — verifies the password, creates a session, sets the
//!    signed cookie, and redirects to `/`.
//! 5. `POST /logout` — deletes the session, clears the cookie, redirects to
//!    `/login`.

use axum::extract::{Extension, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::auth;
use crate::crypto;
use crate::models::session::Session;
use crate::templates::auth::{login_page, setup_page};
use crate::AppState;

/// Form body for `POST /setup`.
#[derive(Debug, Deserialize)]
pub struct SetupForm {
    password: String,
    confirm: String,
}

/// Form body for `POST /login`.
#[derive(Debug, Deserialize)]
pub struct LoginForm {
    password: String,
}

/// Build a redirect response that also sets a `Set-Cookie` header.
///
/// `Redirect::to` produces a `303 See Other`, which is what we want after a
/// form POST; this helper just appends the cookie header to it.
fn redirect_with_cookie(path: &str, cookie: &str) -> Response {
    let mut resp = Redirect::to(path).into_response();
    if let Ok(val) = HeaderValue::from_str(cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, val);
    }
    resp
}

/// `GET /setup` — render the setup wizard, but only on a fresh install.
///
/// If `admin_password_hash` is already set, the wizard is done and we bounce
/// to `/login`. A DB error here is treated as "not set up" so the user can
/// still reach the wizard (the POST handler re-checks before writing).
pub async fn get_setup(State(state): State<AppState>) -> Response {
    if let Ok(Some(_)) = state.db.get_setting("admin_password_hash").await {
        return Redirect::to("/login").into_response();
    }
    setup_page().into_response()
}

/// `POST /setup` — validate, hash, and store the admin password.
///
/// * Redirects to `/login` if setup is already complete.
/// * Redirects back to `/setup` if the passwords don't match.
/// * Hashes with argon2, stores as `admin_password_hash`.
/// * Ensures a `session_secret` exists (32 random bytes, base64url).
pub async fn post_setup(
    State(state): State<AppState>,
    Form(form): Form<SetupForm>,
) -> Response {
    // Never allow re-running setup over an existing password.
    if let Ok(Some(_)) = state.db.get_setting("admin_password_hash").await {
        return Redirect::to("/login").into_response();
    }

    if form.password != form.confirm || form.password.is_empty() {
        return Redirect::to("/setup").into_response();
    }

    let hash = match crypto::hash_password(&form.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("failed to hash admin password: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to set password").into_response();
        }
    };

    if let Err(e) = state.db.set_setting("admin_password_hash", &hash).await {
        tracing::error!("failed to store admin password hash: {e:#}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to set password").into_response();
    }

    // Generate the session secret if it doesn't exist yet (it is normally
    // created by `rusno init`, but setup may run first).
    if let Ok(None) = state.db.get_setting("session_secret").await {
        let secret = crypto::random_base64url(32);
        if let Err(e) = state.db.set_setting("session_secret", &secret).await {
            tracing::error!("failed to store session secret: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to set password").into_response();
        }
    }

    Redirect::to("/login").into_response()
}

/// `GET /login` — render the login form, but only once setup is complete.
///
/// If no admin password is configured yet, send the user to the wizard.
pub async fn get_login(State(state): State<AppState>) -> Response {
    if !matches!(
        state.db.get_setting("admin_password_hash").await,
        Ok(Some(_))
    ) {
        return Redirect::to("/setup").into_response();
    }
    login_page(None).into_response()
}

/// `POST /login` — verify the password and start a session.
///
/// On success, creates a session, sets the signed cookie, and redirects to
/// `/`. On failure, re-renders the login page with an error alert. A missing
/// password hash or session secret is treated as a redirect to `/setup`
/// (the secret is generated on the fly if it was somehow lost).
pub async fn post_login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let hash = match state.db.get_setting("admin_password_hash").await {
        Ok(Some(h)) => h,
        _ => return Redirect::to("/setup").into_response(),
    };

    let valid = crypto::verify_password(&form.password, &hash).unwrap_or(false);
    if !valid {
        return login_page(Some("Invalid password")).into_response();
    }

    let session_secret = match state.db.get_setting("session_secret").await {
        Ok(Some(s)) => s,
        _ => {
            // Session secret missing — regenerate so login can still proceed.
            let s = crypto::random_base64url(32);
            if let Err(e) = state.db.set_setting("session_secret", &s).await {
                tracing::error!("failed to store session secret: {e:#}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "login failed").into_response();
            }
            s
        }
    };

    let (signed_value, _csrf_token) = match auth::create_session(&state.db, &session_secret).await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("failed to create session: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "login failed").into_response();
        }
    };

    redirect_with_cookie("/", &auth::session_cookie(&signed_value))
}

/// `POST /logout` — end the current session.
///
/// Deletes the session row from the DB, clears the cookie, and redirects to
/// `/login`. The session is provided by the auth middleware via request
/// extensions. DB failures are logged but not fatal — the cookie is cleared
/// regardless so the client ends up logged out either way.
pub async fn post_logout(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Response {
    if let Err(e) = state.db.delete_session(&session.id).await {
        tracing::error!("failed to delete session: {e:#}");
    }
    redirect_with_cookie("/login", &auth::clear_cookie())
}
