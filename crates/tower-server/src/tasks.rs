//! Tasks module (D§5.2.1, D§9.4, plan T3.1-T3.3): the assignment-only job
//! queue.
//!
//! - **Assign** (operator; orchestrator later) is the only way a job gains
//!   an owner. It first sweeps the job's expired lease, then CASes
//!   `owner_id IS NULL AND state='queued'` — a live-owned job can never be
//!   assigned to another agent. There is no claim/pull operation.
//! - **Owner writes** (start/heartbeat/status/release) CAS on
//!   `owner_id = :as`. Ownership ends only when a lease is swept (sweeper
//!   tick or assign-time sweep), so a late but un-swept owner can still
//!   finish instead of the work being redone.
//! - **Leases**: every assign/start/heartbeat/non-terminal status extends
//!   `lease_expires_at` by the task's `lease_s`. Expiry requeues with an
//!   attempt bump, or fails with `lease_exhausted` at `max_attempts`.
//!   `input-required` pauses expiry (D§5.2). Heartbeats write no events —
//!   the row is the truth; the log records transitions only.
//!
//! Every service fn takes `now` (ms) so tests inject the clock.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::Row;
use tower_core::{
    Agent, AgentId, AgentState, EventKind, MessageKind, Part, PartyKind, Task, TaskId, TaskState,
    TowerError,
};

use crate::http::error_response;
use crate::state::AppState;
use crate::storage::{enum_str, parse_enum};

/// Default lease window (D§5.2.1).
pub const DEFAULT_LEASE_S: i64 = 60;

/// Owner-writable (non-terminal, owned) states.
const OWNED: &str = "('assigned','working','input-required')";
/// States whose lease can expire (`input-required` pauses expiry).
const EXPIRABLE: &str = "('assigned','working')";
const TERMINAL: &str = "('completed','failed','canceled','rejected')";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/tasks", get(list_route).post(create_route))
        .route("/v1/tasks/{id}", get(show_route))
        .route("/v1/tasks/{id}/assign", post(assign_route))
        .route("/v1/tasks/{id}/start", post(start_route))
        .route("/v1/tasks/{id}/heartbeat", post(heartbeat_route))
        .route("/v1/tasks/{id}/status", post(status_route))
        .route("/v1/tasks/{id}/release", post(release_route))
        .route("/v1/tasks/{id}/cancel", post(cancel_route))
}

// ---- request shapes ------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateTask {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Pre-assign to this agent (name or id).
    #[serde(default)]
    pub assign: Option<String>,
    /// With `assign`: reserve instead — deliver when the agent is available.
    #[serde(default)]
    pub when_available: bool,
    /// With `assign`: reserve, and hold back from dispatch until this time
    /// (ms). Implies `when_available`.
    #[serde(default)]
    pub not_before: Option<i64>,
    #[serde(default)]
    pub lease_s: Option<i64>,
    #[serde(default)]
    pub max_attempts: Option<i64>,
}

#[derive(Deserialize)]
pub struct AssignBody {
    pub to: String,
    #[serde(default)]
    pub lease_s: Option<i64>,
    /// Reserve instead of assigning now: deliver when the agent is free.
    #[serde(default)]
    pub when_available: bool,
}

#[derive(Deserialize)]
pub struct OwnerBody {
    /// The owning agent (name or id) making the call.
    #[serde(rename = "as")]
    pub as_agent: String,
}

