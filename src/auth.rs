use anyhow::Result;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use cookie::Cookie;
use rand::RngCore;

use crate::db::Db;
use crate::AppState;

pub const SESSION_COOKIE_NAME: &str = "rusno_session";
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60; // 7 days
const CSRF_HEADER: &str = "X-CSRF-Token";

/// Extract a session from the request's Cookie header value.
pub async fn session_from_cookie(
    db: &Db,
    cookie_header: &str,
    session_secret: &str,
) -> Option<crate::models::session::Session> {
    // Find the rusno_session cookie in the Cookie header
    let signed_value = cookie_header
        .split(';')
        .map(|s| s.trim())
        .filter_map(|s| Cookie::parse(s).ok())
        .find(|c| c.name() == SESSION_COOKIE_NAME)?
        .value()
        .to_string();

    let (session_id, valid) = unsign_cookie(&signed_value, session_secret)?;
    if !valid {
        return None;
    }

    let session = db.get_session(&session_id).await.ok()??;

    // Check expiry
    if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&session.expires_at) {
        if chrono::Utc::now() > expires.with_timezone(&chrono::Utc) {
            let _ = db.delete_session(&session_id).await;
            return None;
        }
    }

    Some(session)
}

/// Auth middleware: every non-hook, non-setup, non-login route requires a valid session.
pub fn require_auth(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = Response> + Send>> {
    Box::pin(async move {
        // Allow these paths without auth
        let path = req.uri().path().to_string();
        if path.starts_with("/hook/") || path == "/setup" || path == "/login" || path == "/static/"
        {
            return next.run(req).await;
        }

        let session_secret = match state.db.get_setting("session_secret").await {
            Ok(Some(s)) => s,
            _ => return Redirect::to("/setup").into_response(),
        };

        // Extract cookie header before the async boundary (Request<Body> is not Sync)
        let cookie_header = req
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let session = match session_from_cookie(&state.db, cookie_header, &session_secret).await {
            Some(s) => s,
            None => return Redirect::to("/login").into_response(),
        };

        // CSRF check on state-changing requests (POST, PUT, DELETE)
        let method = req.method().clone();
        if (method == "POST" || method == "PUT" || method == "DELETE")
            && !path.starts_with("/hook/")
        {
            let provided = req
                .headers()
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if provided != session.csrf_token {
                return (StatusCode::FORBIDDEN, "invalid CSRF token").into_response();
            }
        }

        // Touch session expiry (sliding window)
        let _ = state.db.touch_session(&session.id, SESSION_TTL_SECS).await;

        // Insert the session into request extensions for handlers
        let mut req = req;
        req.extensions_mut().insert(session);

        next.run(req).await
    })
}

/// Sign a session cookie: `id.hmac_sha256_base64(session_secret, id)`.
pub fn sign_cookie(session_id: &str, session_secret: &str) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(session_secret.as_bytes()).expect("HMAC key length");
    mac.update(session_id.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    format!("{}.{}", session_id, sig)
}

/// Unsign and verify a session cookie. Returns (session_id, valid).
pub fn unsign_cookie(cookie_value: &str, session_secret: &str) -> Option<(String, bool)> {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let (id, sig) = cookie_value.split_once('.')?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(session_secret.as_bytes()).ok()?;
    mac.update(id.as_bytes());
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let valid: bool = subtle::ConstantTimeEq::ct_eq(sig.as_bytes(), expected.as_bytes()).into();
    Some((id.to_string(), valid))
}

/// Create a new session and return a signed cookie string + CSRF token.
pub async fn create_session(db: &Db, session_secret: &str) -> Result<(String, String)> {
    use base64::Engine;
    let mut id_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut id_bytes);
    let session_id = base64::engine::general_purpose::STANDARD.encode(id_bytes);

    let mut csrf_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut csrf_bytes);
    let csrf_token = base64::engine::general_purpose::STANDARD.encode(csrf_bytes);

    db.create_session(&session_id, &csrf_token, SESSION_TTL_SECS)
        .await?;

    let signed = sign_cookie(&session_id, session_secret);
    Ok((signed, csrf_token))
}

/// Build the Set-Cookie header value for a session.
pub fn session_cookie(signed_value: &str) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_COOKIE_NAME, signed_value, SESSION_TTL_SECS
    )
}

/// Build a cookie that clears the session.
pub fn clear_cookie() -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE_NAME
    )
}

/// Get the session from request extensions (set by the auth middleware).
pub fn session_from_extensions(req: &Request) -> Option<&crate::models::session::Session> {
    req.extensions().get::<crate::models::session::Session>()
}
