//! Recurring schedules (D§5.2.2, plan 2b T2.1-T2.2; decision:
//! scheduled-jobs — its policy table is normative).
//!
//! A schedule is a job template + cron expression + IANA timezone. The
//! sweeper calls `fire_due(now)`; each firing CAS-advances `next_run_at`
//! (exactly one firing per occurrence even across restarts or racing
//! sweeps) and materializes an ordinary job through
//! `tasks::create_occurrence` (unique `(schedule_id, occurrence_at)`).
//!
//! Policies on firing:
//! - previous occurrence being worked (owned) → **skip** (`schedule.skipped`)
//! - previous occurrence never delivered (queued) → **expire** it, create new
//! - missed firings (server down / asleep) → **coalesce** into one firing
//!   for the most recent missed time (`missed: N` in `schedule.fired`)
//! - target removed → **pause** + clear target (`on_target_removed`)
//!
//! Every fn takes `now` (ms) so tests inject the clock.

use std::str::FromStr;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use serde::Deserialize;
use sqlx::Row;
use tower_core::{AgentId, EventKind, Schedule, ScheduleId, TaskState, TowerError};

use crate::http::error_response;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/schedules", get(list_route).post(create_route))
        .route("/v1/schedules/{id}", get(show_route).delete(remove_route))
        .route("/v1/schedules/{id}/pause", post(pause_route))
        .route("/v1/schedules/{id}/resume", post(resume_route))
        .route("/v1/schedules/{id}/run", post(run_route))
}

/// Cap on counting missed occurrences after downtime (an every-second cron
/// over a week would otherwise loop ~600k times just to report a number).
const MISSED_COUNT_CAP: i64 = 10_000;

#[derive(Deserialize)]
pub struct CreateSchedule {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub lease_s: Option<i64>,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    /// Agent (name or id) the jobs are reserved for; none → general queue.
    #[serde(default)]
    pub target: Option<String>,
    /// Cron expression (5 or 6 fields, croner syntax)…
    #[serde(default)]
    pub cron: Option<String>,
    /// …or `HH:MM` for a daily schedule (sugar for `M H * * *`).
    #[serde(default)]
    pub daily: Option<String>,
    /// IANA zone; defaults to the machine's zone, fixed at creation.
    #[serde(default)]
    pub timezone: Option<String>,
}

/// What one firing did.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Fired {
    /// A job was created (and possibly an undelivered predecessor expired).
    Created {
        task_id: tower_core::TaskId,
        expired: Option<tower_core::TaskId>,
    },
    /// The previous occurrence is still being worked.
    Skipped { running: tower_core::TaskId },
    /// This occurrence already had a job (duplicate firing, exactly-once).
    Duplicate,
}

// ---- time ----------------------------------------------------------------

fn parse_cron(expr: &str) -> anyhow::Result<Cron> {
    Cron::from_str(expr).map_err(|e| TowerError::invalid(format!("cron {expr:?}: {e}")).into())
}

fn parse_tz(name: &str) -> anyhow::Result<Tz> {
    Tz::from_str(name).map_err(|_| {
        TowerError::invalid(format!(
            "unknown timezone {name:?} (IANA name, e.g. America/Los_Angeles)"
        ))
        .into()
    })
}

/// `HH:MM` → `M H * * *`.
pub fn daily_cron(hhmm: &str) -> anyhow::Result<String> {
    let (h, m) = hhmm
        .split_once(':')
        .and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)))
        .filter(|(h, m)| *h < 24 && *m < 60)
        .ok_or_else(|| TowerError::invalid(format!("--daily {hhmm:?}: use HH:MM (00:00-23:59)")))?;
    Ok(format!("{m} {h} * * *"))
}

/// The machine's IANA zone, or UTC when it can't be determined/parsed.
pub fn system_timezone() -> String {
    iana_time_zone::get_timezone()
        .ok()
        .filter(|z| Tz::from_str(z).is_ok())
        .unwrap_or_else(|| "UTC".into())
}

fn to_zoned(ms: i64, tz: Tz) -> chrono::DateTime<Tz> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&tz)
}

/// First occurrence strictly after `ms`.
fn next_after(cron: &Cron, tz: Tz, ms: i64) -> anyhow::Result<i64> {
    let t = cron
        .find_next_occurrence(&to_zoned(ms, tz), false)
        .map_err(|e| TowerError::invalid(format!("cron has no next occurrence: {e}")))?;
    Ok(t.timestamp_millis())
}

