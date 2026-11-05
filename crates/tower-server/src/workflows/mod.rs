//! Workflows (D§5.6, D§9.7; decision: workflows): versioned graphs of job
//! nodes, their runs, and the engine that drives a run from the event log.
//!
//! - **apply** stores a version when the definition changed and syncs the
//!   managed schedules of its schedule triggers.
//! - **engine**: every write to run and step rows holds
//!   `AppState::workflow_lock`, and every transition is a CAS on the step's
//!   state, so a repeated event or a reconcile pass changes nothing twice.
//! - **consumer**: follows `events` from a durable cursor
//!   (`event_consumers`), woken by the log's watch head: a step's job going
//!   terminal finishes the step; a matching event trigger starts a run.
//! - **reconcile** (boot + every sweep): the same transitions from the rows,
//!   for events lost between a state write and its append.
//!
//! Every service fn takes `now` (ms) so tests inject the clock.

pub mod def;

use std::collections::VecDeque;
use std::fmt::Write as _;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use tower_core::{
    AgentId, EventKind, RunId, RunState, Schedule, ScheduleId, StepId, StepState, Task, TaskId,
    TaskState, TowerError, Workflow, WorkflowId, WorkflowRun, WorkflowStep,
};

use crate::http::error_response;
use crate::state::AppState;
use crate::storage::{enum_str, parse_enum};
use def::{AgentRef, Definition, Join, Node, Trigger};

/// Name of the engine's row in `event_consumers`.
pub const CONSUMER: &str = "workflows";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/workflows", get(list_route).post(apply_route))
        .route("/v1/workflows/{name}", get(show_route).delete(remove_route))
        .route("/v1/workflows/{name}/enable", post(enable_route))
        .route("/v1/workflows/{name}/disable", post(disable_route))
        .route("/v1/workflows/{name}/runs", post(run_route))
        .route("/v1/runs", get(runs_route))
        .route("/v1/runs/{id}", get(run_show_route))
        .route("/v1/runs/{id}/cancel", post(cancel_route))
}

// ---- definitions -----------------------------------------------------------

/// What `apply` did.
#[derive(Debug, serde::Serialize)]
pub struct Applied {
    pub workflow: Workflow,
    pub version: i64,
    pub changed: bool,
}

/// Validate `source` and store it as the workflow's next version when the
/// definition changed (operator-only, D§7).
pub async fn apply(state: &AppState, source: &str, now: i64) -> anyhow::Result<Applied> {
    let def = def::parse(source)?;
    for name in def.agent_names() {
        if crate::inventory::get_agent(state, name).await?.is_none() {
            return Err(TowerError::invalid(format!(
                "agent `{name}` not found: spawn it before applying the workflow"
            ))
            .into());
        }
    }
    ensure_cursor(state).await?;
    let _guard = state.workflow_lock.lock().await;
    let json = serde_json::to_string(&def)?;
    let (id, version, changed) = match get_workflow(state, &def.name).await? {
        None => {
            let id = WorkflowId::new();
            sqlx::query(
                "INSERT INTO workflows (id, name, description, enabled, current_version,
                                        created_at, updated_at)
                 VALUES (?1, ?2, ?3, 1, 1, ?4, ?4)",
            )
            .bind(id.0.as_str())
            .bind(&def.name)
            .bind(&def.description)
            .bind(now)
            .execute(&state.pool)
            .await?;
            insert_version(state, &id, 1, source, &json, now).await?;
            (id, 1, true)
        }
        Some(w) => {
            let current: String = sqlx::query_scalar(
                "SELECT definition FROM workflow_versions WHERE workflow_id = ?1 AND version = ?2",
            )
            .bind(w.id.0.as_str())
            .bind(w.current_version)
            .fetch_one(&state.pool)
            .await?;
            if current == json {
                (w.id, w.current_version, false)
            } else {
                let v = w.current_version + 1;
                insert_version(state, &w.id, v, source, &json, now).await?;
                sqlx::query(
                    "UPDATE workflows SET current_version = ?1, description = ?2, updated_at = ?3
                     WHERE id = ?4",
                )
                .bind(v)
                .bind(&def.description)
                .bind(now)
                .bind(w.id.0.as_str())
                .execute(&state.pool)
                .await?;
                (w.id, v, true)
            }
        }
    };
    let workflow = require_workflow(state, &id.0).await?;
    if changed {
        crate::schedules::sync_workflow(state, &workflow, &def, now).await?;
    }
    event(
        state,
        EventKind::WorkflowApplied,
        "workflow",
        &id.0,
        json!({ "workflow_id": id, "name": def.name, "version": version, "changed": changed }),
    )
    .await?;
    Ok(Applied {
        workflow,
        version,
        changed,
    })
}

async fn insert_version(
    state: &AppState,
    id: &WorkflowId,
    version: i64,
    source: &str,
    json: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workflow_versions (workflow_id, version, source, definition, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id.0.as_str())
    .bind(version)
    .bind(source)
    .bind(json)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Every workflow with its current version's shape and open run count.
pub async fn list(state: &AppState) -> anyhow::Result<Vec<Value>> {
    let rows = sqlx::query("SELECT * FROM workflows ORDER BY name")
        .fetch_all(&state.pool)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let w = row_to_workflow(r);
        let d = definition(state, &w.id, w.current_version).await?;
        let open: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id = ?1 AND state = 'running'",
        )
        .bind(w.id.0.as_str())
        .fetch_one(&state.pool)
        .await?;
        let mut v = serde_json::to_value(&w)?;
        v["nodes"] = json!(d.nodes.len());
        v["triggers"] = serde_json::to_value(&d.triggers)?;
        v["open_runs"] = json!(open);
        out.push(v);
    }
    Ok(out)
}

/// One workflow: current definition and source, managed schedules, and its
/// 20 newest runs.
pub async fn show(state: &AppState, name: &str) -> anyhow::Result<Value> {
    let w = require_workflow(state, name).await?;
    let row = sqlx::query(
        "SELECT source, definition FROM workflow_versions WHERE workflow_id = ?1 AND version = ?2",
    )
    .bind(w.id.0.as_str())
    .bind(w.current_version)
    .fetch_one(&state.pool)
    .await?;
    let definition: Value = serde_json::from_str(&row.get::<String, _>("definition"))?;
    let runs = list_runs(
        state,
        &RunFilter {
            workflow: Some(w.id.0.clone()),
            state: None,
            limit: Some(20),
        },
    )
    .await?;
    let schedules = crate::schedules::managed(state, &w.id).await?;
    Ok(json!({
        "workflow": w, "definition": definition, "source": row.get::<String, _>("source"),
        "schedules": schedules, "runs": runs,
    }))
}