#[derive(Deserialize)]
pub struct StatusBody {
    #[serde(rename = "as")]
    pub as_agent: String,
    #[serde(default)]
    pub state: Option<TaskState>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ReleaseBody {
    #[serde(rename = "as")]
    pub as_agent: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ListFilter {
    #[serde(default)]
    pub state: Option<TaskState>,
    /// Comma-separated; a task matches when it carries every tag.
    #[serde(default)]
    pub tags: Option<String>,
    /// Owner (name or id), any state.
    #[serde(default)]
    pub owner: Option<String>,
    /// Owner (name or id), open jobs only — the agent's work-loop view.
    #[serde(default)]
    pub mine: Option<String>,
    /// `updated_at > since` (ms).
    #[serde(default)]
    pub since: Option<i64>,
}

// ---- service -------------------------------------------------------------

pub async fn create(state: &AppState, req: CreateTask, now: i64) -> anyhow::Result<Task> {
    if req.title.trim().is_empty() {
        return Err(TowerError::invalid("title must not be empty").into());
    }
    let lease_s = req.lease_s.unwrap_or(DEFAULT_LEASE_S);
    let max_attempts = req.max_attempts.unwrap_or(3);
    if lease_s <= 0 || max_attempts < 1 {
        return Err(TowerError::invalid("lease_s must be > 0 and max_attempts >= 1").into());
    }
    // validate the pre-assignee before writing anything
    match &req.assign {
        Some(to) if req.when_available || req.not_before.is_some() => {
            resolve_agent(state, to).await?; // a reservation may wait for a dead agent
        }
        Some(to) => {
            assignable_agent(state, to).await?;
        }
        None if req.when_available || req.not_before.is_some() => {
            return Err(TowerError::invalid(
                "when_available / not_before need an `assign` target (nothing else dispatches it)",
            )
            .into());
        }
        None => {}
    }

    let id = TaskId::new();
    sqlx::query(
        "INSERT INTO tasks (id, title, description, state, priority, tags, max_attempts, lease_s, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?8)",
    )
    .bind(id.0.as_str())
    .bind(&req.title)
    .bind(&req.description)
    .bind(req.priority.unwrap_or(0))
    .bind(serde_json::to_string(&req.tags)?)
    .bind(max_attempts)
    .bind(lease_s)
    .bind(now)
    .execute(&state.pool)
    .await?;
    event(
        state,
        EventKind::TaskCreated,
        &id,
        serde_json::json!({
            "task_id": id, "title": req.title,
            "priority": req.priority.unwrap_or(0), "tags": req.tags,
        }),
    )
    .await?;

    match req.assign {
        Some(to) if req.when_available || req.not_before.is_some() => {
            reserve(state, &id, &to, req.not_before, "me", now).await
        }
        Some(to) => assign(state, &id, &to, None, "me", now).await,
        None => require(state, &id).await,
    }
}

/// Reserve a queued job for an agent (D§5.2.2): it stays `queued` and the
/// dispatcher assigns it when the agent is available (and `not_before` has
/// passed). Re-reserving re-targets. Delivery is attempted right away.
pub async fn reserve(
    state: &AppState,
    id: &TaskId,
    to: &str,
    not_before: Option<i64>,
    by: &str,
    now: i64,
) -> anyhow::Result<Task> {
    let agent = resolve_agent(state, to).await?;
    let res = sqlx::query(
        "UPDATE tasks SET target_agent_id = ?1, not_before = COALESCE(?2, not_before), updated_at = ?3
         WHERE id = ?4 AND owner_id IS NULL AND state = 'queued'",
    )
    .bind(agent.id.0.as_str())
    .bind(not_before)
    .bind(now)
    .bind(id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(conflict_reason(state, id, None).await);
    }
    let task = require(state, id).await?;
    event(
        state,
        EventKind::TaskReserved,
        id,
        serde_json::json!({
            "task_id": id, "target": agent.id,
            "not_before": task.not_before, "by": by,
        }),
    )
    .await?;
    dispatch_for(state, &agent.id, now).await?;
    require(state, id).await
}

/// Deliver the next reserved job to `agent` if it is available (D§5.2.2):
/// row `idle`/`done`, no open job. Highest priority, then oldest, first;
/// `not_before` must have passed. The one-job rule is enforced atomically
/// by `assign`, so racing dispatch paths can't double-book the agent.
pub async fn dispatch_for(
    state: &AppState,
    agent: &AgentId,
    now: i64,
) -> anyhow::Result<Option<Task>> {
    let Some(a) = crate::inventory::get_agent(state, &agent.0).await? else {
        return Ok(None);
    };
    if !matches!(a.state, AgentState::Idle | AgentState::Done) {
        return Ok(None);
    }
    let due: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, schedule_id FROM tasks
         WHERE target_agent_id = ?1 AND state = 'queued' AND owner_id IS NULL
           AND (not_before IS NULL OR not_before <= ?2)
         ORDER BY priority DESC, created_at ASC",
    )
    .bind(agent.0.as_str())
    .bind(now)
    .fetch_all(&state.pool)
    .await?;
    for (id, schedule) in due {
        let by = match &schedule {
            Some(s) => format!("schedule:{s}"),
            None => "dispatch".to_string(),
        };
        match assign(state, &TaskId::from(id), &agent.0, None, &by, now).await {
            Ok(t) => return Ok(Some(t)),
            // agent became busy, or this job was just taken: stop / next
            Err(e) if e.downcast_ref::<TowerError>().is_some() => {
                if agent_busy(state, agent).await? {
                    return Ok(None);
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Dispatcher sweep: try every agent that has reserved jobs due.
pub async fn dispatch_all(state: &AppState, now: i64) -> anyhow::Result<usize> {
    let targets: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT target_agent_id FROM tasks
         WHERE target_agent_id IS NOT NULL AND state = 'queued' AND owner_id IS NULL
           AND (not_before IS NULL OR not_before <= ?1)",
    )
    .bind(now)
    .fetch_all(&state.pool)
    .await?;
    let mut delivered = 0;
    for t in targets {
        if dispatch_for(state, &AgentId::from(t), now).await?.is_some() {
            delivered += 1;
        }
    }
    Ok(delivered)
}

async fn agent_busy(state: &AppState, agent: &AgentId) -> anyhow::Result<bool> {
    Ok(open_job_of(state, agent).await?.is_some())
}

async fn open_job_of(state: &AppState, agent: &AgentId) -> anyhow::Result<Option<TaskId>> {
    let open: Option<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM tasks WHERE owner_id = ?1 AND state IN {OWNED} LIMIT 1"
    ))
    .bind(agent.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    Ok(open.map(TaskId::from))
}

/// Assign a queued job to an agent (D§5.2.1). `by` is the dispatching
/// principal (operator `me`; the orchestrator later).
pub async fn assign(
    state: &AppState,
    id: &TaskId,
    to: &str,
    lease_s: Option<i64>,
    by: &str,
    now: i64,
) -> anyhow::Result<Task> {
    let agent = assignable_agent(state, to).await?;
    if lease_s.is_some_and(|s| s <= 0) {
        return Err(TowerError::invalid("lease_s must be > 0").into());
    }
    // one expiry code path: an expired lease is swept before the CAS
    expire_lease(state, id, now).await?;

    // one statement = one atomic check: the job is unowned AND the agent owns
    // no other open job (one job per agent, D§5.2.2). Assigning a job
    // reserved for someone else is an explicit override: the reservation
    // is dropped.
    let res = sqlx::query(&format!(
        "UPDATE tasks SET owner_id = ?1, state = 'assigned',
                lease_s = COALESCE(?2, lease_s),
                lease_expires_at = ?3 + COALESCE(?2, lease_s) * 1000,
                target_agent_id = CASE WHEN target_agent_id = ?1 THEN target_agent_id END,
                updated_at = ?3
         WHERE id = ?4 AND owner_id IS NULL AND state = 'queued'
           AND NOT EXISTS (SELECT 1 FROM tasks o WHERE o.owner_id = ?1 AND o.state IN {OWNED})"
    ))
    .bind(agent.id.0.as_str())
    .bind(lease_s)
    .bind(now)
    .bind(id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        if let Some(busy_with) = open_job_of(state, &agent.id).await? {
            if busy_with != *id {
                let mut err = TowerError::conflict(format!(
                    "{} is busy with job {busy_with} (one job at a time); \
                     reserve with --when-available to deliver it when free",
                    agent.name
                ));
                err.detail = Some(serde_json::json!({ "busy_with": busy_with }));
                return Err(err.into());
            }
        }
        return Err(conflict_reason(state, id, None).await);
    }

    let task = require(state, id).await?;
    event(
        state,
        EventKind::TaskAssigned,
        id,
        serde_json::json!({
            "task_id": id, "owner_id": agent.id,
            "lease_expires_at": task.lease_expires_at, "by": by,
        }),
    )
    .await?;
    notify_assignee(state, &agent, &task, by).await;
    Ok(task)
}

/// Owner declares work started: `assigned` → `working`, lease renewed.
pub async fn start(
    state: &AppState,
    id: &TaskId,
    as_agent: &str,
    now: i64,
) -> anyhow::Result<Task> {
    let owner = resolve_agent(state, as_agent).await?;
    owner_cas(
        state,
        id,
        &owner.id,
        "state IN ('assigned','working')",
        "state = 'working', lease_expires_at = ?3 + lease_s * 1000",
        now,
    )
    .await?;
    let task = require(state, id).await?;
    status_event(state, &task).await?;
    Ok(task)
}

/// Owner renews its lease. No event (row is the truth).
pub async fn heartbeat(
    state: &AppState,
    id: &TaskId,
    as_agent: &str,
    now: i64,
) -> anyhow::Result<Task> {
    let owner = resolve_agent(state, as_agent).await?;
    owner_cas(
        state,
        id,
        &owner.id,
        &format!("state IN {OWNED}"),
        "lease_expires_at = ?3 + lease_s * 1000",
        now,
    )
    .await?;
    require(state, id).await
}

/// Owner status report (D§5.2.1): `working | input-required` renew the
/// lease; `completed | failed` close the task (lease cleared, result kept).
pub async fn report(
    state: &AppState,
    id: &TaskId,
    as_agent: &str,
    new_state: Option<TaskState>,
    result: Option<serde_json::Value>,
    now: i64,
) -> anyhow::Result<Task> {
    let owner = resolve_agent(state, as_agent).await?;
    let result_json = result.as_ref().map(serde_json::to_string).transpose()?;
    let set = match new_state {
        None => "result = COALESCE(?4, result), lease_expires_at = ?3 + lease_s * 1000",
        Some(TaskState::Working | TaskState::InputRequired) => {
            "state = ?5, result = COALESCE(?4, result), lease_expires_at = ?3 + lease_s * 1000"
        }
        Some(TaskState::Completed | TaskState::Failed) => {
            "state = ?5, result = COALESCE(?4, result), lease_expires_at = NULL"
        }
        Some(other) => {
            return Err(TowerError::invalid(format!(
                "owners report working|input-required|completed|failed, not {}",
                enum_str(other)
            ))
            .into())
        }
    };
    let sql = format!(
        "UPDATE tasks SET {set}, updated_at = ?3
         WHERE id = ?1 AND owner_id = ?2 AND state IN {OWNED}"
    );
    let res = sqlx::query(&sql)
        .bind(id.0.as_str())
        .bind(owner.id.0.as_str())
        .bind(now)
        .bind(result_json)
        .bind(new_state.map(enum_str))
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(conflict_reason(state, id, Some(&owner.id)).await);
    }

    let task = require(state, id).await?;
    status_event(state, &task).await?;
    let closing = match task.state {
        TaskState::Completed => Some(EventKind::TaskCompleted),
        TaskState::Failed => Some(EventKind::TaskFailed),
        _ => None,
    };
    if let Some(kind) = closing {
        event(
            state,
            kind,
            id,
            serde_json::json!({"task_id": id, "owner_id": owner.id, "result": task.result}),
        )
        .await?;
        // the owner is free: its next reserved job may go (if it's idle)
        dispatch_for(state, &owner.id, now).await?;
    }
    Ok(task)
}

/// Owner voluntarily gives the job back to the queue (no attempt bump).
pub async fn release(
    state: &AppState,
    id: &TaskId,
    as_agent: &str,
    reason: Option<String>,
    now: i64,
) -> anyhow::Result<Task> {
    let owner = resolve_agent(state, as_agent).await?;
    owner_cas(
        state,
        id,
        &owner.id,
        &format!("state IN {OWNED}"),
        // a job reserved for the releaser would be dispatched straight back:
        // releasing it drops that reservation (general queue)
        "owner_id = NULL, state = 'queued', lease_expires_at = NULL,
         target_agent_id = CASE WHEN target_agent_id = ?2 THEN NULL ELSE target_agent_id END",
        now,
    )
    .await?;
    let task = require(state, id).await?;
    event(
        state,
        EventKind::TaskStatus,
        id,
        serde_json::json!({
            "task_id": id, "state": task.state,
            "released_by": owner.id, "reason": reason,
        }),
    )
    .await?;
    dispatch_for(state, &owner.id, now).await?;
    Ok(task)
}

/// Operator cancel: any non-terminal job → `canceled`; the owning agent
/// (if any) is interrupted (D§7).
pub async fn cancel(state: &AppState, id: &TaskId, now: i64) -> anyhow::Result<Task> {
    let before = require(state, id).await?;
    let res = sqlx::query(&format!(
        "UPDATE tasks SET state = 'canceled', lease_expires_at = NULL, updated_at = ?1
         WHERE id = ?2 AND state NOT IN {TERMINAL}"
    ))
    .bind(now)
    .bind(id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(conflict_reason(state, id, None).await);
    }
    if let Some(owner) = &before.owner_id {
        if let Some(agent) = crate::inventory::get_agent(state, &owner.0).await? {
            if let Err(e) = state.driver.interrupt(&agent.name).await {
                tracing::warn!(error = %e, agent = %agent.name, "cancel: interrupt failed");
            }
        }
    }
    let task = require(state, id).await?;
    status_event(state, &task).await?;
    if let Some(owner) = &before.owner_id {
        dispatch_for(state, owner, now).await?;
    }
    Ok(task)
}

/// Sweep one task's lease if it expired (D§5.2.1): requeue with an attempt
/// bump, or `failed` with `lease_exhausted` once attempts hit
/// `max_attempts`. CAS on the observed owner + lease so a concurrent
/// heartbeat or sweep wins cleanly. Returns true when this call swept it.
pub async fn expire_lease(state: &AppState, id: &TaskId, now: i64) -> anyhow::Result<bool> {
    let row = sqlx::query(&format!(
        "SELECT owner_id, attempt_count, max_attempts, lease_expires_at FROM tasks
         WHERE id = ?1 AND owner_id IS NOT NULL AND state IN {EXPIRABLE}
           AND lease_expires_at < ?2"
    ))
    .bind(id.0.as_str())
    .bind(now)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else { return Ok(false) };
    let owner: String = row.get("owner_id");
    let attempts: i64 = row.get::<i64, _>("attempt_count") + 1;
    let max: i64 = row.get("max_attempts");
    let lease: i64 = row.get("lease_expires_at");
    let exhausted = attempts >= max;

    let set = if exhausted {
        "state = 'failed', lease_expires_at = NULL, result = '{\"error\":\"lease_exhausted\"}'"
    } else {
        "owner_id = NULL, state = 'queued', lease_expires_at = NULL"
    };
    let res = sqlx::query(&format!(
        "UPDATE tasks SET {set}, attempt_count = ?1, updated_at = ?2
         WHERE id = ?3 AND owner_id = ?4 AND lease_expires_at = ?5 AND state IN {EXPIRABLE}"
    ))
    .bind(attempts)
    .bind(now)
    .bind(id.0.as_str())
    .bind(&owner)
    .bind(lease)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(false);
    }

    let new_state = if exhausted { "failed" } else { "queued" };
    event(
        state,
        EventKind::TaskLeasedOut,
        id,
        serde_json::json!({
            "task_id": id, "prior_owner": owner,
            "attempt_count": attempts, "max_attempts": max, "state": new_state,
        }),
    )
    .await?;
    if exhausted {
        event(
            state,
            EventKind::TaskFailed,
            id,
            serde_json::json!({
                "task_id": id, "owner_id": owner,
                "result": {"error": "lease_exhausted"},
            }),
        )
        .await?;
    }
    Ok(true)
}

/// Lease sweeper pass (D§9.4): sweep every expired lease; returns the count.
pub async fn sweep_leases(state: &AppState, now: i64) -> anyhow::Result<usize> {
    let ids: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM tasks
         WHERE owner_id IS NOT NULL AND state IN {EXPIRABLE} AND lease_expires_at < ?1"
    ))
    .bind(now)
    .fetch_all(&state.pool)
    .await?;
    let mut swept = 0;
    for id in ids {
        if expire_lease(state, &TaskId::from(id), now).await? {
            swept += 1;
        }
    }
    Ok(swept)
}

