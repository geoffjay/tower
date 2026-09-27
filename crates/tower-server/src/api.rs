//! Routes: /healthz, /v1/schema (D§7). Introspectable contract registry.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/schema", get(schema))
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .is_ok();
    let body = serde_json::json!({
        "ok": db_ok,
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": state.started_at,
        "db": if db_ok { "ok" } else { "error" },
    });
    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body))
}

/// Route + event-type registry (herdr's `api schema` idea, D§7).
async fn schema(State(_state): State<AppState>) -> impl IntoResponse {
    let routes = [
        ("GET", "/healthz"),
        ("GET", "/v1/agents"),
        ("GET", "/v1/agents/{id}"),
        ("GET", "/v1/agents/adoptable"),
        ("POST", "/v1/agents"),
        ("POST", "/v1/agents/{id}/prompt"),
        ("POST", "/v1/agents/{id}/interrupt"),
        ("POST", "/v1/agents/{id}/send-keys"),
        ("POST", "/v1/agents/{id}/stop"),
        ("GET", "/v1/agents/{id}/read"),
        ("GET", "/v1/agents/{id}/stream"),
        ("GET", "/v1/messages"),
        ("POST", "/v1/messages"),
        ("POST", "/v1/messages/{id}/respond"),
        ("GET", "/v1/events"),
        ("GET", "/v1/events/heads"),
    ];
    let event_types = [
        "server.started",
        "agent.created",
        "agent.removed",
        "agent.state",
        "agent.output",
        "task.created",
        "task.status",
        "task.assigned",
        "task.leased_out",
        "task.completed",
        "task.failed",
        "message.created",
        "message.status",
        "approval.expired",
        "machine.state",
        "node.registered",
        "node.disconnected",
    ];
    Json(serde_json::json!({
        "name": "tower",
        "version": env!("CARGO_PKG_VERSION"),
        "routes": routes
            .iter()
            .map(|(m, p)| serde_json::json!({"method": m, "path": p}))
            .collect::<Vec<_>>(),
        "event_types": event_types,
        "sse": {
            "cursor_param": "cursor",
            "last_event_id": "Last-Event-ID",
            "heartbeat_comment": ":",
            "heartbeat_interval_secs": 15,
        },
        "error_codes": ["not_found", "conflict", "timeout", "driver", "invalid",
                        "unauthorized", "machine_offline"],
        "note": "phase 1 routes only; more arrive with milestones 5-6",
    }))
}
