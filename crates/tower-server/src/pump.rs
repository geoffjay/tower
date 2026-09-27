//! Driver event pump (D§9.2/9.3, plan T2.1): consumes the harness event
//! stream, mirrors state changes into the event log + agents table, and
//! turns `blocked` detections into inbox questions for the operator.
//!
//! Dedup: one open question per blocked episode — a blocked agent that
//! stays blocked across polls emits exactly one message. A new blocked
//! episode (agent unblocked, then blocked again) opens a new question.

use tower_core::{MessageKind, Part, PartyKind};
use tower_driver::HarnessEvent;

use crate::state::AppState;

/// Spawn the pump task. Runs until the server exits; transient driver
/// errors inside the stream are swallowed (the driver retries polling).
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    let mut events = state.driver.events();
    let st = state.clone();
    tokio::spawn(async move {
        while let Some(ev) = futures::StreamExt::next(&mut events).await {
            if let Err(e) = handle(&st, ev).await {
                tracing::warn!(error = %e, "pump: event handling failed");
            }
        }
    })
}

async fn handle(state: &AppState, ev: HarnessEvent) -> anyhow::Result<()> {
    match ev {
        HarnessEvent::AgentUp(a) => {
            upsert_state(state, &a.name, a.state).await?;
        }
        HarnessEvent::StateChange { name, to, .. } => {
            upsert_state(state, &name, to).await?;
            if to == tower_driver::HarnessState::Blocked {
                if let Err(e) = question_for_blocked(state, &name).await {
                    tracing::warn!(error = %e, "pump: blocked-question failed");
                }
            }
        }
        HarnessEvent::Output { name, text } => {
            state
                .events
                .append(
                    tower_core::EventKind::AgentOutput,
                    Some("agent"),
                    Some(&name),
                    serde_json::json!({"chunk": text}),
                )
                .await?;
        }
        HarnessEvent::AgentDown { name, .. } => {
            sqlx::query(
                "UPDATE agents SET state='dead', pane_id=NULL, updated_at=?1 WHERE name=?2",
            )
            .bind(tower_core::now_ms())
            .bind(&name)
            .execute(&state.pool)
            .await?;
            state
                .events
                .append(
                    tower_core::EventKind::AgentStateChange,
                    Some("agent"),
                    Some(&name),
                    serde_json::json!({"to": "dead"}),
                )
                .await?;
        }
    }
    Ok(())
}

async fn upsert_state(
    state: &AppState,
    name: &str,
    hs: tower_driver::HarnessState,
) -> anyhow::Result<()> {
    let agent = match crate::inventory::get_agent(state, name).await? {
        Some(a) => a,
        None => return Ok(()), // harness agent we don't own (adoptable); ignore
    };
    let new = crate::inventory::map_harness_state(hs);
    if agent.state == new {
        return Ok(());
    }
    sqlx::query("UPDATE agents SET state=?1, updated_at=?2 WHERE id=?3")
        .bind(new.as_str())
        .bind(tower_core::now_ms())
        .bind(agent.id.0.as_str())
        .execute(&state.pool)
        .await?;
    state
        .events
        .append(
            tower_core::EventKind::AgentStateChange,
            Some("agent"),
            Some(&agent.id.0),
            serde_json::json!({"from": agent.state.as_str(), "to": new.as_str()}),
        )
        .await?;
    Ok(())
}

/// One open inbox item per blocked episode (T2.1): if the agent already has
/// a `pending` question/approval addressed to the operator, don't create
/// another. claude's `blocked` is a permission prompt → `approval`
/// (answered with keys `1`/`2`, D§8.2); other harnesses → `question`.
async fn question_for_blocked(state: &AppState, name: &str) -> anyhow::Result<()> {
    let Some(agent) = crate::inventory::get_agent(state, name).await? else {
        return Ok(());
    };
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

    // recent pane text as context part (best effort; last 2000 chars)
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
    let parts = [
        Part::text(format!("{} is blocked and waiting for input.", agent.name)),
        Part::data(serde_json::json!({ "context": context_tail })),
    ];
    crate::messaging::insert(
        state,
        crate::messaging::NewMessage {
            task_id: None,
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