/// Agent removal (D§5.2.1 "reassign, not orphan"): release its open jobs
/// back to the queue (no attempt bump) and detach its finished ones so the
/// agent row can be deleted. History stays in the event log. Returns the
/// number of jobs released.
pub async fn detach_agent(state: &AppState, agent: &Agent, now: i64) -> anyhow::Result<usize> {
    // first: jobs reserved for it fall back to the general queue (decision
    // table) — before releasing, so a departing agent is never offered work
    let reserved: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM tasks WHERE target_agent_id = ?1 AND state NOT IN ('completed','failed','canceled','rejected')",
    )
    .bind(agent.id.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    sqlx::query("UPDATE tasks SET target_agent_id = NULL WHERE target_agent_id = ?1")
        .bind(agent.id.0.as_str())
        .execute(&state.pool)
        .await?;
    for id in reserved {
        event(
            state,
            EventKind::TaskReserved,
            &TaskId::from(id.clone()),
            serde_json::json!({"task_id": id, "target": null, "reason": "target_removed"}),
        )
        .await?;
    }
    let open: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM tasks WHERE owner_id = ?1 AND state IN {OWNED}"
    ))
    .bind(agent.id.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    let mut released = 0;
    for id in open {
        let id = TaskId::from(id);
        match release(state, &id, &agent.id.0, Some("agent removed".into()), now).await {
            Ok(_) => released += 1,
            // a concurrent sweep/cancel got there first — nothing to release
            Err(e) if e.downcast_ref::<TowerError>().is_some() => {}
            Err(e) => return Err(e),
        }
    }
    sqlx::query("UPDATE tasks SET owner_id = NULL WHERE owner_id = ?1")
        .bind(agent.id.0.as_str())
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE tasks SET agent_id = NULL WHERE agent_id = ?1")
        .bind(agent.id.0.as_str())
        .execute(&state.pool)
        .await?;
    Ok(released)
}