/// Remove a workflow, its versions, and its schedules; finished runs keep
/// their rows. Refused while a run is `running`.
pub async fn remove(state: &AppState, name: &str, _now: i64) -> anyhow::Result<()> {
    let _guard = state.workflow_lock.lock().await;
    let w = require_workflow(state, name).await?;
    let open: Option<String> = sqlx::query_scalar(
        "SELECT id FROM workflow_runs WHERE workflow_id = ?1 AND state = 'running' LIMIT 1",
    )
    .bind(w.id.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    if let Some(run) = open {
        let mut e = TowerError::conflict(format!(
            "workflow {} has a running run ({run}): cancel it first",
            w.name
        ));
        e.detail = Some(json!({ "run_id": run }));
        return Err(e.into());
    }
    crate::schedules::remove_managed(state, &w.id).await?;
    sqlx::query("DELETE FROM workflows WHERE id = ?1")
        .bind(w.id.0.as_str())
        .execute(&state.pool)
        .await?;
    event(
        state,
        EventKind::WorkflowRemoved,
        "workflow",
        &w.id.0,
        json!({ "workflow_id": w.id, "name": w.name }),
    )
    .await
}

/// Enable or disable: a disabled workflow refuses manual starts, its
/// schedules pause, and its event triggers stop matching.
pub async fn set_enabled(
    state: &AppState,
    name: &str,
    enabled: bool,
    now: i64,
) -> anyhow::Result<Workflow> {
    let w = require_workflow(state, name).await?;
    if w.enabled == enabled {
        return Ok(w);
    }
    sqlx::query("UPDATE workflows SET enabled = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(enabled)
        .bind(now)
        .bind(w.id.0.as_str())
        .execute(&state.pool)
        .await?;
    for s in crate::schedules::managed(state, &w.id).await? {
        if enabled {
            crate::schedules::resume(state, &s.id, now).await?;
        } else {
            crate::schedules::pause(state, &s.id, "workflow_disabled", now).await?;
        }
    }
    let kind = if enabled {
        EventKind::WorkflowEnabled
    } else {
        EventKind::WorkflowDisabled
    };
    event(
        state,
        kind,
        "workflow",
        &w.id.0,
        json!({ "workflow_id": w.id, "name": w.name }),
    )
    .await?;
    require_workflow(state, &w.id.0).await
}

pub async fn is_enabled(state: &AppState, id: &WorkflowId) -> anyhow::Result<bool> {
    Ok(get_workflow(state, &id.0).await?.is_some_and(|w| w.enabled))
}

/// By name or id.
pub async fn get_workflow(state: &AppState, name_or_id: &str) -> anyhow::Result<Option<Workflow>> {
    let row = sqlx::query("SELECT * FROM workflows WHERE name = ?1 OR id = ?1")
        .bind(name_or_id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.as_ref().map(row_to_workflow))
}

async fn require_workflow(state: &AppState, name_or_id: &str) -> anyhow::Result<Workflow> {
    get_workflow(state, name_or_id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("workflow {name_or_id} not found")).into())
}

async fn definition(state: &AppState, id: &WorkflowId, version: i64) -> anyhow::Result<Definition> {
    let json: String = sqlx::query_scalar(
        "SELECT definition FROM workflow_versions WHERE workflow_id = ?1 AND version = ?2",
    )
    .bind(id.0.as_str())
    .bind(version)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| TowerError::not_found(format!("workflow {id} version {version} not found")))?;
    Ok(serde_json::from_str(&json)?)
}

// ---- runs ------------------------------------------------------------------

/// Start a run of `name`'s current version (manual, schedule, or event
/// trigger). Checks the input against `[inputs]`.
pub async fn start_run(
    state: &AppState,
    name: &str,
    input: Map<String, Value>,
    trigger: Value,
    now: i64,
) -> anyhow::Result<WorkflowRun> {
    ensure_cursor(state).await?;
    let _guard = state.workflow_lock.lock().await;
    let w = require_workflow(state, name).await?;
    if !w.enabled {
        return Err(TowerError::conflict(format!(
            "workflow {} is disabled: `tower workflow enable {}` first",
            w.name, w.name
        ))
        .into());
    }
    let def = definition(state, &w.id, w.current_version).await?;
    let input = resolve_input(&def, input)?;
    let run = WorkflowRun {
        id: RunId::new(),
        workflow_id: w.id.clone(),
        workflow_name: w.name.clone(),
        version: w.current_version,
        state: RunState::Running,
        input: Value::Object(input),
        trigger,
        result: None,
        error: None,
        created_at: now,
        ended_at: None,
    };
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_id, workflow_name, version, state, input, trigger,
                                    created_at)
         VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7)",
    )
    .bind(run.id.0.as_str())
    .bind(w.id.0.as_str())
    .bind(&w.name)
    .bind(run.version)
    .bind(run.input.to_string())
    .bind(run.trigger.to_string())
    .bind(now)
    .execute(&state.pool)
    .await?;
    event(
        state,
        EventKind::RunStarted,
        "workflow_run",
        &run.id.0,
        json!({
            "run_id": run.id, "workflow_id": w.id, "workflow": w.name, "version": run.version,
            "trigger": run.trigger, "input": run.input,
        }),
    )
    .await?;
    let id = run.id.clone();
    let mut engine = Engine::new(state, run, def, now);
    for n in engine.def.start_nodes() {
        engine.queue.push_back(Action::Activate {
            node: n.id.clone(),
            from: None,
        });
    }
    engine.drain().await?;
    require_run(state, &id).await
}

fn resolve_input(
    def: &Definition,
    given: Map<String, Value>,
) -> anyhow::Result<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in given {
        if !def.inputs.contains_key(&k) {
            let declared: Vec<&str> = def.inputs.keys().map(String::as_str).collect();
            return Err(TowerError::invalid(format!(
                "unknown input `{k}` (declared: {})",
                if declared.is_empty() {
                    "none".to_string()
                } else {
                    declared.join(", ")
                }
            ))
            .into());
        }
        out.insert(k, v);
    }
    for (k, spec) in &def.inputs {
        if out.contains_key(k) {
            continue;
        }
        match &spec.default {
            Some(d) => {
                out.insert(k.clone(), d.clone());
            }
            None if spec.required => {
                return Err(TowerError::invalid(format!("missing required input `{k}`")).into())
            }
            None => {}
        }
    }
    Ok(out)
}

/// Cancel a running run: its open steps are canceled and their jobs
/// canceled (interrupting their owners). Operator-only.
pub async fn cancel_run(state: &AppState, id: &RunId, now: i64) -> anyhow::Result<WorkflowRun> {
    let _guard = state.workflow_lock.lock().await;
    let Some(mut engine) = Engine::load(state, id, now).await? else {
        let run = require_run(state, id).await?;
        return Err(TowerError::conflict(format!("run {id} is {}", run.state.as_str())).into());
    };
    engine.end(RunState::Canceled, None).await?;
    require_run(state, id).await
}

#[derive(Deserialize, Default)]
pub struct RunFilter {
    /// Workflow name or id.
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub state: Option<RunState>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Runs newest first (default 50, at most 500).
pub async fn list_runs(state: &AppState, f: &RunFilter) -> anyhow::Result<Vec<WorkflowRun>> {
    let rows = sqlx::query(
        "SELECT * FROM workflow_runs
         WHERE (?1 IS NULL OR workflow_id = ?1 OR workflow_name = ?1)
           AND (?2 IS NULL OR state = ?2)
         ORDER BY created_at DESC, id DESC LIMIT ?3",
    )
    .bind(&f.workflow)
    .bind(f.state.map(|s| s.as_str()))
    .bind(f.limit.unwrap_or(50).clamp(1, 500))
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_run).collect())
}

/// The runs a workflow schedule started, newest first.
pub async fn runs_of_schedule(
    state: &AppState,
    schedule: &ScheduleId,
    limit: i64,
) -> anyhow::Result<Vec<WorkflowRun>> {
    let rows = sqlx::query(
        "SELECT * FROM workflow_runs WHERE json_extract(trigger, '$.schedule_id') = ?1
         ORDER BY created_at DESC LIMIT ?2",
    )
    .bind(schedule.0.as_str())
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_run).collect())
}

