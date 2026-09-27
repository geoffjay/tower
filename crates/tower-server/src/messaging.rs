//! Messaging module (D§9.3, plan T1.2): unified message store + delivery.
//!
//! "Delivery" to a human = the row is queryable by the inbox (`pending` for
//! questions/approvals until responded; `delivered` otherwise) + a
//! `message.created` event — UIs poll/SSE, no push channel in v1.
//! "Delivery" to an agent = driver `prompt`; the row is marked `delivered`
//! when the driver accepts it.
//!
//! Deadline sweeper (T2.3) expires `pending` questions/approvals at
//! `deadline_at`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::Row;
use tower_core::{Message, MessageId, MessageKind, MessageStatus, Part, PartyKind, TaskId};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/messages", get(list_route).post(send_route))
        .route("/v1/messages/{id}/respond", post(respond_route))
}

fn tower_err(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error": {"code": code_for(status), "message": msg}})),
    )
        .into_response()
}

fn code_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        _ => "invalid",
    }
}

// ---- request shapes ------------------------------------------------------

#[derive(Deserialize)]
pub struct SendBody {
    /// Recipient: agent name/id, human id (`me`), or room name.
    pub to: String,
    #[serde(default)]
    pub to_kind: Option<PartyKind>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub from_kind: Option<PartyKind>,
    #[serde(default)]
    pub kind: Option<MessageKind>,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// Deadline for questions/approvals, in seconds from now.
    #[serde(default)]
    pub deadline_s: Option<i64>,
}

#[derive(Deserialize)]
pub struct RespondBody {
    pub parts: Vec<Part>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub status: Option<MessageStatus>,
    /// `created_at > since` (ms epoch).
    #[serde(default)]
    pub since: Option<i64>,
}

// ---- service -------------------------------------------------------------

const DEFAULT_DEADLINE_MS: i64 = 5 * 60 * 1000;

