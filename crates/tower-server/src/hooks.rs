//! Job lifecycle prompts (D§5.2.3; decision: job-lifecycle-hooks).
//!
//! A job may carry a pre-run prompt (typed before its job prompt, on every
//! delivery) and a post-run prompt (typed after its owner closes it). Both
//! go to the harness verbatim, so `/compact`, `/clear`, or `/reflect` stay
//! harness commands.
//!
//! - **pre**: the assignment `UPDATE` marks `hook = 'pre'` (lease expiry
//!   pauses). A background runner waits for the agent to settle, sends the
//!   prompt, waits for it to settle, then clears the mark, renews the lease,
//!   and sends the delegation — the job prompt never overtakes it.
//! - **post**: the closing `UPDATE` marks `hook = 'post'`; the owner counts
//!   as busy (no assignment, manual or dispatched) until the runner has
//!   sent the prompt and the agent settled. Then the dispatcher runs.
//! - A lifecycle prompt never blocks the job: a send error or a timeout
//!   (`[jobs] hook_timeout_s`) is recorded and the sequence goes on.
//! - Sequences run in their own tasks: callers (the pump, routes, the
//!   sweep) never wait on a `/compact`. The send takes the per-agent lock;
//!   the waits hold none.
//! - Boot clears marks a crash left behind (`recover`).

use tower_core::{
    Agent, EventKind, HookPhase, MessageKind, MessageStatus, Part, PartyKind, Task, TaskId,
    TaskState,
};
use tower_driver::{DriverError, HarnessState};

use crate::state::AppState;

/// How long a sent prompt has to show `working`/`blocked` before tower
/// assumes it already finished (an instant command like `/clear` may never
/// show it) — herdr's own `prompt --wait` window.
const TAKEN_UP_MS: u64 = 5_000;

const READY: [HarnessState; 2] = [HarnessState::Idle, HarnessState::Done];
const TAKEN_UP: [HarnessState; 2] = [HarnessState::Working, HarnessState::Blocked];

/// A lifecycle prompt as stored: trimmed (a slash command must start the
/// input), blank → none.
pub fn prompt_text(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// After a successful assignment: the delegation, preceded by the job's
/// pre-run prompt when the assignment marked one.
pub async fn deliver(state: &AppState, agent: &Agent, task: &Task, by: &str) {
    if task.hook != Some(HookPhase::Pre) {
        crate::tasks::notify_assignee(state, agent, task, by).await;
        return;
    }
    let (state, agent, task, by) = (state.clone(), agent.clone(), task.clone(), by.to_string());
    tokio::spawn(async move {
        if let Err(e) = run_pre(&state, &agent, &task, &by).await {
            tracing::warn!(error = %e, task = %task.id, agent = %agent.name, "pre-run sequence failed");
        }
    });
}

/// After the owner closed `task`: its post-run prompt, when the close
/// marked one.
pub fn closed(state: &AppState, owner: &Agent, task: &Task) {
    if task.hook != Some(HookPhase::Post) {
        return;
    }
    let (state, owner, task) = (state.clone(), owner.clone(), task.clone());
    tokio::spawn(async move {
        if let Err(e) = run_post(&state, &owner, &task).await {
            tracing::warn!(error = %e, task = %task.id, agent = %owner.name, "post-run sequence failed");
        }
    });
}

async fn run_pre(state: &AppState, agent: &Agent, task: &Task, by: &str) -> anyhow::Result<()> {
    let text = task.pre_run.as_deref().unwrap_or_default();
    let outcome = run_prompt(state, agent, task, HookPhase::Pre, text).await;
    record(state, task, agent, HookPhase::Pre, outcome).await;
    // clear the mark and restart the lease in one step — unless cancel,
    // release, or a reassignment took this delivery away meanwhile
    let now = tower_core::now_ms();
    let res = sqlx::query(
        "UPDATE tasks SET hook = NULL, updated_at = ?1,
                lease_expires_at = CASE WHEN state IN ('assigned','working','input-required')
                                        THEN ?1 + lease_s * 1000 ELSE lease_expires_at END
         WHERE id = ?2 AND owner_id = ?3 AND hook = 'pre'",
    )
    .bind(now)
    .bind(task.id.0.as_str())
    .bind(agent.id.0.as_str())
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(());
    }
    match crate::tasks::get_task(state, &task.id).await? {
        Some(t) if t.state == TaskState::Assigned && t.owner_id.as_ref() == Some(&agent.id) => {
            crate::tasks::notify_assignee(state, agent, &t, by).await;
        }
        // the agent declared start on its own: it has the job already
        _ => {}
    }
    Ok(())
}

async fn run_post(state: &AppState, owner: &Agent, task: &Task) -> anyhow::Result<()> {
    let text = task.post_run.as_deref().unwrap_or_default();
    let outcome = run_prompt(state, owner, task, HookPhase::Post, text).await;
    record(state, task, owner, HookPhase::Post, outcome).await;
    sqlx::query("UPDATE tasks SET hook = NULL, updated_at = ?1 WHERE id = ?2 AND hook = 'post'")
        .bind(tower_core::now_ms())
        .bind(task.id.0.as_str())
        .execute(&state.pool)
        .await?;
    // the agent is free now: its next reserved job may go (if it's idle)
    crate::tasks::dispatch_for(state, &owner.id, tower_core::now_ms()).await?;
    Ok(())
}

