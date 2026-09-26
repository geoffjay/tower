//! Agent control + query routes (D§7, plan T5.2).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_core::TowerError;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/agents", get(list).post(spawn))
        .route("/v1/agents/adoptable", get(adoptable))
        .route("/v1/agents/{id}", get(show))
        .route("/v1/agents/{id}/read", get(read))
        .route("/v1/agents/{id}/prompt", post(prompt))
        .route("/v1/agents/{id}/interrupt", post(interrupt))
        .route("/v1/agents/{id}/send-keys", post(send_keys))
        .route("/v1/agents/{id}/stop", post(stop))
}

fn tower_err(status: StatusCode, msg: &str) -> axum::response::Response {
    let code = match status {
        StatusCode::NOT_FOUND => tower_core::ErrorCode::NotFound,
        StatusCode::CONFLICT => tower_core::ErrorCode::Conflict,
        _ => tower_core::ErrorCode::Invalid,
    };
    (
        status,
        Json(serde_json::json!({ "error": TowerError::new(code, msg) })),
    )
        .into_response()
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    match crate::inventory::list_agents(&state).await {
        Ok(agents) => Json(serde_json::json!({ "agents": agents })).into_response(),
        Err(e) => tower_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn adoptable(State(state): State<AppState>) -> impl IntoResponse {
    match crate::inventory::adoption_candidates(&state).await {
        Ok(candidates) => Json(serde_json::json!({ "adoptable": candidates })).into_response(),
        Err(e) => tower_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn show(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match crate::inventory::get_agent(&state, &id).await {
        Ok(Some(a)) => Json(serde_json::json!({ "agent": a })).into_response(),
        Ok(None) => tower_err(StatusCode::NOT_FOUND, &format!("agent {id} not found")),
        Err(e) => tower_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn spawn(
    State(state): State<AppState>,
    Json(req): Json<crate::sessions::SpawnRequest>,
) -> impl IntoResponse {
    match crate::sessions::spawn(&state, req).await {
        Ok(a) => (StatusCode::CREATED, Json(serde_json::json!({ "agent": a }))).into_response(),
        Err(e) => tower_err(StatusCode::CONFLICT, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct PromptBody {
    text: String,
    #[serde(default)]
    wait: bool,
}

async fn prompt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PromptBody>,
) -> impl IntoResponse {
    match crate::sessions::prompt(&state, &id, &body.text, body.wait).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => tower_err(StatusCode::NOT_FOUND, &e.to_string()),
    }
}

async fn interrupt(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match crate::sessions::interrupt(&state, &id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => tower_err(StatusCode::NOT_FOUND, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct SendKeysBody {
    keys: Vec<String>,
}

async fn send_keys(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendKeysBody>,
) -> impl IntoResponse {
    // power-user escape hatch: raw keys to the agent's pane
    match crate::inventory::get_agent(&state, &id).await {
        Ok(Some(agent)) => {
            let name = agent.name;
            for key in &body.keys {
                let _ = state
                    .driver
                    .interrupt(&name) // v1: interrupt covers ctrl+c; extend Harness for raw keys in M6
                    .await;
                let _ = key;
            }
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(None) => tower_err(StatusCode::NOT_FOUND, &format!("agent {id} not found")),
        Err(e) => tower_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct ReadQuery {
    #[serde(default)]
    pub format: Option<String>,
}

async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ReadQuery>,
) -> impl IntoResponse {
    let ansi = q.format.as_deref() == Some("ansi");
    match crate::sessions::read(&state, &id, ansi).await {
        Ok(text) => Json(serde_json::json!({ "output": text })).into_response(),
        Err(e) => tower_err(StatusCode::NOT_FOUND, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct StopBody {
    #[serde(default)]
    remove: bool,
}

async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<StopBody>>,
) -> impl IntoResponse {
    let remove = body.map(|b| b.0.remove).unwrap_or_default();
    match crate::sessions::stop(&state, &id, remove).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => tower_err(StatusCode::NOT_FOUND, &e.to_string()),
    }
}