/// Latest occurrence at or before `ms`.
fn latest_at_or_before(cron: &Cron, tz: Tz, ms: i64) -> anyhow::Result<i64> {
    let t = cron
        .find_previous_occurrence(&to_zoned(ms, tz), true)
        .map_err(|e| TowerError::invalid(format!("cron has no previous occurrence: {e}")))?;
    Ok(t.timestamp_millis())
}

/// Occurrences in `[from, until)` — the ones a coalesced firing replaces.
fn count_between(cron: &Cron, tz: Tz, from: i64, until: i64) -> i64 {
    let mut n = 0;
    let mut t = to_zoned(from, tz);
    while n < MISSED_COUNT_CAP {
        if t.timestamp_millis() >= until {
            break;
        }
        n += 1;
        match cron.find_next_occurrence(&t, false) {
            Ok(next) => t = next,
            Err(_) => break,
        }
    }
    n
}

// ---- service -------------------------------------------------------------

pub async fn create(state: &AppState, req: CreateSchedule, now: i64) -> anyhow::Result<Schedule> {
    if req.title.trim().is_empty() {
        return Err(TowerError::invalid("title must not be empty").into());
    }
    let cron = match (&req.cron, &req.daily) {
        (Some(c), None) => c.trim().to_string(),
        (None, Some(d)) => daily_cron(d)?,
        _ => return Err(TowerError::invalid("give exactly one of `cron` or `daily`").into()),
    };
    let parsed = parse_cron(&cron)?;
    let timezone = req.timezone.clone().unwrap_or_else(system_timezone);
    let tz = parse_tz(&timezone)?;
    let lease_s = req.lease_s.unwrap_or(crate::tasks::DEFAULT_LEASE_S);
    let max_attempts = req.max_attempts.unwrap_or(3);
    if lease_s <= 0 || max_attempts < 1 {
        return Err(TowerError::invalid("lease_s must be > 0 and max_attempts >= 1").into());
    }
    let target = match &req.target {
        Some(t) => Some(
            crate::inventory::get_agent(state, t)
                .await?
                .ok_or_else(|| TowerError::not_found(format!("agent {t} not found")))?
                .id,
        ),
        None => None,
    };
    let next = next_after(&parsed, tz, now)?;
    let id = ScheduleId::new();
    sqlx::query(
        "INSERT INTO schedules (id, title, description, tags, priority, lease_s, max_attempts,
                                target_agent_id, cron, timezone, enabled, next_run_at,
                                created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?12)",
    )
    .bind(id.0.as_str())
    .bind(&req.title)
    .bind(&req.description)
    .bind(serde_json::to_string(&req.tags)?)
    .bind(req.priority.unwrap_or(0))
    .bind(lease_s)
    .bind(max_attempts)
    .bind(target.as_ref().map(|a| a.0.as_str()))
    .bind(&cron)
    .bind(&timezone)
    .bind(next)
    .bind(now)
    .execute(&state.pool)
    .await?;
    let s = require(state, &id).await?;
    event(
        state,
        EventKind::ScheduleCreated,
        &id,
        serde_json::json!({
            "schedule_id": id, "title": s.title, "cron": s.cron, "timezone": s.timezone,
            "target": s.target_agent_id, "next_run_at": next,
        }),
    )
    .await?;
    Ok(s)
}

/// Sweep step: fire every enabled schedule whose `next_run_at` has passed.
pub async fn fire_due(state: &AppState, now: i64) -> anyhow::Result<usize> {
    let due: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
    )
    .bind(now)
    .fetch_all(&state.pool)
    .await?;
    let mut fired = 0;
    for id in due {
        match fire_one(state, &ScheduleId::from(id.clone()), now).await {
            Ok(true) => fired += 1,
            Ok(false) => {}
            // one bad schedule must not stop the others
            Err(e) => tracing::warn!(error = %e, schedule = %id, "schedule firing failed"),
        }
    }
    Ok(fired)
}

/// Fire one due schedule: claim the firing by CAS on `next_run_at`, then
/// apply the occurrence. Returns false when another sweep claimed it.
async fn fire_one(state: &AppState, id: &ScheduleId, now: i64) -> anyhow::Result<bool> {
    let Some(s) = get_schedule(state, id).await? else {
        return Ok(false);
    };
    let Some(scheduled) = s.next_run_at.filter(|n| *n <= now && s.enabled) else {
        return Ok(false);
    };
    let cron = parse_cron(&s.cron)?;
    let tz = parse_tz(&s.timezone)?;
    // coalesce: one firing, for the most recent due time
    let occurrence = latest_at_or_before(&cron, tz, now)?.max(scheduled);
    let missed = count_between(&cron, tz, scheduled, occurrence);
    let next = next_after(&cron, tz, now)?;

    let claimed = sqlx::query(
        "UPDATE schedules SET next_run_at = ?1, last_run_at = ?2, updated_at = ?3
         WHERE id = ?4 AND enabled = 1 AND next_run_at = ?5",
    )
    .bind(next)
    .bind(occurrence)
    .bind(now)
    .bind(id.0.as_str())
    .bind(scheduled)
    .execute(&state.pool)
    .await?;
    if claimed.rows_affected() == 0 {
        return Ok(false);
    }
    apply_occurrence(state, &s, occurrence, missed, false, now).await?;
    Ok(true)
}