/// A run with its steps in order.
pub async fn show_run(state: &AppState, id: &RunId) -> anyhow::Result<Value> {
    let run = require_run(state, id).await?;
    let steps = steps_of(state, id).await?;
    Ok(json!({ "run": run, "steps": steps }))
}

pub async fn get_run(state: &AppState, id: &RunId) -> anyhow::Result<Option<WorkflowRun>> {
    let row = sqlx::query("SELECT * FROM workflow_runs WHERE id = ?1")
        .bind(id.0.as_str())
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.as_ref().map(row_to_run))
}

async fn require_run(state: &AppState, id: &RunId) -> anyhow::Result<WorkflowRun> {
    get_run(state, id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("run {id} not found")).into())
}

async fn steps_of(state: &AppState, run: &RunId) -> anyhow::Result<Vec<WorkflowStep>> {
    let rows = sqlx::query(
        "SELECT * FROM workflow_steps WHERE run_id = ?1 ORDER BY created_at, visit, id",
    )
    .bind(run.0.as_str())
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.iter().map(row_to_step).collect())
}

// ---- triggers --------------------------------------------------------------

/// What a workflow schedule's firing did.
pub enum Scheduled {
    Started(Box<WorkflowRun>),
    /// The run this schedule started last is still running.
    Running(RunId),
}

/// A managed schedule fired (decision §5): start a run, or report the
/// schedule's still-running run.
pub async fn fire_schedule(
    state: &AppState,
    s: &Schedule,
    occurrence: i64,
    now: i64,
) -> anyhow::Result<Scheduled> {
    let Some(workflow) = &s.workflow_id else {
        return Err(TowerError::invalid(format!("schedule {} starts no workflow", s.id)).into());
    };
    let running: Option<String> = sqlx::query_scalar(
        "SELECT id FROM workflow_runs
         WHERE workflow_id = ?1 AND state = 'running' AND json_extract(trigger, '$.schedule_id') = ?2
         LIMIT 1",
    )
    .bind(workflow.0.as_str())
    .bind(s.id.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    if let Some(r) = running {
        return Ok(Scheduled::Running(RunId::from(r)));
    }
    let input = match &s.workflow_input {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let trigger = json!({ "kind": "schedule", "schedule_id": s.id, "occurrence_at": occurrence });
    Ok(Scheduled::Started(Box::new(
        start_run(state, &workflow.0, input, trigger, now).await?,
    )))
}

/// An enabled workflow's event trigger.
struct EventTrigger {
    workflow: WorkflowId,
    name: String,
    index: usize,
    event_type: String,
    matches: Map<String, Value>,
    input: Map<String, Value>,
}

async fn event_triggers(state: &AppState) -> anyhow::Result<Vec<EventTrigger>> {
    let rows = sqlx::query(
        "SELECT w.id, w.name, v.definition FROM workflows w
         JOIN workflow_versions v ON v.workflow_id = w.id AND v.version = w.current_version
         WHERE w.enabled = 1 AND v.definition LIKE '%\"kind\":\"event\"%'",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let def: Definition = serde_json::from_str(&r.get::<String, _>("definition"))?;
        for (index, t) in def.triggers.into_iter().enumerate() {
            if let Trigger::Event {
                event_type,
                matches,
                input,
            } = t
            {
                out.push(EventTrigger {
                    workflow: WorkflowId::from(r.get::<String, _>("id")),
                    name: r.get("name"),
                    index,
                    event_type,
                    matches,
                    input,
                });
            }
        }
    }
    Ok(out)
}

/// The event as one object for matching and templates: its payload plus
/// the envelope's `type`, `seq`, `subject_type`, `subject_id`.
fn event_object(e: &tower_core::Event) -> Value {
    let mut obj = match &e.payload {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    obj.insert("type".into(), json!(e.kind.as_str()));
    obj.insert("seq".into(), json!(e.seq));
    obj.insert("subject_type".into(), json!(e.subject_type));
    obj.insert("subject_id".into(), json!(e.subject_id));
    Value::Object(obj)
}

fn lookup<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .fold(v, |v, seg| v.get(seg).unwrap_or(&Value::Null))
}

/// Every `match` path equals its value, or contains it when an array.
fn trigger_matches(t: &EventTrigger, e: &tower_core::Event, obj: &Value) -> bool {
    e.kind.as_str() == t.event_type
        && t.matches
            .iter()
            .all(|(path, want)| match lookup(obj, path) {
                Value::Array(items) => items.contains(want),
                got => got == want,
            })
}

/// Whether `e` is about one of `workflow`'s own runs or jobs (a run must
/// never trigger itself).
async fn about_workflow(
    state: &AppState,
    e: &tower_core::Event,
    workflow: &WorkflowId,
) -> anyhow::Result<bool> {
    let Some(subject) = e.subject_id.as_deref() else {
        return Ok(false);
    };
    let run = match e.subject_type.as_deref() {
        Some("workflow") => return Ok(subject == workflow.0),
        Some("workflow_run") => Some(subject.to_string()),
        Some("task") => sqlx::query_scalar::<_, Option<String>>(
            "SELECT context_id FROM tasks WHERE id = ?1 AND origin = 'workflow'",
        )
        .bind(subject)
        .fetch_optional(&state.pool)
        .await?
        .flatten(),
        _ => None,
    };
    let Some(run) = run else { return Ok(false) };
    let owner: Option<String> =
        sqlx::query_scalar("SELECT workflow_id FROM workflow_runs WHERE id = ?1")
            .bind(&run)
            .fetch_optional(&state.pool)
            .await?;
    Ok(owner.as_deref() == Some(workflow.0.as_str()))
}

async fn fire_event_trigger(
    state: &AppState,
    t: &EventTrigger,
    e: &tower_core::Event,
    obj: &Value,
    now: i64,
) -> anyhow::Result<()> {
    if about_workflow(state, e, &t.workflow).await? {
        return Ok(());
    }
    let running: Option<String> = sqlx::query_scalar(
        "SELECT id FROM workflow_runs
         WHERE workflow_id = ?1 AND state = 'running'
           AND json_extract(trigger, '$.kind') = 'event' AND json_extract(trigger, '$.trigger') = ?2
         LIMIT 1",
    )
    .bind(t.workflow.0.as_str())
    .bind(t.index as i64)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(r) = running {
        tracing::debug!(workflow = %t.name, run = %r, seq = e.seq, "event trigger skipped: previous run still running");
        return Ok(());
    }
    let ctx = json!({ "event": obj });
    let input: Map<String, Value> = t
        .input
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(s) => Value::String(def::render(s, &ctx)),
                other => other.clone(),
            };
            (k.clone(), v)
        })
        .collect();
    let trigger = json!({
        "kind": "event", "trigger": t.index, "event_type": t.event_type, "event_seq": e.seq,
    });
    start_run(state, &t.workflow.0, input, trigger, now).await?;
    Ok(())
}

// ---- result contract ---------------------------------------------------------

fn contract_error(reason: &str, msg: String) -> anyhow::Error {
    let mut e = TowerError::invalid(msg);
    e.detail = Some(json!({ "reason": reason }));
    e.into()
}