/// Agent-state → task-state mapping (D§5.2): an owner going `blocked` puts
/// its working job in `input-required` (pausing lease expiry); leaving
/// `blocked` for `working` resumes it with a fresh lease.
pub async fn sync_agent_state(
    state: &AppState,
    agent: &AgentId,
    agent_state: AgentState,
    now: i64,
) -> anyhow::Result<()> {
    let (from, set) = match agent_state {
        AgentState::Blocked => ("working", "state = 'input-required'"),
        AgentState::Working => (
            "input-required",
            "state = 'working', lease_expires_at = ?2 + lease_s * 1000",
        ),
        _ => return Ok(()),
    };
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM tasks WHERE owner_id = ?1 AND state = ?2")
            .bind(agent.0.as_str())
            .bind(from)
            .fetch_all(&state.pool)
            .await?;
    for id in ids {
        let res = sqlx::query(&format!(
            "UPDATE tasks SET {set}, updated_at = ?2 WHERE id = ?1 AND owner_id = ?3 AND state = ?4"
        ))
        .bind(&id)
        .bind(now)
        .bind(agent.0.as_str())
        .bind(from)
        .execute(&state.pool)
        .await?;
        if res.rows_affected() == 1 {
            let task = require(state, &TaskId::from(id)).await?;
            status_event(state, &task).await?;
        }
    }
    Ok(())
}

