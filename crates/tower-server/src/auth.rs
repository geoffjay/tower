//! Bearer-token auth middleware (D§7, D§13).
//!
//! Unix socket connections are exempt (filesystem permissions are the auth);
//! TCP requires `Authorization: Bearer <token>`. The middleware is told
//! which transport the connection arrived on via an extension.

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_core::TowerError;

/// Extension marking requests that arrived over the unix socket.
#[derive(Clone, Copy)]
pub struct ViaSocket;

#[derive(Clone)]
pub struct Auth {
    pub token: String,
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

    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| constant_time_eq(t, &auth.token));

    if ok {
        next.run(req).await
    } else {
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
}

fn constant_time_eq(a: &str, b: &str) -> bool {
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
