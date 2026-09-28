//! Periodic sweeper (D§9.3, D§9.4): message deadlines, task leases,
//! schedule firing, and the reserved-job dispatcher backstop.
//!
//! Every sweep takes `now` explicitly so tests inject the clock; the tick
//! loop passes `now_ms()`.

use std::time::Duration;

use tower_core::{MessageId, MessageKind, MessageStatus, Part, PartyKind};

use crate::messaging;
use crate::state::AppState;
use crate::storage::parse_enum;

/// Sweep cadence (D§9.4: ~10s).
pub const TICK: Duration = Duration::from_secs(10);

/// Event retention runs this often (D§17.6); deletes are cheap but hourly
/// is plenty for a day-granular horizon.
pub const PRUNE_EVERY_MS: i64 = 60 * 60 * 1000;

/// Prompt sent to an agent whose question expired unanswered (D§9.3).
pub const EXPIRED_QUESTION_PROMPT: &str =
    "No answer arrived before the deadline. Proceed with your default or fallback approach, or stop if you cannot.";

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        let mut last_prune = i64::MIN;
        loop {
            tick.tick().await;
            let now = tower_core::now_ms();
            if now.saturating_sub(last_prune) >= PRUNE_EVERY_MS {
                last_prune = now;
                if let Err(e) = sweep_events(&state, now).await {
                    tracing::warn!(error = %e, "sweeper: event retention failed");
                }
            }
            if let Err(e) = sweep_deadlines(&state, now).await {
                tracing::warn!(error = %e, "sweeper: deadline sweep failed");
            }
            if let Err(e) = crate::tasks::sweep_leases(&state, now).await {
                tracing::warn!(error = %e, "sweeper: lease sweep failed");
            }
            if let Err(e) = crate::schedules::fire_due(&state, now).await {
                tracing::warn!(error = %e, "sweeper: schedule firing failed");
            }
            if let Err(e) = crate::tasks::dispatch_all(&state, now).await {
                tracing::warn!(error = %e, "sweeper: dispatch failed");
            }
        }
    })
}

/// Delete events older than `server.event_retention_days` (0 keeps
/// everything); open `/v1/events` streams hold back what they have not
/// sent. Returns the number deleted.
pub async fn sweep_events(state: &AppState, now: i64) -> anyhow::Result<u64> {
    let days = i64::from(state.config.server.event_retention_days);
    if days == 0 {
        return Ok(0);
    }
    let deleted = state.events.prune(now - days * 86_400_000).await?;
    if deleted > 0 {
        tracing::info!(deleted, days, "sweeper: pruned expired events");
    }
    Ok(deleted)
}

/// Expire `pending` questions/approvals with `deadline_at <= now`; returns
/// the number expired. Each expiry is a CAS on `status='pending'`, so a
/// response racing the sweep wins or loses cleanly — never both.
pub async fn sweep_deadlines(state: &AppState, now: i64) -> anyhow::Result<usize> {
    let due: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, kind, from_kind, from_id FROM messages
         WHERE status='pending' AND deadline_at IS NOT NULL AND deadline_at <= ?1",
    )
    .bind(now)
    .fetch_all(&state.pool)
    .await?;

    let mut expired = 0;
    for (id, kind, from_kind, from_id) in due {
        let res =
            sqlx::query("UPDATE messages SET status='expired' WHERE id=?1 AND status='pending'")
                .bind(&id)
                .execute(&state.pool)
                .await?;
        if res.rows_affected() == 0 {
            continue; // answered in the meantime
        }
        expired += 1;
        let id = MessageId::from(id);
        state
            .events
            .append(
                tower_core::EventKind::MessageStatusChange,
                Some("message"),
                Some(id.0.as_str()),
                serde_json::json!({ "status": MessageStatus::Expired }),
            )
            .await?;

        if parse_enum::<PartyKind>(from_kind) != PartyKind::Agent {
            continue;
        }
        let kind: MessageKind = parse_enum(kind);
        let notified = if kind == MessageKind::Approval {
            // an unattended permission is never granted: deny = esc (D§9.3)
            messaging::answer_dialog(state, &from_id, false).await
        } else {
            messaging::deliver_prompt(state, &from_id, &[Part::text(EXPIRED_QUESTION_PROMPT)]).await
        };
        if let Err(e) = &notified {
            tracing::warn!(error = %e, agent = %from_id, "sweeper: expiry notification failed");
        }
        state
            .events
            .append(
                tower_core::EventKind::ApprovalExpired,
                Some("message"),
                Some(id.0.as_str()),
                serde_json::json!({
                    "message_id": id,
                    "agent_id": from_id,
                    "kind": kind,
                    "notified": notified.is_ok(),
                }),
            )
            .await?;
    }
    Ok(expired)
}