/// One lifecycle prompt: settle → send (recorded as a `prompt` message) →
/// taken up → settled. `Err` says what went wrong, for the trail.
async fn run_prompt(
    state: &AppState,
    agent: &Agent,
    task: &Task,
    phase: HookPhase,
    text: &str,
) -> Result<(), String> {
    let settle = state.config.jobs.hook_timeout_s.saturating_mul(1000);
    let driver = &state.driver;
    driver
        .wait_until(&agent.name, &READY, settle)
        .await
        .map_err(|e| format!("agent not ready: {e}"))?;
    send(state, agent, task, phase, text)
        .await
        .map_err(|e| format!("send failed: {e:#}"))?;
    if let Err(e) = event(state, &task.id, &agent.id.0, phase, "running", None).await {
        tracing::warn!(error = %e, task = %task.id, "task.hook event failed");
    }
    match driver.wait_until(&agent.name, &TAKEN_UP, TAKEN_UP_MS).await {
        Ok(()) | Err(DriverError::Timeout(_)) => {}
        Err(e) => return Err(format!("lost the agent: {e}")),
    }
    driver
        .wait_until(&agent.name, &READY, settle)
        .await
        .map_err(|e| format!("did not settle: {e}"))
}

/// Record the prompt as a message row, then type it into the harness.
async fn send(
    state: &AppState,
    agent: &Agent,
    task: &Task,
    phase: HookPhase,
    text: &str,
) -> anyhow::Result<()> {
    let parts = [
        Part::text(text),
        Part::data(serde_json::json!({ "task_id": task.id, "hook": phase })),
    ];
    let id = crate::messaging::insert(
        state,
        crate::messaging::NewMessage {
            task_id: Some(&task.id),
            from_kind: PartyKind::Human,
            from_id: "me",
            to_kind: PartyKind::Agent,
            to_id: &agent.name,
            kind: MessageKind::Prompt,
            parts: &parts,
            deadline_s: None,
        },
    )
    .await?;
    if let Err(e) = crate::messaging::deliver_prompt(state, &agent.name, &parts).await {
        crate::messaging::set_status(state, &id, MessageStatus::Failed).await?;
        return Err(e);
    }
    Ok(())
}

/// The outcome on the trail. Never fails the sequence: a lost event must
/// not leave the mark behind.
async fn record(
    state: &AppState,
    task: &Task,
    agent: &Agent,
    phase: HookPhase,
    outcome: Result<(), String>,
) {
    let written = match outcome {
        Ok(()) => event(state, &task.id, &agent.id.0, phase, "done", None).await,
        Err(error) => {
            tracing::warn!(task = %task.id, agent = %agent.name, phase = phase.as_str(), %error, "lifecycle prompt failed");
            event(state, &task.id, &agent.id.0, phase, "failed", Some(error)).await
        }
    };
    if let Err(e) = written {
        tracing::warn!(error = %e, task = %task.id, "task.hook event failed");
    }
}

async fn event(
    state: &AppState,
    task: &TaskId,
    owner: &str,
    phase: HookPhase,
    status: &str,
    error: Option<String>,
) -> anyhow::Result<()> {
    state
        .events
        .append(
            EventKind::TaskHook,
            Some("task"),
            Some(task.0.as_str()),
            serde_json::json!({
                "task_id": task, "owner_id": owner, "phase": phase,
                "status": status, "error": error,
            }),
        )
        .await?;
    Ok(())
}

/// Boot (D§5.2.3): a restart abandoned every running lifecycle prompt.
/// Clear the marks so leases tick and agents count as free again; a job
/// that waited on its pre-run prompt is then recovered by its lease.
/// Returns the number cleared.
pub async fn recover(state: &AppState) -> anyhow::Result<usize> {
    let marked: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, hook, owner_id FROM tasks WHERE hook IS NOT NULL")
            .fetch_all(&state.pool)
            .await?;
    let now = tower_core::now_ms();
    let mut cleared = 0;
    for (id, hook, owner) in marked {
        let res = sqlx::query(
            "UPDATE tasks SET hook = NULL, updated_at = ?1 WHERE id = ?2 AND hook = ?3",
        )
        .bind(now)
        .bind(&id)
        .bind(&hook)
        .execute(&state.pool)
        .await?;
        if res.rows_affected() == 0 {
            continue;
        }
        cleared += 1;
        let phase = if hook == "post" {
            HookPhase::Post
        } else {
            HookPhase::Pre
        };
        event(
            state,
            &TaskId::from(id),
            owner.as_deref().unwrap_or_default(),
            phase,
            "interrupted",
            Some("server restarted".into()),
        )
        .await?;
    }
    Ok(cleared)
}
