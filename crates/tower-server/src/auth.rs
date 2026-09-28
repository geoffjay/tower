//! Bearer-token auth middleware (D§7, D§13).
//!
//! Unix socket connections are exempt (filesystem permissions are the auth);
//! TCP requires `Authorization: Bearer <token>`. The middleware is told
//! which transport the connection arrived on via an extension.
//!
//! The web UI's browser holds a scoped read-only token instead (D§13,
//! plan S4.C): derived from the bearer token, it opens `/ui` and nothing
//! else, and arrives as the `tower_ui` cookie (or a bearer header).

use axum::extract::Request;
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use hmac::{Hmac, Mac};
use tower_core::TowerError;

/// Extension marking requests that arrived over the unix socket.
#[derive(Clone, Copy)]
pub struct ViaSocket;

/// Cookie carrying the UI token.
pub const UI_COOKIE: &str = "tower_ui";
/// Where `tower ui` links point.
pub const UI_LOGIN_PATH: &str = "/ui/login";
/// Topcoat's page re-render request header (rewritten to a `GET` inside
/// the UI router before any handler runs).
const RUNTIME_HEADER: &str = "x-topcoat-runtime";

/// The scoped read-only UI token: `hex(HMAC-SHA256(token, label))`.
/// Derived, so rotating the bearer token rotates it.
pub fn ui_token(token: &str) -> String {
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key size");
    mac.update(b"tower-ui-read-v1");
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone)]
pub struct Auth {
    token: String,
    ui_token: String,
}

impl Auth {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            ui_token: ui_token(token),
        }
    }
}

/// Requests the UI token may make: anything under `/ui` that reads —
/// `GET`/`HEAD` (the runtime's WebSocket is a `GET` upgrade) and the
/// runtime's page re-render `POST`.
fn in_ui_scope(method: &Method, path: &str, headers: &HeaderMap) -> bool {
    if path != "/ui" && !path.starts_with("/ui/") {
        return false;
    }
    match *method {
        Method::GET | Method::HEAD => true,
        Method::POST => headers
            .get(RUNTIME_HEADER)
            .is_some_and(|v| v.as_bytes() == b"true"),
        _ => false,
    }
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

/// Applied only to the TCP listener's router.
pub async fn tcp_auth(
    axum::extract::State(auth): axum::extract::State<Auth>,
    req: Request,
    next: Next,
) -> Response {
    if req.extensions().get::<ViaSocket>().is_some() {
        return next.run(req).await;
    }
    let path = req.uri().path();
    // the login link carries its own proof; `/` only redirects to `/ui`
    if path == UI_LOGIN_PATH || (path == "/" && req.method() == Method::GET) {
        return next.run(req).await;
    }

    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if bearer.is_some_and(|t| constant_time_eq(t, &auth.token)) {
        return next.run(req).await;
    }
    if in_ui_scope(req.method(), path, req.headers()) {
        let ui = bearer.or_else(|| cookie(req.headers(), UI_COOKIE));
        if ui.is_some_and(|t| constant_time_eq(t, &auth.ui_token)) {
            return next.run(req).await;
        }
    }

    if path == "/ui" || path.starts_with("/ui/") {
        return ui_login_needed();
    }
    let err = TowerError::new(
        tower_core::ErrorCode::Unauthorized,
        "missing or invalid token",
    );
    (
        StatusCode::from_u16(err.code.http_status()).unwrap(),
        axum::Json(serde_json::json!({ "error": err })),
    )
        .into_response()
}

/// A browser without a valid UI cookie gets instructions, not JSON.
pub fn ui_login_needed() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Html(
            "<!DOCTYPE html><html><head><title>tower</title></head>\
             <body style=\"background:#0b0e14;color:#c9d1d9;font:15px system-ui;padding:40px\">\
             <h1>tower</h1><p>This browser has no valid UI login. Run \
             <code>tower ui</code> and open the link it prints.</p></body></html>",
        ),
    )
        .into_response()
}

pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