/// Queue / inventory query (D§7): `priority DESC, created_at ASC`.
pub async fn list(state: &AppState, f: &ListFilter) -> anyhow::Result<Vec<Task>> {
    let owner = match f.mine.as_deref().or(f.owner.as_deref()) {
        Some(o) => Some(resolve_agent(state, o).await?.id),
        None => None,
    };
    let tags: Vec<&str> = f
        .tags
        .as_deref()
        .map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let rows = sqlx::query(&format!(
        "SELECT * FROM tasks
         WHERE (?1 IS NULL OR state = ?1)
           AND (?2 IS NULL OR owner_id = ?2)
           AND (?3 = 0 OR state NOT IN {TERMINAL})
           AND (?4 IS NULL OR updated_at > ?4)
           AND NOT EXISTS (
             SELECT 1 FROM json_each(?5) want
             WHERE want.value NOT IN (SELECT value FROM json_each(tasks.tags)))
         ORDER BY priority DESC, created_at ASC"
    ))
    .bind(f.state.map(enum_str))
    .bind(owner.as_ref().map(|o| o.0.as_str()))
    .bind(f.mine.is_some())
    .bind(f.since)
    .bind(serde_json::to_string(&tags)?)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_task).collect())
}

pub async fn get_task(state: &AppState, id: &TaskId) -> anyhow::Result<Option<Task>> {
    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?1")
        .bind(id.0.as_str())
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.as_ref().map(row_to_task))
}