/// Insert one message row and deliver it (D§9.3).
pub async fn send(state: &AppState, body: SendBody) -> anyhow::Result<Message> {
    let from_kind = body.from_kind.unwrap_or(PartyKind::Human);
    let from_id = body.from.clone().unwrap_or_else(|| "me".into());
    let kind = body.kind.unwrap_or(MessageKind::Prompt);

    // Resolve recipient kind: explicit wins, else agent lookup, else human.
    let to_kind = match body.to_kind {
        Some(k) => k,
        None => {
            if crate::inventory::get_agent(state, &body.to)
                .await?
                .is_some()
            {
                PartyKind::Agent
            } else {
                PartyKind::Human
            }
        }
    };

    let now = tower_core::now_ms();
    // Questions and approvals wait for a response (pending + deadline);
    // everything else is considered delivered at write time.
    let needs_answer = matches!(kind, MessageKind::Question | MessageKind::Approval);
    let deadline_at = if needs_answer {
        Some(now + body.deadline_s.unwrap_or(DEFAULT_DEADLINE_MS / 1000) * 1000)
    } else {
        body.deadline_s.map(|s| now + s * 1000)
    };
    let status = if needs_answer {
        MessageStatus::Pending
    } else {
        MessageStatus::Delivered
    };

    let id = MessageId::from(tower_core::new_id().as_str());
    let parts_json = serde_json::to_string(&body.parts)?;

    sqlx::query(
        "INSERT INTO messages (id, task_id, from_kind, from_id, to_kind, to_id, kind, parts, status, deadline_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(id.0.as_str())
    .bind(body.task_id.as_ref().map(|t| t.0.as_str()))
    .bind(enum_str(from_kind))
    .bind(&from_id)
    .bind(enum_str(to_kind))
    .bind(&body.to)
    .bind(enum_str(kind))
    .bind(&parts_json)
    .bind(enum_str(status))
    .bind(deadline_at)
    .bind(now)
    .execute(&state.pool)
    .await?;

    let msg = get_message(state, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message {id} vanished after insert"))?;

    state
        .events
        .append(
            tower_core::EventKind::MessageCreated,
            Some("message"),
            Some(id.0.as_str()),
            serde_json::json!({
                "kind": kind,
                "from": {"kind": from_kind, "id": from_id},
                "to": {"kind": to_kind, "id": body.to},
                "status": status,
            }),
        )
        .await?;

    // To-agent delivery = driver prompt (D§9.3).
    if to_kind == PartyKind::Agent {
        deliver_to_agent(state, &body.to, &body.parts).await?;
    }
    Ok(msg)
}

/// Answer/respond: sets `responded_at`, flips `pending` → `answered`, and
/// prompts the original sender when it is an agent (the unblock path, T2.2).
pub async fn respond(state: &AppState, id: &MessageId, parts: &[Part]) -> anyhow::Result<Message> {
    let existing = get_message(state, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message {id} not found"))?;

    let now = tower_core::now_ms();
    let res = sqlx::query(
        "UPDATE messages SET status='answered', responded_at=?1 WHERE id=?2 AND status='pending'",
    )
    .bind(now)
    .bind(id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        anyhow::bail!("conflict: message {id} is not pending");
    }

    if existing.from_kind == PartyKind::Agent {
        deliver_to_agent(state, &existing.from_id, parts).await?;
    }

    state
        .events
        .append(
            tower_core::EventKind::MessageStatusChange,
            Some("message"),
            Some(id.0.as_str()),
            serde_json::json!({"status": "answered"}),
        )
        .await?;

    get_message(state, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message {id} vanished after respond"))
}

async fn deliver_to_agent(state: &AppState, to: &str, parts: &[Part]) -> anyhow::Result<()> {
    let agent = crate::inventory::get_agent(state, to)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent {to} not found"))?;
    let text = parts
        .iter()
        .filter_map(|p| p.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let guard = crate::sessions::lock_for(&agent.name);
    let _guard = guard.lock().await;
    state.driver.prompt(&agent.name, &text, false).await?;
    Ok(())
}

/// Inbox query: `?to=&status=&since=` (D§7).
pub async fn list(
    state: &AppState,
    to: Option<&str>,
    status: Option<MessageStatus>,
    since: Option<i64>,
) -> anyhow::Result<Vec<Message>> {
    let status_str = status.map(enum_str);
    let rows = sqlx::query(
        "SELECT * FROM messages
         WHERE (?1 IS NULL OR to_id = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR created_at > ?3)
         ORDER BY created_at DESC",
    )
    .bind(to)
    .bind(status_str)
    .bind(since)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_message).collect())
}

pub async fn get_message(state: &AppState, id: &MessageId) -> anyhow::Result<Option<Message>> {
    let row = sqlx::query("SELECT * FROM messages WHERE id = ?1")
        .bind(id.0.as_str())
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.as_ref().map(row_to_message))
}

/// Row → Message for the messaging API responses.
pub fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Message {
    let parts_json: String = r.get("parts");
    let parts: Vec<Part> = serde_json::from_str(&parts_json).unwrap_or_default();
    let id: String = r.get("id");
    let from_id: String = r.get("from_id");
    let to_id: String = r.get("to_id");
    let from_kind: String = r.get("from_kind");
    let to_kind: String = r.get("to_kind");
    let kind: String = r.get("kind");
    let status: String = r.get("status");
    let task_id: Option<String> = r.get("task_id");
    let deadline_at: Option<i64> = r.get("deadline_at");
    let responded_at: Option<i64> = r.get("responded_at");
    let created_at: i64 = r.get("created_at");
    Message {
        id: MessageId::from(id),
        task_id: task_id.map(TaskId::from),
        from_kind: parse_enum(from_kind),
        from_id,
        to_kind: parse_enum(to_kind),
        to_id,
        kind: parse_enum(kind),
        parts,
        status: parse_enum(status),
        deadline_at,
        responded_at,
        created_at,
    }
}

/// Enum columns are kebab-case strings; serialize via JSON trick.
fn enum_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Row columns are kebab-case enum strings; parse via JSON string trick.
fn parse_enum<T: serde::de::DeserializeOwned>(s: String) -> T {
    serde_json::from_value(serde_json::json!(s)).expect("valid enum column")
}

// ---- route handlers ------------------------------------------------------

async fn send_route(State(state): State<AppState>, Json(body): Json<SendBody>) -> Response {
    match send(&state, body).await {
        Ok(m) => (StatusCode::CREATED, Json(serde_json::json!({"message": m}))).into_response(),
        Err(e) => tower_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn respond_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RespondBody>,
) -> Response {
    let mid = MessageId::from(id);
    match respond(&state, &mid, &body.parts).await {
        Ok(m) => Json(serde_json::json!({"message": m})).into_response(),
        Err(e) => {
            let status = if e.to_string().starts_with("conflict") {
                StatusCode::CONFLICT
            } else if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            tower_err(status, &e.to_string())
        }
    }
}

async fn list_route(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    match list(&state, q.to.as_deref(), q.status, q.since).await {
        Ok(msgs) => Json(serde_json::json!({"messages": msgs})).into_response(),
        Err(e) => tower_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
