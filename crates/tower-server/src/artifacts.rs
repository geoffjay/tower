//! Artifacts (D§5.6, D§6; decision: workflows §4): text a job's owner
//! attaches to its open job — the large output a workflow step hands on by
//! reference instead of in its summary. Stored in SQLite with the job.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::Row;
use tower_core::{AgentId, Artifact, ArtifactId, EventKind, TaskId, TaskState, TowerError};

use crate::http::error_response;
use crate::state::AppState;

/// Largest artifact accepted, in bytes (decision §11.4).
pub const ARTIFACT_MAX_BYTES: usize = 1024 * 1024;
const NAME_MAX_CHARS: usize = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/tasks/{id}/artifacts", post(put_route))
        .route("/v1/artifacts/{id}", get(get_route))
}

#[derive(Deserialize)]
pub struct PutArtifact {
    /// The job's owner (name or id).
    #[serde(rename = "as")]
    pub as_agent: String,
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

/// Attach `content` to `task`: only its owner, only while it is open.
pub async fn put(
    state: &AppState,
    task: &TaskId,
    req: PutArtifact,
    now: i64,
) -> anyhow::Result<Artifact> {
    let agent = crate::inventory::get_agent(state, &req.as_agent)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {} not found", req.as_agent)))?;
    let job = crate::tasks::get_task(state, task)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("task {task} not found")))?;
    let open = matches!(
        job.state,
        TaskState::Assigned | TaskState::Working | TaskState::InputRequired
    );
    if job.owner_id.as_ref() != Some(&agent.id) || !open {
        let mut e = TowerError::conflict(format!(
            "artifacts attach to your own open job: task {task} is {} and not owned by {}",
            crate::storage::enum_str(job.state),
            agent.name
        ));
        e.detail = Some(serde_json::json!({ "state": job.state, "owner_id": job.owner_id }));
        return Err(e.into());
    }
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX_CHARS {
        return Err(TowerError::invalid(format!(
            "artifact name must be 1-{NAME_MAX_CHARS} characters"
        ))
        .into());
    }
    if req.content.len() > ARTIFACT_MAX_BYTES {
        let mut e = TowerError::invalid(format!(
            "artifact is {} bytes; the limit is {ARTIFACT_MAX_BYTES}",
            req.content.len()
        ));
        e.detail = Some(serde_json::json!({ "reason": "artifact_too_large" }));
        return Err(e.into());
    }
    let media_type = req
        .media_type
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let id = ArtifactId::new();
    let size = req.content.len() as i64;
    sqlx::query(
        "INSERT INTO artifacts (id, task_id, name, media_type, content, size, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id.0.as_str())
    .bind(task.0.as_str())
    .bind(name)
    .bind(media_type)
    .bind(&req.content)
    .bind(size)
    .bind(agent.id.0.as_str())
    .bind(now)
    .execute(&state.pool)
    .await?;
    state
        .events
        .append(
            EventKind::ArtifactCreated,
            Some("task"),
            Some(task.0.as_str()),
            serde_json::json!({
                "task_id": task, "artifact_id": id, "name": name,
                "media_type": media_type, "size": size, "by": agent.id,
            }),
        )
        .await?;
    Ok(Artifact {
        id,
        task_id: task.clone(),
        name: name.to_string(),
        media_type: media_type.map(str::to_string),
        size,
        created_by: Some(agent.id),
        created_at: now,
    })
}

/// One artifact with its content.
pub async fn fetch(state: &AppState, id: &ArtifactId) -> anyhow::Result<(Artifact, String)> {
    let row = sqlx::query("SELECT * FROM artifacts WHERE id = ?1")
        .bind(id.0.as_str())
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("artifact {id} not found")))?;
    Ok((row_to_artifact(&row), row.get("content")))
}

/// A job's artifacts, oldest first (rows only).
pub async fn list_for_task(state: &AppState, task: &TaskId) -> anyhow::Result<Vec<Artifact>> {
    let rows = sqlx::query(
        "SELECT id, task_id, name, media_type, size, created_by, created_at
         FROM artifacts WHERE task_id = ?1 ORDER BY created_at, id",
    )
    .bind(task.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_artifact).collect())
}

fn row_to_artifact(r: &sqlx::sqlite::SqliteRow) -> Artifact {
    Artifact {
        id: ArtifactId::from(r.get::<String, _>("id")),
        task_id: TaskId::from(r.get::<String, _>("task_id")),
        name: r.get("name"),
        media_type: r.get("media_type"),
        size: r.get("size"),
        created_by: r.get::<Option<String>, _>("created_by").map(AgentId::from),
        created_at: r.get("created_at"),
    }
}

async fn put_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<PutArtifact>,
) -> Response {
    match put(&s, &TaskId::from(id), b, tower_core::now_ms()).await {
        Ok(a) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "artifact": a })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

async fn get_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match fetch(&s, &ArtifactId::from(id)).await {
        Ok((a, content)) => {
            Json(serde_json::json!({ "artifact": a, "content": content })).into_response()
        }
        Err(e) => error_response(e),
    }
}
