//! Messaging module (D§9.3, plan T1.2/T2.2): unified message store + delivery.
//!
//! "Delivery" to a human = the row is queryable by the inbox (`pending` for
//! questions/approvals until responded; `delivered` otherwise) + a
//! `message.created` event — UIs poll/SSE, no push channel in v1.
//! "Delivery" to an agent = driver `prompt`; a driver failure marks the row
//! `failed`.
//!
//! Responding to an agent's question prompts the agent with the answer;
//! responding to an agent's approval answers its on-screen dialog
//! (`dialog`, D§8.2: deny = esc, approve = first plain "Yes"). The pending → answered transition is a CAS, so a double respond
//! can never deliver twice.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::Row;
use tower_core::{
    Message, MessageId, MessageKind, MessageStatus, Part, PartyKind, TaskId, TowerError,
};

use crate::http::error_response;
use crate::state::AppState;
use crate::storage::{enum_str, parse_enum};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/messages", get(list_route).post(send_route))
        .route("/v1/messages/{id}/respond", post(respond_route))
}

/// Default question/approval deadline (D§5.3).
pub const DEFAULT_DEADLINE_S: i64 = 5 * 60;

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
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Required for approval messages: true = approve, false = deny
    /// (`dialog::answer_keys`). Ignored for questions.
    #[serde(default)]
    pub approve: Option<bool>,
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

/// Insert one message row and deliver it (D§9.3).
pub async fn send(state: &AppState, body: SendBody) -> anyhow::Result<Message> {
    let from_kind = body.from_kind.unwrap_or(PartyKind::Human);
    let from_id = body.from.clone().unwrap_or_else(|| "me".into());
    let kind = body.kind.unwrap_or(MessageKind::Prompt);

    // Resolve recipient kind: explicit wins, else agent lookup, else human.
    let to_kind = match body.to_kind {
        Some(k) => k,
        None if crate::inventory::get_agent(state, &body.to)
            .await?
            .is_some() =>
        {
            PartyKind::Agent
        }
        None => PartyKind::Human,
    };
    if to_kind == PartyKind::Agent
        && crate::inventory::get_agent(state, &body.to)
            .await?
            .is_none()
    {
        return Err(TowerError::not_found(format!("agent {} not found", body.to)).into());
    }

    let id = insert(
        state,
        NewMessage {
            task_id: body.task_id.as_ref(),
            from_kind,
            from_id: &from_id,
            to_kind,
            to_id: &body.to,
            kind,
            parts: &body.parts,
            deadline_s: body.deadline_s,
        },
    )
    .await?;

    // To-agent delivery = driver prompt (D§9.3).
    if to_kind == PartyKind::Agent {
        if let Err(e) = deliver_prompt(state, &body.to, &body.parts).await {
            set_status(state, &id, MessageStatus::Failed).await?;
            return Err(e);
        }
    }
    get_message(state, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message {id} vanished after insert"))
}

/// Fields for a new message row.
pub struct NewMessage<'a> {
    pub task_id: Option<&'a TaskId>,
    pub from_kind: PartyKind,
    pub from_id: &'a str,
    pub to_kind: PartyKind,
    pub to_id: &'a str,
    pub kind: MessageKind,
    pub parts: &'a [Part],
    pub deadline_s: Option<i64>,
}

