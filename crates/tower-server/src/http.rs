//! Error → HTTP response mapping (D§7 conventions).
//!
//! Service functions return `anyhow::Error`; a wrapped `TowerError` carries
//! its code (conflict, not_found, ...), a `DriverError` maps to `driver`,
//! anything else is an internal `driver`-class failure.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tower_core::{ErrorCode, TowerError};

/// Classify a service error into the D§7 envelope.
pub fn to_tower_error(e: anyhow::Error) -> TowerError {
    match e.downcast::<TowerError>() {
        Ok(t) => t,
        Err(e) => match e.downcast::<tower_driver::DriverError>() {
            Ok(d) => TowerError::driver(d.to_string()),
            Err(e) => TowerError::new(ErrorCode::Driver, e.to_string()),
        },
    }
}

pub fn error_response(e: anyhow::Error) -> Response {
    let err = to_tower_error(e);
    let status =
        StatusCode::from_u16(err.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(serde_json::json!({ "error": err }))).into_response()
}