/// The result contract of a workflow job's `completed` report (decision
/// §4): `{summary ≤ 2000 chars, outcome ∈ node outcomes, data ≤ 16 KB}`; a
/// string result is `{summary}`. Returns the normalized result; jobs that
/// are not workflow steps (or reports by a non-owner, which the report's
/// CAS refuses) pass through unchanged.
pub async fn check_report(
    state: &AppState,
    task: &TaskId,
    owner: &AgentId,
    result: Option<Value>,
) -> anyhow::Result<Option<Value>> {
    let row = sqlx::query(
        "SELECT s.node_id, r.workflow_id, r.version, t.result AS task_result, t.owner_id
         FROM workflow_steps s
         JOIN workflow_runs r ON r.id = s.run_id
         JOIN tasks t ON t.id = s.task_id
         WHERE s.task_id = ?1",
    )
    .bind(task.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else { return Ok(result) };
    if row.get::<Option<String>, _>("owner_id").as_deref() != Some(owner.0.as_str()) {
        return Ok(result);
    }
    let def = definition(
        state,
        &WorkflowId::from(row.get::<String, _>("workflow_id")),
        row.get("version"),
    )
    .await?;
    let node_id: String = row.get("node_id");
    let Some(node) = def.node(&node_id) else {
        return Ok(result);
    };
    let effective = result.or_else(|| {
        row.get::<Option<String>, _>("task_result")
            .and_then(|s| serde_json::from_str(&s).ok())
    });
    let mut obj = match effective {
        Some(Value::Object(m)) => m,
        Some(Value::String(s)) => {
            let mut m = Map::new();
            m.insert("summary".into(), Value::String(s));
            m
        }
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(contract_error(
            "summary_required",
            "a workflow step's result is an object {summary, outcome?, data?} or a summary string"
                .into(),
        )),
    };
    let summary = obj
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if summary.is_empty() {
        return Err(contract_error(
            "summary_required",
            format!(
                "step `{node_id}` needs a summary: report result {{\"summary\": \"what you did and what the next step needs\"}} (at most {} characters)",
                def::SUMMARY_MAX_CHARS
            ),
        ));
    }
    let chars = summary.chars().count();
    if chars > def::SUMMARY_MAX_CHARS {
        return Err(contract_error(
            "summary_too_long",
            format!(
                "summary is {chars} characters; the limit is {}: shorten it and attach the rest as an artifact",
                def::SUMMARY_MAX_CHARS
            ),
        ));
    }
    let summary = summary.to_string();
    obj.insert("summary".into(), Value::String(summary));
    if !node.outcomes.is_empty() {
        let outcome = obj.get("outcome").and_then(Value::as_str);
        if !outcome.is_some_and(|o| node.outcomes.iter().any(|x| x == o)) {
            return Err(contract_error(
                "outcome_invalid",
                format!(
                    "step `{node_id}` needs an outcome: report result.outcome as one of {}",
                    node.outcomes
                        .iter()
                        .map(|o| format!("`{o}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    if let Some(data) = obj.get("data") {
        let size = data.to_string().len();
        if size > def::DATA_MAX_BYTES {
            return Err(contract_error(
                "data_too_large",
                format!(
                    "data is {size} bytes; the limit is {}: attach it as an artifact",
                    def::DATA_MAX_BYTES
                ),
            ));
        }
    }
    Ok(Some(Value::Object(obj)))
}

// ---- consumer + reconciliation ----------------------------------------------

/// The engine's durable cursor, created at the log head on first use (old
/// events never fire triggers).
async fn ensure_cursor(state: &AppState) -> anyhow::Result<i64> {
    if let Some(c) = load_cursor(state).await? {
        return Ok(c);
    }
    let head = state.events.latest_seq().await?;
    sqlx::query("INSERT OR IGNORE INTO event_consumers (name, cursor) VALUES (?1, ?2)")
        .bind(CONSUMER)
        .bind(head)
        .execute(&state.pool)
        .await?;
    Ok(load_cursor(state).await?.unwrap_or(head))
}

async fn load_cursor(state: &AppState) -> anyhow::Result<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT cursor FROM event_consumers WHERE name = ?1")
            .bind(CONSUMER)
            .fetch_optional(&state.pool)
            .await?,
    )
}

/// Handle every event past the cursor; returns the new cursor. A failing
/// event is logged and passed (reconciliation covers job steps).
pub async fn drain(state: &AppState, now: i64) -> anyhow::Result<i64> {
    let mut cursor = ensure_cursor(state).await?;
    loop {
        let batch = state.events.since(cursor, 256).await?;
        let Some(last) = batch.last().map(|e| e.seq) else {
            break;
        };
        let triggers = event_triggers(state).await?;
        for e in &batch {
            if let Err(err) = handle_event(state, e, &triggers, now).await {
                tracing::warn!(error = %err, seq = e.seq, kind = e.kind.as_str(), "workflows: event handling failed");
            }
        }
        cursor = last;
        sqlx::query("UPDATE event_consumers SET cursor = ?1 WHERE name = ?2")
            .bind(cursor)
            .bind(CONSUMER)
            .execute(&state.pool)
            .await?;
    }
    Ok(cursor)
}

async fn handle_event(
    state: &AppState,
    e: &tower_core::Event,
    triggers: &[EventTrigger],
    now: i64,
) -> anyhow::Result<()> {
    if matches!(
        e.kind,
        EventKind::TaskCompleted | EventKind::TaskFailed | EventKind::TaskStatus
    ) && e.subject_type.as_deref() == Some("task")
    {
        if let Some(id) = &e.subject_id {
            on_task(state, &TaskId::from(id.as_str()), now).await?;
        }
    }
    if triggers.is_empty() {
        return Ok(());
    }
    let obj = event_object(e);
    for t in triggers.iter().filter(|t| trigger_matches(t, e, &obj)) {
        if let Err(err) = fire_event_trigger(state, t, e, &obj, now).await {
            tracing::warn!(error = %err, workflow = %t.name, seq = e.seq, "workflows: event trigger failed");
        }
    }
    Ok(())
}

/// A job event: finish its running step once the job is terminal.
async fn on_task(state: &AppState, task: &TaskId, now: i64) -> anyhow::Result<()> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT id, run_id FROM workflow_steps WHERE task_id = ?1 AND state = 'running'",
    )
    .bind(task.0.as_str())
    .fetch_optional(&state.pool)
    .await?;
    let Some((step, run)) = row else {
        return Ok(());
    };
    let Some(job) = crate::tasks::get_task(state, task).await? else {
        return Ok(());
    };
    if !job.state.is_terminal() {
        return Ok(());
    }
    let _guard = state.workflow_lock.lock().await;
    let Some(mut engine) = Engine::load(state, &RunId::from(run), now).await? else {
        return Ok(());
    };
    let step = engine.step(&StepId::from(step)).await?;
    if step.state == StepState::Running {
        engine.finish(&step, &job).await?;
    }
    engine.drain().await
}

/// Advance every running run from its rows: finish steps whose job is
/// terminal, create missing jobs for `ready` steps, end quiet runs.
pub async fn reconcile(state: &AppState, now: i64) -> anyhow::Result<usize> {
    let runs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM workflow_runs WHERE state = 'running'")
            .fetch_all(&state.pool)
            .await?;
    let mut advanced = 0;
    for id in runs {
        let _guard = state.workflow_lock.lock().await;
        let Some(mut engine) = Engine::load(state, &RunId::from(id), now).await? else {
            continue;
        };
        for step in steps_of(state, &engine.run.id).await? {
            match step.state {
                StepState::Running => {
                    let job = match &step.task_id {
                        Some(t) => crate::tasks::get_task(state, t).await?,
                        None => None,
                    };
                    match job {
                        Some(job) if job.state.is_terminal() => {
                            engine.finish(&step, &job).await?;
                            advanced += 1;
                        }
                        Some(_) => {}
                        None => {
                            engine
                                .fail(&step, "the step's job is missing".into())
                                .await?;
                            advanced += 1;
                        }
                    }
                }
                StepState::Ready => {
                    if let Some(node) = engine.def.node(&step.node_id).cloned() {
                        engine.start(&node, step).await?;
                        advanced += 1;
                    }
                }
                _ => {}
            }
        }
        engine.drain().await?;
    }
    Ok(advanced)
}

/// The event-log consumer task (D§9.6): its position is registered with
/// the log before the sweeper's first prune.
pub struct Consumer {
    state: AppState,
    position: crate::storage::CursorHandle,
}

impl Consumer {
    pub async fn start(state: &AppState) -> anyhow::Result<Self> {
        let cursor = ensure_cursor(state).await?;
        Ok(Self {
            state: state.clone(),
            position: state.events.track_cursor(cursor),
        })
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut head = self.state.events.subscribe();
            if let Err(e) = reconcile(&self.state, tower_core::now_ms()).await {
                tracing::warn!(error = %e, "workflows: boot reconcile failed");
            }
            loop {
                head.borrow_and_update();
                match drain(&self.state, tower_core::now_ms()).await {
                    Ok(cursor) => self
                        .position
                        .store(cursor, std::sync::atomic::Ordering::Relaxed),
                    Err(e) => tracing::warn!(error = %e, "workflows: event consumer failed"),
                }
                if head.changed().await.is_err() {
                    break;
                }
            }
        })
    }
}

// ---- engine ------------------------------------------------------------------

/// Work for one run, processed in order (no async recursion).
enum Action {
    /// An edge fired into `node` (from no step: run start).
    Activate { node: String, from: Option<StepId> },
    /// A step reached `completed` or `failed`: route its edges.
    Closed(StepId),
}

/// One running run under `workflow_lock`.
struct Engine<'a> {
    state: &'a AppState,
    run: WorkflowRun,
    def: Definition,
    now: i64,
    queue: VecDeque<Action>,
}