/// The policy table, for one occurrence.
async fn apply_occurrence(
    state: &AppState,
    s: &Schedule,
    occurrence: i64,
    missed: i64,
    manual: bool,
    now: i64,
) -> anyhow::Result<Fired> {
    let mut expired = None;
    if let Some(prev) = crate::tasks::open_occurrence(state, &s.id).await? {
        if prev.state == TaskState::Queued && prev.owner_id.is_none() {
            if crate::tasks::expire_occurrence(state, &prev.id, now).await? {
                expired = Some(prev.id);
            }
        } else {
            // being worked (or delivered in the meantime): skip this one
            event(
                state,
                EventKind::ScheduleSkipped,
                &s.id,
                serde_json::json!({
                    "schedule_id": s.id, "occurrence_at": occurrence,
                    "reason": "previous_running", "running": prev.id, "manual": manual,
                }),
            )
            .await?;
            return Ok(Fired::Skipped { running: prev.id });
        }
    }
    let Some(task) = crate::tasks::create_occurrence(state, s, occurrence, now).await? else {
        return Ok(Fired::Duplicate);
    };
    event(
        state,
        EventKind::ScheduleFired,
        &s.id,
        serde_json::json!({
            "schedule_id": s.id, "task_id": task.id, "occurrence_at": occurrence,
            "missed": missed, "manual": manual, "expired": expired,
        }),
    )
    .await?;
    Ok(Fired::Created {
        task_id: task.id,
        expired,
    })
}

/// One extra occurrence now (same overlap/expiry rules; cadence untouched).
pub async fn run_now(state: &AppState, id: &ScheduleId, now: i64) -> anyhow::Result<Fired> {
    let s = require(state, id).await?;
    // whole seconds, like cron occurrences
    let occurrence = now - now.rem_euclid(1000);
    let fired = apply_occurrence(state, &s, occurrence, 0, true, now).await?;
    sqlx::query("UPDATE schedules SET last_run_at = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(occurrence)
        .bind(now)
        .bind(id.0.as_str())
        .execute(&state.pool)
        .await?;
    Ok(fired)
}

pub async fn pause(
    state: &AppState,
    id: &ScheduleId,
    reason: &str,
    now: i64,
) -> anyhow::Result<Schedule> {
    let s = require(state, id).await?;
    if s.enabled {
        sqlx::query("UPDATE schedules SET enabled = 0, updated_at = ?1 WHERE id = ?2")
            .bind(now)
            .bind(id.0.as_str())
            .execute(&state.pool)
            .await?;
        event(
            state,
            EventKind::SchedulePaused,
            id,
            serde_json::json!({"schedule_id": id, "reason": reason}),
        )
        .await?;
    }
    require(state, id).await
}

/// Resume from now: no catch-up for the paused span.
pub async fn resume(state: &AppState, id: &ScheduleId, now: i64) -> anyhow::Result<Schedule> {
    let s = require(state, id).await?;
    if !s.enabled {
        let next = next_after(&parse_cron(&s.cron)?, parse_tz(&s.timezone)?, now)?;
        sqlx::query(
            "UPDATE schedules SET enabled = 1, next_run_at = ?1, updated_at = ?2 WHERE id = ?3",
        )
        .bind(next)
        .bind(now)
        .bind(id.0.as_str())
        .execute(&state.pool)
        .await?;
        event(
            state,
            EventKind::ScheduleResumed,
            id,
            serde_json::json!({"schedule_id": id, "next_run_at": next}),
        )
        .await?;
    }
    require(state, id).await
}

/// Delete the template; jobs it created keep their `schedule_id` (history)
/// and open ones are left alone.
pub async fn remove(state: &AppState, id: &ScheduleId) -> anyhow::Result<()> {
    let res = sqlx::query("DELETE FROM schedules WHERE id = ?1")
        .bind(id.0.as_str())
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(TowerError::not_found(format!("schedule {id} not found")).into());
    }
    event(
        state,
        EventKind::ScheduleRemoved,
        id,
        serde_json::json!({"schedule_id": id}),
    )
    .await
}