/// Assignment/lease/status trail: the task's events, oldest first.
pub async fn trail(state: &AppState, id: &TaskId) -> anyhow::Result<Vec<tower_core::Event>> {
    let seqs: Vec<i64> = sqlx::query_scalar(
        "SELECT seq FROM events WHERE subject_type = 'task' AND subject_id = ?1 ORDER BY seq",
    )
    .bind(id.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::with_capacity(seqs.len());
    for seq in seqs {
        out.extend(state.events.since(seq - 1, 1).await?);
    }
    Ok(out)
}

pub fn row_to_task(r: &sqlx::sqlite::SqliteRow) -> Task {
    let tags: String = r.get("tags");
    let result: Option<String> = r.get("result");
    let agent_id: Option<String> = r.get("agent_id");
    let owner_id: Option<String> = r.get("owner_id");
    Task {
        id: TaskId::from(r.get::<String, _>("id")),
        agent_id: agent_id.map(AgentId::from),
        owner_id: owner_id.map(AgentId::from),
        origin: r.get("origin"),
        external_ref: r.get("external_ref"),
        context_id: r.get("context_id"),
        title: r.get("title"),
        description: r.get("description"),
        state: parse_enum(r.get("state")),
        priority: r.get("priority"),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        attempt_count: r.get("attempt_count"),
        max_attempts: r.get("max_attempts"),
        lease_expires_at: r.get("lease_expires_at"),
        lease_s: r.get("lease_s"),
        target_agent_id: r
            .get::<Option<String>, _>("target_agent_id")
            .map(AgentId::from),
        not_before: r.get("not_before"),
        schedule_id: r
            .get::<Option<String>, _>("schedule_id")
            .map(tower_core::ScheduleId::from),
        occurrence_at: r.get("occurrence_at"),
        result: result.and_then(|s| serde_json::from_str(&s).ok()),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

// ---- helpers -------------------------------------------------------------

async fn require(state: &AppState, id: &TaskId) -> anyhow::Result<Task> {
    get_task(state, id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("task {id} not found")).into())
}

async fn resolve_agent(state: &AppState, name_or_id: &str) -> anyhow::Result<Agent> {
    crate::inventory::get_agent(state, name_or_id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name_or_id} not found")).into())
}

async fn assignable_agent(state: &AppState, name_or_id: &str) -> anyhow::Result<Agent> {
    let agent = resolve_agent(state, name_or_id).await?;
    if agent.state == AgentState::Dead {
        return Err(TowerError::invalid(format!("agent {} is dead", agent.name)).into());
    }
    Ok(agent)
}

/// Owner-only CAS: `SET {set}` where `owner_id = owner AND {cond}`.
/// Bind order: ?1 id, ?2 owner, ?3 now.
async fn owner_cas(
    state: &AppState,
    id: &TaskId,
    owner: &AgentId,
    cond: &str,
    set: &str,
    now: i64,
) -> anyhow::Result<()> {
    let sql = format!(
        "UPDATE tasks SET {set}, updated_at = ?3 WHERE id = ?1 AND owner_id = ?2 AND {cond}"
    );
    let res = sqlx::query(&sql)
        .bind(id.0.as_str())
        .bind(owner.0.as_str())
        .bind(now)
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(conflict_reason(state, id, Some(owner)).await);
    }
    Ok(())
}

/// Explain a failed CAS: `not_found`, or `conflict` naming the reason.
async fn conflict_reason(
    state: &AppState,
    id: &TaskId,
    as_owner: Option<&AgentId>,
) -> anyhow::Error {
    let task = match get_task(state, id).await {
        Ok(Some(t)) => t,
        Ok(None) => return TowerError::not_found(format!("task {id} not found")).into(),
        Err(e) => return e,
    };
    let owner_name = match &task.owner_id {
        Some(o) => match crate::inventory::get_agent(state, &o.0).await {
            Ok(Some(a)) => a.name,
            _ => o.0.clone(),
        },
        None => String::new(),
    };
    let msg = if task.state.is_terminal() {
        format!("task {id} is {}", enum_str(task.state))
    } else {
        match (&task.owner_id, as_owner) {
            (Some(o), Some(me)) if o != me => format!("task {id} is owned by {owner_name}"),
            (None, Some(_)) => {
                format!("task {id} is not assigned (state {})", enum_str(task.state))
            }
            (Some(_), None) => format!("task {id} is already owned by {owner_name}"),
            _ => format!("task {id} is {}", enum_str(task.state)),
        }
    };
    let mut err = TowerError::conflict(msg);
    err.detail = Some(serde_json::json!({
        "state": task.state, "owner_id": task.owner_id,
        "lease_expires_at": task.lease_expires_at,
    }));
    err.into()
}

async fn event(
    state: &AppState,
    kind: EventKind,
    id: &TaskId,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    state
        .events
        .append(kind, Some("task"), Some(id.0.as_str()), payload)
        .await?;
    Ok(())
}

async fn status_event(state: &AppState, task: &Task) -> anyhow::Result<()> {
    event(
        state,
        EventKind::TaskStatus,
        &task.id,
        serde_json::json!({"task_id": task.id, "state": task.state, "owner_id": task.owner_id}),
    )
    .await
}

/// Assignment notice (D§5.2.1): a `delegation` message prompts the owner.
/// Failure is logged, never fatal — the lease covers an agent that never
/// starts.
async fn notify_assignee(state: &AppState, agent: &Agent, task: &Task, by: &str) {
    let mut text = format!("Job {} is assigned to you: {}", task.id, task.title);
    if let Some(d) = &task.description {
        text.push_str(&format!("\n\n{d}"));
    }
    text.push_str(&format!(
        "\n\nYou own this job exclusively — never take other work from the queue. \
         Declare start, heartbeat at least every {}s, and report the outcome \
         (completed|failed); release it if you cannot do it. MCP: tower_task_start / \
         tower_task_heartbeat / tower_task_status / tower_task_release. Shell: \
         `tower task start|heartbeat|status|release {}`. Contract: docs/agent-loop.md.",
        (task.lease_s / 3).max(1),
        task.id
    ));
    let parts = [
        Part::text(text),
        Part::data(serde_json::json!({ "task_id": task.id })),
    ];
    let sent = async {
        let id = crate::messaging::insert(
            state,
            crate::messaging::NewMessage {
                task_id: Some(&task.id),
                from_kind: PartyKind::Human,
                from_id: by,
                to_kind: PartyKind::Agent,
                to_id: &agent.name,
                kind: MessageKind::Delegation,
                parts: &parts,
                deadline_s: None,
            },
        )
        .await?;
        if let Err(e) = crate::messaging::deliver_prompt(state, &agent.name, &parts).await {
            crate::messaging::set_status(state, &id, tower_core::MessageStatus::Failed).await?;
            return Err(e);
        }
        anyhow::Ok(())
    };
    if let Err(e) = sent.await {
        tracing::warn!(error = %e, task = %task.id, agent = %agent.name, "assignment notice failed");
    }
}

// ---- route handlers ------------------------------------------------------

fn task_json(r: anyhow::Result<Task>) -> Response {
    match r {
        Ok(t) => Json(serde_json::json!({ "task": t })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn create_route(State(s): State<AppState>, Json(b): Json<CreateTask>) -> Response {
    match create(&s, b, tower_core::now_ms()).await {
        Ok(t) => (StatusCode::CREATED, Json(serde_json::json!({ "task": t }))).into_response(),
        Err(e) => error_response(e),
    }
}

async fn list_route(State(s): State<AppState>, Query(f): Query<ListFilter>) -> Response {
    match list(&s, &f).await {
        Ok(tasks) => Json(serde_json::json!({ "tasks": tasks })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn show_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let id = TaskId::from(id);
    let res = async {
        let task = require(&s, &id).await?;
        let trail = trail(&s, &id).await?;
        let messages: Vec<tower_core::Message> =
            sqlx::query("SELECT * FROM messages WHERE task_id = ?1 ORDER BY created_at")
                .bind(id.0.as_str())
                .fetch_all(&s.pool)
                .await?
                .iter()
                .map(crate::messaging::row_to_message)
                .collect();
        anyhow::Ok(serde_json::json!({ "task": task, "trail": trail, "messages": messages }))
    };
    match res.await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

async fn assign_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<AssignBody>,
) -> Response {
    let (id, now) = (TaskId::from(id), tower_core::now_ms());
    task_json(if b.when_available {
        reserve(&s, &id, &b.to, None, "me", now).await
    } else {
        assign(&s, &id, &b.to, b.lease_s, "me", now).await
    })
}

async fn start_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<OwnerBody>,
) -> Response {
    task_json(start(&s, &TaskId::from(id), &b.as_agent, tower_core::now_ms()).await)
}

async fn heartbeat_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<OwnerBody>,
) -> Response {
    task_json(heartbeat(&s, &TaskId::from(id), &b.as_agent, tower_core::now_ms()).await)
}

async fn status_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<StatusBody>,
) -> Response {
    task_json(
        report(
            &s,
            &TaskId::from(id),
            &b.as_agent,
            b.state,
            b.result,
            tower_core::now_ms(),
        )
        .await,
    )
}

async fn release_route(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<ReleaseBody>,
) -> Response {
    task_json(
        release(
            &s,
            &TaskId::from(id),
            &b.as_agent,
            b.reason,
            tower_core::now_ms(),
        )
        .await,
    )
}

async fn cancel_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    task_json(cancel(&s, &TaskId::from(id), tower_core::now_ms()).await)
}