impl<'a> Engine<'a> {
    fn new(state: &'a AppState, run: WorkflowRun, def: Definition, now: i64) -> Self {
        Self {
            state,
            run,
            def,
            now,
            queue: VecDeque::new(),
        }
    }

    /// The run, if it is still running.
    async fn load(state: &'a AppState, id: &RunId, now: i64) -> anyhow::Result<Option<Self>> {
        let Some(run) = get_run(state, id).await? else {
            return Ok(None);
        };
        if run.state != RunState::Running {
            return Ok(None);
        }
        let def = definition(state, &run.workflow_id, run.version).await?;
        Ok(Some(Self::new(state, run, def, now)))
    }

    /// Process queued work, then settle; repeat until nothing changes.
    async fn drain(&mut self) -> anyhow::Result<()> {
        loop {
            while let Some(action) = self.queue.pop_front() {
                if self.run.state != RunState::Running {
                    return Ok(());
                }
                match action {
                    Action::Activate { node, from } => self.activate(&node, from).await?,
                    Action::Closed(step) => self.route(&step).await?,
                }
            }
            if !self.settle().await? {
                return Ok(());
            }
        }
    }

    async fn step(&self, id: &StepId) -> anyhow::Result<WorkflowStep> {
        let row = sqlx::query("SELECT * FROM workflow_steps WHERE id = ?1")
            .bind(id.0.as_str())
            .fetch_one(&self.state.pool)
            .await?;
        Ok(row_to_step(&row))
    }

    async fn steps(&self) -> anyhow::Result<Vec<WorkflowStep>> {
        steps_of(self.state, &self.run.id).await
    }

    /// The latest completed step of `node`.
    async fn latest_completed(&self, node: &str) -> anyhow::Result<Option<WorkflowStep>> {
        Ok(self
            .steps()
            .await?
            .into_iter()
            .filter(|s| s.node_id == node && s.state == StepState::Completed)
            .max_by_key(|s| s.visit))
    }

    async fn activate(&mut self, node_id: &str, from: Option<StepId>) -> anyhow::Result<()> {
        let Some(node) = self.def.node(node_id).cloned() else {
            return Ok(()); // validated at apply time
        };
        if node.join == Join::All {
            let pending = self
                .steps()
                .await?
                .into_iter()
                .find(|s| s.node_id == node.id && s.state == StepState::Pending);
            let step = match pending {
                Some(mut p) => {
                    if let Some(f) = from {
                        if !p.activated_by.contains(&f) {
                            p.activated_by.push(f);
                            sqlx::query(
                                "UPDATE workflow_steps SET activated_by = ?1 WHERE id = ?2",
                            )
                            .bind(serde_json::to_string(&p.activated_by)?)
                            .bind(p.id.0.as_str())
                            .execute(&self.state.pool)
                            .await?;
                        }
                    }
                    p
                }
                None => {
                    self.insert_step(&node, StepState::Pending, from.into_iter().collect(), None)
                        .await?
                }
            };
            // every incoming edge fired: run now (else when the run goes quiet)
            let fired: std::collections::BTreeSet<String> = {
                let all = self.steps().await?;
                step.activated_by
                    .iter()
                    .filter_map(|id| all.iter().find(|s| &s.id == id))
                    .map(|s| s.node_id.clone())
                    .collect()
            };
            let needed = self.def.incoming_sources(&node.id);
            if needed.iter().all(|n| fired.contains(*n)) {
                self.promote(&node, step).await?;
            }
            return Ok(());
        }
        let from: Vec<StepId> = from.into_iter().collect();
        if let Some(limit) = self.over_limit(&node).await? {
            let step = self
                .insert_step(&node, StepState::Failed, from, Some(limit))
                .await?;
            self.step_event(EventKind::StepFailed, &step, json!({ "error": step.error }))
                .await?;
            self.queue.push_back(Action::Closed(step.id));
            return Ok(());
        }
        let step = self
            .insert_step(&node, StepState::Ready, from, None)
            .await?;
        self.start(&node, step).await
    }

    /// `Some(error)` when `node` already used all its visits.
    async fn over_limit(&self, node: &Node) -> anyhow::Result<Option<String>> {
        let Some(max) = node.max_visits else {
            return Ok(None);
        };
        let used = self
            .steps()
            .await?
            .iter()
            .filter(|s| s.node_id == node.id && s.state != StepState::Pending)
            .count();
        Ok((used as u32 >= max).then(|| format!("max_visits ({max}) reached")))
    }

    /// A pending join runs (or fails on its visit limit).
    async fn promote(&mut self, node: &Node, step: WorkflowStep) -> anyhow::Result<()> {
        if let Some(limit) = self.over_limit(node).await? {
            return self.fail(&step, limit).await;
        }
        let res = sqlx::query(
            "UPDATE workflow_steps SET state = 'ready' WHERE id = ?1 AND state = 'pending'",
        )
        .bind(step.id.0.as_str())
        .execute(&self.state.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(());
        }
        let step = self.step(&step.id).await?;
        self.start(node, step).await
    }