/// Insert a message row + `message.created` event. Questions and approvals
/// start `pending` with a deadline; everything else is `delivered`.
pub async fn insert(state: &AppState, m: NewMessage<'_>) -> anyhow::Result<MessageId> {
    let now = tower_core::now_ms();
    let needs_answer = matches!(m.kind, MessageKind::Question | MessageKind::Approval);
    let deadline_at = if needs_answer {
        Some(now + m.deadline_s.unwrap_or(DEFAULT_DEADLINE_S) * 1000)
    } else {
        m.deadline_s.map(|s| now + s * 1000)
    };
    let status = if needs_answer {
        MessageStatus::Pending
    } else {
        MessageStatus::Delivered
    };
    let id = MessageId::new();

    sqlx::query(
        "INSERT INTO messages (id, task_id, from_kind, from_id, to_kind, to_id, kind, parts, status, deadline_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(id.0.as_str())
    .bind(m.task_id.map(|t| t.0.as_str()))
    .bind(enum_str(m.from_kind))
    .bind(m.from_id)
    .bind(enum_str(m.to_kind))
    .bind(m.to_id)
    .bind(enum_str(m.kind))
    .bind(serde_json::to_string(m.parts)?)
    .bind(enum_str(status))
    .bind(deadline_at)
    .bind(now)
    .execute(&state.pool)
    .await?;

    state
        .events
        .append(
            tower_core::EventKind::MessageCreated,
            Some("message"),
            Some(id.0.as_str()),
            serde_json::json!({
                "kind": m.kind,
                "from": {"kind": m.from_kind, "id": m.from_id},
                "to": {"kind": m.to_kind, "id": m.to_id},
                "status": status,
                "task_id": m.task_id,
            }),
        )
        .await?;
    Ok(id)
}

/// Respond to a pending question/approval (D§7): CAS `pending` → `answered`,
/// then deliver to the original sender when it is an agent (T2.2).
pub async fn respond(
    state: &AppState,
    id: &MessageId,
    parts: &[Part],
    approve: Option<bool>,
) -> anyhow::Result<Message> {
    let existing = get_message(state, id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("message {id} not found")))?;
    if existing.kind == MessageKind::Approval && approve.is_none() {
        return Err(TowerError::invalid("approval responses need `approve: true|false`").into());
    }
    if existing.kind != MessageKind::Approval && !parts.iter().any(|p| p.text.is_some()) {
        return Err(TowerError::invalid("question responses need a text part").into());
    }

    let res = sqlx::query(
        "UPDATE messages SET status='answered', responded_at=?1 WHERE id=?2 AND status='pending'",
    )
    .bind(tower_core::now_ms())
    .bind(id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(TowerError::conflict(format!("message {id} is not pending")).into());
    }

    if existing.from_kind == PartyKind::Agent {
        let delivered = if existing.kind == MessageKind::Approval {
            answer_dialog(state, &existing.from_id, approve == Some(true)).await
        } else {
            deliver_prompt(state, &existing.from_id, parts).await
        };
        if let Err(e) = delivered {
            set_status(state, id, MessageStatus::Failed).await?;
            return Err(e);
        }
    }

    // the response itself is recorded in the event log (audit surface, D§15)
    state
        .events
        .append(
            tower_core::EventKind::MessageStatusChange,
            Some("message"),
            Some(id.0.as_str()),
            serde_json::json!({"status": "answered", "response": parts, "approve": approve}),
        )
        .await?;

    get_message(state, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message {id} vanished after respond"))
}

/// Blocked agent → operator inbox (D§9.3, T2.1). One open item per blocked
/// episode: skipped while the agent already has a pending question/approval
/// to the operator (detection flapping never spams). claude's `blocked` is a
/// permission prompt → `approval` (D§8.2); others → `question`.
/// The recent pane text rides along as a data part.
pub async fn inbox_for_blocked(state: &AppState, agent: &tower_core::Agent) -> anyhow::Result<()> {
    let open: Option<String> = sqlx::query_scalar(
        "SELECT id FROM messages
         WHERE from_kind='agent' AND from_id=?1 AND to_kind='human'
           AND kind IN ('question', 'approval') AND status='pending'
         LIMIT 1",
    )
    .bind(&agent.name)
    .fetch_optional(&state.pool)
    .await?;
    if open.is_some() {
        return Ok(());
    }
    let context = state
        .driver
        .read(&agent.name, tower_driver::ReadSource::Recent, false)
        .await
        .map(|r| r.text)
        .unwrap_or_default();
    let skip = context.chars().count().saturating_sub(2000);
    let context_tail: String = context.chars().skip(skip).collect();
    let kind = if agent.kind == "claude" {
        MessageKind::Approval
    } else {
        MessageKind::Question
    };
    // the job the agent is blocked on, when it owns one
    let task_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM tasks WHERE owner_id = ?1 AND state = 'input-required' LIMIT 1",
    )
    .bind(agent.id.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    let task_id = task_id.map(TaskId::from);
    let parts = [
        Part::text(format!("{} is blocked and waiting for input.", agent.name)),
        Part::data(serde_json::json!({ "context": context_tail })),
    ];
    insert(
        state,
        NewMessage {
            task_id: task_id.as_ref(),
            from_kind: PartyKind::Agent,
            from_id: &agent.name,
            to_kind: PartyKind::Human,
            to_id: "me",
            kind,
            parts: &parts,
            deadline_s: None,
        },
    )
    .await?;
    Ok(())
}

/// Set a message status + `message.status` event.
pub async fn set_status(
    state: &AppState,
    id: &MessageId,
    status: MessageStatus,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE messages SET status=?1 WHERE id=?2")
        .bind(enum_str(status))
        .bind(id.0.as_str())
        .execute(&state.pool)
        .await?;
    state
        .events
        .append(
            tower_core::EventKind::MessageStatusChange,
            Some("message"),
            Some(id.0.as_str()),
            serde_json::json!({ "status": status }),
        )
        .await?;
    Ok(())
}

/// Prompt an agent with the text parts (serialized per agent, D§9.2).
pub async fn deliver_prompt(state: &AppState, to: &str, parts: &[Part]) -> anyhow::Result<()> {
    let text = parts
        .iter()
        .filter_map(|p| p.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let agent = crate::inventory::get_agent(state, to)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {to} not found")))?;
    let guard = crate::sessions::lock_for(&agent.name);
    let _guard = guard.lock().await;
    state.driver.prompt(&agent.name, &text, false).await?;
    Ok(())
}

/// Answer the approval dialog on an agent's screen (D§8.2, `dialog`):
/// deny = `esc`; approve = navigate to the first plain "Yes" + `enter`, or
/// fail if the screen shows no such option — never a guessed key.
pub async fn answer_dialog(state: &AppState, to: &str, approve: bool) -> anyhow::Result<()> {
    let agent = crate::inventory::get_agent(state, to)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {to} not found")))?;
    let guard = crate::sessions::lock_for(&agent.name);
    let _guard = guard.lock().await;
    let screen = if approve {
        state
            .driver
            .read(&agent.name, tower_driver::ReadSource::Visible, false)
            .await?
            .text
    } else {
        String::new()
    };
    let keys = crate::dialog::answer_keys(&screen, approve).ok_or_else(|| {
        TowerError::driver(format!(
            "no plain 'Yes' option on {}'s screen; answer the dialog in herdr",
            agent.name
        ))
    })?;
    state.driver.send_keys(&agent.name, &keys).await?;
    Ok(())
}

/// Inbox query: `?to=&status=&since=` (D§7), newest first.
pub async fn list(
    state: &AppState,
    to: Option<&str>,
    status: Option<MessageStatus>,
    since: Option<i64>,
) -> anyhow::Result<Vec<Message>> {
    let rows = sqlx::query(
        "SELECT * FROM messages
         WHERE (?1 IS NULL OR to_id = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR created_at > ?3)
         ORDER BY created_at DESC",
    )
    .bind(to)
    .bind(status.map(enum_str))
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

/// Row → Message.
pub fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Message {
    let parts_json: String = r.get("parts");
    let task_id: Option<String> = r.get("task_id");
    Message {
        id: MessageId::from(r.get::<String, _>("id")),
        task_id: task_id.map(TaskId::from),
        from_kind: parse_enum(r.get("from_kind")),
        from_id: r.get("from_id"),
        to_kind: parse_enum(r.get("to_kind")),
        to_id: r.get("to_id"),
        kind: parse_enum(r.get("kind")),
        parts: serde_json::from_str(&parts_json).unwrap_or_default(),
        status: parse_enum(r.get("status")),
        deadline_at: r.get("deadline_at"),
        responded_at: r.get("responded_at"),
        created_at: r.get("created_at"),
    }
}

// ---- route handlers ------------------------------------------------------

async fn send_route(State(state): State<AppState>, Json(body): Json<SendBody>) -> Response {
    match send(&state, body).await {
        Ok(m) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "message": m })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

async fn respond_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RespondBody>,
) -> Response {
    match respond(&state, &MessageId::from(id), &body.parts, body.approve).await {
        Ok(m) => Json(serde_json::json!({ "message": m })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn list_route(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    match list(&state, q.to.as_deref(), q.status, q.since).await {
        Ok(msgs) => Json(serde_json::json!({ "messages": msgs })).into_response(),
        Err(e) => error_response(e),
    }
}