/// Target agent removed (decision table): pause its schedules and clear
/// the target (resuming then feeds the general queue).
pub async fn on_target_removed(state: &AppState, agent: &AgentId, now: i64) -> anyhow::Result<()> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM schedules WHERE target_agent_id = ?1")
            .bind(agent.0.as_str())
            .fetch_all(&state.pool)
            .await?;
    for id in ids {
        let id = ScheduleId::from(id);
        sqlx::query("UPDATE schedules SET enabled = 0, target_agent_id = NULL, updated_at = ?1 WHERE id = ?2")
            .bind(now)
            .bind(id.0.as_str())
            .execute(&state.pool)
            .await?;
        event(
            state,
            EventKind::SchedulePaused,
            &id,
            serde_json::json!({"schedule_id": id, "reason": "target_removed", "target": agent}),
        )
        .await?;
    }
    Ok(())
}

pub async fn list(state: &AppState) -> anyhow::Result<Vec<Schedule>> {
    let rows = sqlx::query("SELECT * FROM schedules ORDER BY created_at")
        .fetch_all(&state.pool)
        .await?;
    Ok(rows.iter().map(row_to_schedule).collect())
}

pub async fn get_schedule(state: &AppState, id: &ScheduleId) -> anyhow::Result<Option<Schedule>> {
    let row = sqlx::query("SELECT * FROM schedules WHERE id = ?1")
        .bind(id.0.as_str())
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.as_ref().map(row_to_schedule))
}

async fn require(state: &AppState, id: &ScheduleId) -> anyhow::Result<Schedule> {
    get_schedule(state, id)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("schedule {id} not found")).into())
}

fn row_to_schedule(r: &sqlx::sqlite::SqliteRow) -> Schedule {
    let tags: String = r.get("tags");
    Schedule {
        id: ScheduleId::from(r.get::<String, _>("id")),
        title: r.get("title"),
        description: r.get("description"),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        priority: r.get("priority"),
        lease_s: r.get("lease_s"),
        max_attempts: r.get("max_attempts"),
        target_agent_id: r
            .get::<Option<String>, _>("target_agent_id")
            .map(AgentId::from),
        cron: r.get("cron"),
        timezone: r.get("timezone"),
        enabled: r.get::<i64, _>("enabled") != 0,
        next_run_at: r.get("next_run_at"),
        last_run_at: r.get("last_run_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

async fn event(
    state: &AppState,
    kind: EventKind,
    id: &ScheduleId,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    state
        .events
        .append(kind, Some("schedule"), Some(id.0.as_str()), payload)
        .await?;
    Ok(())
}

// ---- routes --------------------------------------------------------------

fn schedule_json(r: anyhow::Result<Schedule>) -> Response {
    match r {
        Ok(s) => Json(serde_json::json!({ "schedule": s })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn create_route(State(s): State<AppState>, Json(b): Json<CreateSchedule>) -> Response {
    match create(&s, b, tower_core::now_ms()).await {
        Ok(sc) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "schedule": sc })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

async fn list_route(State(s): State<AppState>) -> Response {
    match list(&s).await {
        Ok(v) => Json(serde_json::json!({ "schedules": v })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn show_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let id = ScheduleId::from(id);
    let res = async {
        let sc = require(&s, &id).await?;
        let rows = sqlx::query(
            "SELECT * FROM tasks WHERE schedule_id = ?1 ORDER BY occurrence_at DESC LIMIT 20",
        )
        .bind(id.0.as_str())
        .fetch_all(&s.pool)
        .await?;
        let jobs: Vec<tower_core::Task> = rows.iter().map(crate::tasks::row_to_task).collect();
        anyhow::Ok(serde_json::json!({ "schedule": sc, "jobs": jobs }))
    };
    match res.await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

async fn pause_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    schedule_json(pause(&s, &ScheduleId::from(id), "operator", tower_core::now_ms()).await)
}

async fn resume_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    schedule_json(resume(&s, &ScheduleId::from(id), tower_core::now_ms()).await)
}

async fn run_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match run_now(&s, &ScheduleId::from(id), tower_core::now_ms()).await {
        Ok(f) => Json(serde_json::json!({ "fired": f })).into_response(),
        Err(e) => error_response(e),
    }
}

async fn remove_route(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match remove(&s, &ScheduleId::from(id)).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => error_response(e),
    }
}