    async fn insert_step(
        &self,
        node: &Node,
        state: StepState,
        activated_by: Vec<StepId>,
        error: Option<String>,
    ) -> anyhow::Result<WorkflowStep> {
        let visit: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(visit), 0) + 1 FROM workflow_steps WHERE run_id = ?1 AND node_id = ?2",
        )
        .bind(self.run.id.0.as_str())
        .bind(&node.id)
        .fetch_one(&self.state.pool)
        .await?;
        let ended = (state == StepState::Failed).then_some(self.now);
        let step = WorkflowStep {
            id: StepId::new(),
            run_id: self.run.id.clone(),
            node_id: node.id.clone(),
            visit,
            state,
            activated_by,
            task_id: Some(TaskId::new()),
            agent_id: None,
            outcome: None,
            result: None,
            error,
            created_at: self.now,
            started_at: None,
            ended_at: ended,
        };
        sqlx::query(
            "INSERT INTO workflow_steps (id, run_id, node_id, visit, state, activated_by, task_id,
                                         error, created_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(step.id.0.as_str())
        .bind(self.run.id.0.as_str())
        .bind(&node.id)
        .bind(visit)
        .bind(state.as_str())
        .bind(serde_json::to_string(&step.activated_by)?)
        .bind(step.task_id.as_ref().map(|t| t.0.as_str()))
        .bind(&step.error)
        .bind(self.now)
        .bind(ended)
        .execute(&self.state.pool)
        .await?;
        Ok(step)
    }

    /// A `ready` step: resolve the agent, compose the job, insert it (a
    /// duplicate id inserts nothing), and mark the step running.
    async fn start(&mut self, node: &Node, step: WorkflowStep) -> anyhow::Result<()> {
        let agent = match self.agent_for(node).await? {
            Ok(a) => a,
            Err(msg) => return self.fail(&step, msg).await,
        };
        let Some(task_id) = step.task_id.clone() else {
            return self.fail(&step, "step has no job id".into()).await;
        };
        let ctx = self.render_context().await?;
        let prompt = def::render(&node.prompt, &ctx);
        let title = match &node.title {
            Some(t) => def::render(t, &ctx),
            None => format!("{}: {}", self.run.workflow_name, node.id),
        };
        let description = self.handoff(node, &step, &task_id, &prompt).await?;
        crate::tasks::create_workflow_job(
            self.state,
            crate::tasks::WorkflowJob {
                id: &task_id,
                run: &self.run.id,
                title,
                description,
                target: &agent.id,
                priority: node.priority,
                lease_s: node.lease_s,
                max_attempts: node.max_attempts,
                pre_run: node.pre_run.as_deref(),
                post_run: node.post_run.as_deref(),
            },
            self.now,
        )
        .await?;
        let res = sqlx::query(
            "UPDATE workflow_steps SET state = 'running', agent_id = ?1, started_at = ?2
             WHERE id = ?3 AND state = 'ready'",
        )
        .bind(agent.id.0.as_str())
        .bind(self.now)
        .bind(step.id.0.as_str())
        .execute(&self.state.pool)
        .await?;
        if res.rows_affected() == 1 {
            let step = self.step(&step.id).await?;
            self.step_event(
                EventKind::StepStarted,
                &step,
                json!({ "agent": agent.name, "agent_id": agent.id }),
            )
            .await?;
        }
        Ok(())
    }

    /// The node's agent, or why there is none.
    async fn agent_for(&self, node: &Node) -> anyhow::Result<Result<tower_core::Agent, String>> {
        let name = match &node.agent {
            AgentRef::Name(n) => n.clone(),
            AgentRef::Mapped(m) => {
                let outcome = self
                    .latest_completed(&m.from)
                    .await?
                    .and_then(|s| s.outcome);
                match outcome {
                    Some(o) => match m.map.get(&o) {
                        Some(a) => a.clone(),
                        None => {
                            return Ok(Err(format!(
                                "no agent mapped for outcome `{o}` of `{}`",
                                m.from
                            )))
                        }
                    },
                    None => {
                        return Ok(Err(format!(
                            "`{}` has no completed outcome to pick the agent from",
                            m.from
                        )))
                    }
                }
            }
        };
        Ok(crate::inventory::get_agent(self.state, &name)
            .await?
            .ok_or_else(|| format!("agent `{name}` not found")))
    }

    /// `{input, run, nodes: {id: {outcome, summary, data}}}` from the latest
    /// completed step of each node.
    async fn render_context(&self) -> anyhow::Result<Value> {
        let mut nodes = Map::new();
        let mut steps = self.steps().await?;
        steps.sort_by_key(|s| s.visit);
        for s in steps.iter().filter(|s| s.state == StepState::Completed) {
            let r = s.result.clone().unwrap_or(Value::Null);
            nodes.insert(
                s.node_id.clone(),
                json!({
                    "outcome": s.outcome, "summary": r.get("summary"), "data": r.get("data"),
                }),
            );
        }
        Ok(json!({ "input": self.run.input, "run": { "id": self.run.id }, "nodes": nodes }))
    }

    /// The job description: the prompt, then the run input, the context
    /// steps' outcomes, summaries and artifacts, this node's earlier visits,
    /// and the result contract (decision §4).
    async fn handoff(
        &self,
        node: &Node,
        step: &WorkflowStep,
        task: &TaskId,
        prompt: &str,
    ) -> anyhow::Result<String> {
        let mut s = String::with_capacity(prompt.len() + 1024);
        s.push_str(prompt.trim_end());
        s.push_str("\n\n---\n");
        let visit = match node.max_visits {
            Some(m) => format!("visit {} of at most {m}", step.visit),
            None => format!("visit {}", step.visit),
        };
        let _ = writeln!(
            s,
            "Workflow `{}`, run {}, step `{}` ({visit}).",
            self.run.workflow_name, self.run.id, node.id
        );
        if let Some(input) = self.run.input.as_object().filter(|m| !m.is_empty()) {
            s.push_str("\nInput:\n");
            for (k, v) in input {
                let v = v
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string());
                let _ = writeln!(s, "- {k}: {v}");
            }
        }
        let all = self.steps().await?;
        let context: Vec<String> = match &node.context {
            Some(c) => c.clone(),
            None => {
                let mut v: Vec<String> = Vec::new();
                for id in &step.activated_by {
                    if let Some(n) = all.iter().find(|x| &x.id == id).map(|x| x.node_id.clone()) {
                        if !v.contains(&n) {
                            v.push(n);
                        }
                    }
                }
                v
            }
        };
        let mut shown: Vec<&WorkflowStep> = Vec::new();
        for c in context.iter().filter(|c| **c != node.id) {
            if let Some(latest) = all
                .iter()
                .filter(|x| {
                    &x.node_id == c && matches!(x.state, StepState::Completed | StepState::Failed)
                })
                .max_by_key(|x| x.visit)
            {
                shown.push(latest);
            }
        }
        shown.extend(all.iter().filter(|x| {
            x.node_id == node.id
                && x.visit < step.visit
                && matches!(x.state, StepState::Completed | StepState::Failed)
        }));
        shown.sort_by_key(|x| (x.ended_at.unwrap_or(x.created_at), x.visit));
        if !shown.is_empty() {
            s.push_str("\nEarlier steps:\n");
            for x in shown {
                let mut head = format!("- `{}` visit {}", x.node_id, x.visit);
                if x.node_id == node.id {
                    head.push_str(" (this step, earlier)");
                }
                if let Some(o) = &x.outcome {
                    let _ = write!(head, " → outcome `{o}`");
                }
                if x.state == StepState::Failed {
                    let _ = write!(head, " → failed: {}", x.error.as_deref().unwrap_or("?"));
                }
                let _ = writeln!(s, "{head}");
                if let Some(summary) = x
                    .result
                    .as_ref()
                    .and_then(|r| r.get("summary"))
                    .and_then(Value::as_str)
                {
                    for line in summary.lines() {
                        let _ = writeln!(s, "  {line}");
                    }
                }
                if let Some(t) = &x.task_id {
                    for a in crate::artifacts::list_for_task(self.state, t).await? {
                        let _ = writeln!(s, "  artifact {} `{}` ({} bytes)", a.id, a.name, a.size);
                    }
                }
            }
        }
        s.push_str("\nWhen you finish, report `completed` with a result object:\n");
        let outcome = if node.outcomes.is_empty() {
            String::new()
        } else {
            format!(
                ", \"outcome\": {}",
                node.outcomes
                    .iter()
                    .map(|o| format!("\"{o}\""))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        };
        let _ = writeln!(
            s,
            "  {{\"summary\": \"…\"{outcome}, \"data\": {{…}}}}\n\
             `summary` is required, at most {} characters: what you did and what the next step needs.",
            def::SUMMARY_MAX_CHARS
        );
        if !node.outcomes.is_empty() {
            let _ = writeln!(
                s,
                "`outcome` is required: one of {}. It decides the next step.",
                node.outcomes
                    .iter()
                    .map(|o| format!("`{o}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let _ = writeln!(
            s,
            "`data` is optional JSON, at most {} KB.\n\
             Attach long output to this job as an artifact: MCP tower_artifact_put \
             {{\"task_id\": \"{task}\", \"name\": …, \"content\": …}}, or \
             `tower artifact put {task} <name> --file <path>`. Read an earlier step's \
             artifact with tower_artifact_get {{\"artifact_id\": …}} or `tower artifact get <id>`.\n\
             Report `failed` with a reason if you cannot do it.",
            def::DATA_MAX_BYTES / 1024
        );
        Ok(s)
    }

    /// A running step's job reached a terminal state.
    async fn finish(&mut self, step: &WorkflowStep, job: &Task) -> anyhow::Result<()> {
        match job.state {
            TaskState::Completed => {
                let (outcome, result) = split_result(job.result.as_ref());
                let res = sqlx::query(
                    "UPDATE workflow_steps SET state = 'completed', outcome = ?1, result = ?2,
                            ended_at = ?3
                     WHERE id = ?4 AND state = 'running'",
                )
                .bind(&outcome)
                .bind(result.to_string())
                .bind(self.now)
                .bind(step.id.0.as_str())
                .execute(&self.state.pool)
                .await?;
                if res.rows_affected() == 1 {
                    let summary = result.get("summary").cloned();
                    self.step_event(
                        EventKind::StepCompleted,
                        step,
                        json!({ "outcome": outcome, "summary": summary }),
                    )
                    .await?;
                    self.queue.push_back(Action::Closed(step.id.clone()));
                }
                Ok(())
            }
            s if s.is_terminal() => self.fail(step, job_error(job)).await,
            _ => Ok(()),
        }
    }

    /// An open step fails: route its `failed` edges (or fail the run).
    async fn fail(&mut self, step: &WorkflowStep, error: String) -> anyhow::Result<()> {
        let res = sqlx::query(
            "UPDATE workflow_steps SET state = 'failed', error = ?1, ended_at = ?2
             WHERE id = ?3 AND state IN ('pending','ready','running')",
        )
        .bind(&error)
        .bind(self.now)
        .bind(step.id.0.as_str())
        .execute(&self.state.pool)
        .await?;
        if res.rows_affected() == 1 {
            self.step_event(EventKind::StepFailed, step, json!({ "error": error }))
                .await?;
            self.queue.push_back(Action::Closed(step.id.clone()));
        }
        Ok(())
    }

    /// Fire the closed step's edges; an unhandled failure fails the run.
    async fn route(&mut self, id: &StepId) -> anyhow::Result<()> {
        let step = self.step(id).await?;
        let completed = step.state == StepState::Completed;
        let fired: Vec<String> = self
            .def
            .outgoing(&step.node_id)
            .filter(|e| e.fires(completed, step.outcome.as_deref()))
            .map(|e| e.to.clone())
            .collect();
        if !completed && fired.is_empty() {
            let error = format!(
                "step `{}` (visit {}) failed: {}",
                step.node_id,
                step.visit,
                step.error.as_deref().unwrap_or("?")
            );
            return self.end(RunState::Failed, Some(error)).await;
        }
        for to in fired {
            self.queue.push_back(Action::Activate {
                node: to,
                from: Some(step.id.clone()),
            });
        }
        Ok(())
    }

    /// Nothing `ready`/`running`: start pending joins with what fired, or
    /// complete the run. Returns true when it queued or started work.
    async fn settle(&mut self) -> anyhow::Result<bool> {
        if self.run.state != RunState::Running {
            return Ok(false);
        }
        let steps = self.steps().await?;
        if steps
            .iter()
            .any(|s| matches!(s.state, StepState::Ready | StepState::Running))
        {
            return Ok(false);
        }
        let pending: Vec<WorkflowStep> = steps
            .into_iter()
            .filter(|s| s.state == StepState::Pending)
            .collect();
        if pending.is_empty() {
            self.end(RunState::Completed, None).await?;
            return Ok(false);
        }
        for p in pending {
            if let Some(node) = self.def.node(&p.node_id).cloned() {
                self.promote(&node, p).await?;
            }
        }
        Ok(true)
    }

    /// End the run. Failed or canceled: open steps are canceled with their
    /// jobs (owners interrupted).
    async fn end(&mut self, to: RunState, error: Option<String>) -> anyhow::Result<()> {
        let result = match (&to, &self.def.result) {
            (RunState::Completed, Some(node)) => self.latest_completed(node).await?.map(|s| {
                let mut r = s.result.unwrap_or_else(|| json!({}));
                r["node"] = json!(s.node_id);
                r["outcome"] = json!(s.outcome);
                r
            }),
            _ => None,
        };
        let res = sqlx::query(
            "UPDATE workflow_runs SET state = ?1, result = ?2, error = ?3, ended_at = ?4
             WHERE id = ?5 AND state = 'running'",
        )
        .bind(to.as_str())
        .bind(result.as_ref().map(Value::to_string))
        .bind(&error)
        .bind(self.now)
        .bind(self.run.id.0.as_str())
        .execute(&self.state.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(());
        }
        self.run.state = to;
        if to != RunState::Completed {
            for s in self
                .steps()
                .await?
                .into_iter()
                .filter(|s| s.state.is_open())
            {
                let res = sqlx::query(
                    "UPDATE workflow_steps SET state = 'canceled', ended_at = ?1
                     WHERE id = ?2 AND state IN ('pending','ready','running')",
                )
                .bind(self.now)
                .bind(s.id.0.as_str())
                .execute(&self.state.pool)
                .await?;
                if res.rows_affected() == 0 {
                    continue;
                }
                self.step_event(EventKind::StepCanceled, &s, json!({}))
                    .await?;
                if s.state == StepState::Running {
                    if let Some(t) = &s.task_id {
                        if let Err(e) = crate::tasks::cancel(self.state, t, self.now).await {
                            if e.downcast_ref::<TowerError>().is_none() {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }
        let kind = match to {
            RunState::Completed => EventKind::RunCompleted,
            RunState::Failed => EventKind::RunFailed,
            _ => EventKind::RunCanceled,
        };
        event(
            self.state,
            kind,
            "workflow_run",
            &self.run.id.0,
            json!({
                "run_id": self.run.id, "workflow_id": self.run.workflow_id,
                "workflow": self.run.workflow_name, "state": to.as_str(),
                "result": result, "error": error,
            }),
        )
        .await
    }

    async fn step_event(
        &self,
        kind: EventKind,
        step: &WorkflowStep,
        extra: Value,
    ) -> anyhow::Result<()> {
        let mut payload = json!({
            "run_id": self.run.id, "workflow": self.run.workflow_name, "step_id": step.id,
            "node": step.node_id, "visit": step.visit, "task_id": step.task_id,
        });
        if let (Value::Object(p), Value::Object(x)) = (&mut payload, extra) {
            p.extend(x);
        }
        event(self.state, kind, "workflow_run", &self.run.id.0, payload).await
    }
}

/// A completed job's `(outcome, {summary, data?})`.
fn split_result(result: Option<&Value>) -> (Option<String>, Value) {
    match result {
        Some(Value::Object(m)) => {
            let mut out = Map::new();
            for key in ["summary", "data"] {
                if let Some(v) = m.get(key) {
                    out.insert(key.into(), v.clone());
                }
            }
            let outcome = m.get("outcome").and_then(Value::as_str).map(str::to_string);
            (outcome, Value::Object(out))
        }
        Some(Value::String(s)) => (None, json!({ "summary": s })),
        _ => (None, json!({})),
    }
}

/// Why a job did not complete, for the step's `error`.
fn job_error(job: &Task) -> String {
    let detail = job.result.as_ref().and_then(|r| match r {
        Value::String(s) => Some(s.clone()),
        Value::Object(m) => m
            .get("error")
            .or_else(|| m.get("summary"))
            .or_else(|| m.get("reason"))
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            }),
        _ => None,
    });
    match detail {
        Some(d) => format!("job {}: {d}", enum_str(job.state)),
        None => format!("job {}", enum_str(job.state)),
    }
}

async fn event(
    state: &AppState,
    kind: EventKind,
    subject_type: &str,
    subject_id: &str,
    payload: Value,
) -> anyhow::Result<()> {
    state
        .events
        .append(kind, Some(subject_type), Some(subject_id), payload)
        .await?;
    Ok(())
}

// ---- rows ------------------------------------------------------------------

fn row_to_workflow(r: &sqlx::sqlite::SqliteRow) -> Workflow {
    Workflow {
        id: WorkflowId::from(r.get::<String, _>("id")),
        name: r.get("name"),
        description: r.get("description"),
        enabled: r.get::<i64, _>("enabled") != 0,
        current_version: r.get("current_version"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> WorkflowRun {
    let json = |col: &str| -> Option<Value> {
        r.get::<Option<String>, _>(col)
            .and_then(|s| serde_json::from_str(&s).ok())
    };
    WorkflowRun {
        id: RunId::from(r.get::<String, _>("id")),
        workflow_id: WorkflowId::from(r.get::<String, _>("workflow_id")),
        workflow_name: r.get("workflow_name"),
        version: r.get("version"),
        state: parse_enum(r.get("state")),
        input: json("input").unwrap_or_else(|| json!({})),
        trigger: json("trigger").unwrap_or_else(|| json!({})),
        result: json("result"),
        error: r.get("error"),
        created_at: r.get("created_at"),
        ended_at: r.get("ended_at"),
    }
}

fn row_to_step(r: &sqlx::sqlite::SqliteRow) -> WorkflowStep {
    WorkflowStep {
        id: StepId::from(r.get::<String, _>("id")),
        run_id: RunId::from(r.get::<String, _>("run_id")),
        node_id: r.get("node_id"),
        visit: r.get("visit"),
        state: parse_enum(r.get("state")),
        activated_by: serde_json::from_str(&r.get::<String, _>("activated_by")).unwrap_or_default(),
        task_id: r.get::<Option<String>, _>("task_id").map(TaskId::from),
        agent_id: r.get::<Option<String>, _>("agent_id").map(AgentId::from),
        outcome: r.get("outcome"),
        result: r
            .get::<Option<String>, _>("result")
            .and_then(|s| serde_json::from_str(&s).ok()),
        error: r.get("error"),
        created_at: r.get("created_at"),
        started_at: r.get("started_at"),
        ended_at: r.get("ended_at"),
    }
}

// ---- routes ------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ApplyBody {
    pub source: String,
}

#[derive(Deserialize, Default)]
pub struct RunBody {
    #[serde(default)]
    pub input: Map<String, Value>,
}

async fn apply_route(State(s): State<AppState>, Json(b): Json<ApplyBody>) -> Response {
    match apply(&s, &b.source, tower_core::now_ms()).await {
        Ok(a) => {
            let status = if a.changed && a.version == 1 {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(json!(a))).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn list_route(State(s): State<AppState>) -> Response {
    match list(&s).await {
        Ok(v) => Json(json!({ "workflows": v })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn show_route(State(s): State<AppState>, Path(name): Path<String>) -> Response {
    match show(&s, &name).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

async fn remove_route(State(s): State<AppState>, Path(name): Path<String>) -> Response {
    match remove(&s, &name, tower_core::now_ms()).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn enable_route(State(s): State<AppState>, Path(name): Path<String>) -> Response {
    workflow_json(set_enabled(&s, &name, true, tower_core::now_ms()).await)
}

async fn disable_route(State(s): State<AppState>, Path(name): Path<String>) -> Response {
    workflow_json(set_enabled(&s, &name, false, tower_core::now_ms()).await)
}

fn workflow_json(r: anyhow::Result<Workflow>) -> Response {
    match r {
        Ok(w) => Json(json!({ "workflow": w })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn run_route(
    State(s): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<RunBody>>,
) -> Response {
    let input = body.map(|Json(b)| b.input).unwrap_or_default();
    let trigger = json!({ "kind": "manual", "by": "me" });
    match start_run(&s, &name, input, trigger, tower_core::now_ms()).await {
        Ok(run) => (StatusCode::CREATED, Json(json!({ "run": run }))).into_response(),
        Err(e) => error_response(e),
    }
}

async fn runs_route(State(s): State<AppState>, Query(f): Query<RunFilter>) -> Response {
    match list_runs(&s, &f).await {
        Ok(runs) => Json(json!({ "runs": runs })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn run_show_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match show_run(&s, &RunId::from(id)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

async fn cancel_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match cancel_run(&s, &RunId::from(id), tower_core::now_ms()).await {
        Ok(run) => Json(json!({ "run": run })).into_response(),
        Err(e) => error_response(e),
    }
}
