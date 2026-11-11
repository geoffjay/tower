//! Agent claims (D§5.6; decision: agent-availability). A run claims every
//! agent its definition names before its first step, all or nothing, and
//! keeps them until it ends. A claimed agent gets only its run's jobs (the
//! dispatcher reads `run_claims`, `tasks.rs`). A run that cannot claim
//! waits; waiting runs are admitted oldest first, so one claim order for
//! everybody and no partial holds: no deadlock.
//!
//! Every fn here runs under `AppState::workflow_lock`.

use serde::Serialize;
use sqlx::Row;
use tower_core::{AgentId, RunId, WorkflowRun};

use super::def::Definition;
use crate::state::AppState;

/// An agent a waiting run waits for, and the run that claims it (or the
/// earlier waiting run that wants it first).
#[derive(Debug, Clone, Serialize)]
pub struct Blocker {
    pub agent: String,
    pub run_id: RunId,
}

/// The agents `def` names that exist now, `(id, name)`. A missing agent is
/// skipped: its step fails with "agent not found", as before claims.
pub(super) async fn wanted(
    state: &AppState,
    def: &Definition,
) -> anyhow::Result<Vec<(AgentId, String)>> {
    let mut out = Vec::new();
    for name in def.agent_names() {
        if let Some(a) = crate::inventory::get_agent(state, name).await? {
            out.push((a.id, a.name));
        }
    }
    Ok(out)
}

/// What keeps `run` from claiming `wanted`: each agent another run claims,
/// else each agent an earlier waiting run wants. Empty: it may claim.
pub(super) async fn blockers(
    state: &AppState,
    run: &WorkflowRun,
    wanted: &[(AgentId, String)],
) -> anyhow::Result<Vec<Blocker>> {
    let mut out = Vec::new();
    let mut free = Vec::new();
    for (id, name) in wanted {
        let holder: Option<String> = sqlx::query_scalar(
            "SELECT run_id FROM run_claims WHERE agent_id = ?1 AND run_id != ?2",
        )
        .bind(id.0.as_str())
        .bind(run.id.0.as_str())
        .fetch_optional(&state.pool)
        .await?;
        match holder {
            Some(r) => out.push(Blocker {
                agent: name.clone(),
                run_id: RunId::from(r),
            }),
            None => free.push(name.as_str()),
        }
    }
    if free.is_empty() {
        return Ok(out);
    }
    // first come, first served: an earlier waiting run keeps its place
    let earlier = sqlx::query(
        "SELECT id, workflow_id, version FROM workflow_runs
         WHERE state = 'waiting' AND (created_at, id) < (?1, ?2)
         ORDER BY created_at, id",
    )
    .bind(run.created_at)
    .bind(run.id.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    for r in &earlier {
        let def = super::definition(
            state,
            &tower_core::WorkflowId::from(r.get::<String, _>("workflow_id")),
            r.get("version"),
        )
        .await?;
        let names = def.agent_names();
        let ahead = RunId::from(r.get::<String, _>("id"));
        free.retain(|name| {
            if names.contains(name) {
                out.push(Blocker {
                    agent: (*name).to_string(),
                    run_id: ahead.clone(),
                });
                false
            } else {
                true
            }
        });
        if free.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// Claim every wanted agent for `run` in one transaction (the caller saw
/// no blockers under the same lock).
pub(super) async fn claim(
    state: &AppState,
    run: &RunId,
    wanted: &[(AgentId, String)],
    now: i64,
) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    for (id, _) in wanted {
        sqlx::query("INSERT INTO run_claims (agent_id, run_id, claimed_at) VALUES (?1, ?2, ?3)")
            .bind(id.0.as_str())
            .bind(run.0.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Drop `run`'s claims. The caller admits waiting runs before it hands the
/// released agents other work (`settle_claims`).
pub(super) async fn release(state: &AppState, run: &RunId) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM run_claims WHERE run_id = ?1")
        .bind(run.0.as_str())
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// For a waiting run's detail: what it waits for now.
pub(super) async fn waiting_for(
    state: &AppState,
    run: &WorkflowRun,
    def: &Definition,
) -> anyhow::Result<Vec<Blocker>> {
    let wanted = wanted(state, def).await?;
    blockers(state, run, &wanted).await
}
