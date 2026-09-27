//! Recurring schedules (phase 2b M2): every row of the decision's policy
//! table, exactly-once firing, DST, operator-only. Clock-injected.

mod common;

use chrono::TimeZone;
use chrono_tz::America::Los_Angeles as LA;
use tower_core::{ErrorCode, ScheduleId, TaskState, TowerError};
use tower_server::schedules::{self, CreateSchedule};
use tower_server::tasks;

fn la(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    LA.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .earliest()
        .unwrap()
        .timestamp_millis()
}

/// Local wall clock of a ms instant, as "MM-DD HH:MM".
fn wall(ms: i64) -> String {
    chrono::Utc
        .timestamp_millis_opt(ms)
        .unwrap()
        .with_timezone(&LA)
        .format("%m-%d %H:%M")
        .to_string()
}

fn daily(title: &str, hhmm: &str, target: Option<&str>) -> CreateSchedule {
    CreateSchedule {
        title: title.into(),
        description: None,
        tags: vec![],
        priority: None,
        lease_s: None,
        max_attempts: None,
        target: target.map(str::to_string),
        cron: None,
        daily: Some(hhmm.into()),
        timezone: Some("America/Los_Angeles".into()),
    }
}

async fn jobs_of(ctx: &common::Ctx, id: &ScheduleId) -> Vec<tower_core::Task> {
    tasks::list(&ctx.state, &Default::default())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.schedule_id.as_ref() == Some(id))
        .collect()
}

fn code(e: &anyhow::Error) -> ErrorCode {
    e.downcast_ref::<TowerError>().expect("TowerError").code
}

const DAY: i64 = 86_400_000;

#[tokio::test]
async fn daily_fires_once_per_occurrence_and_delivers_to_an_idle_target() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    let t0 = la(2026, 9, 28, 8, 0);
    let s = schedules::create(&ctx.state, daily("audit", "09:00", Some("backend")), t0)
        .await
        .unwrap();
    assert_eq!(wall(s.next_run_at.unwrap()), "09-28 09:00");

    assert_eq!(
        schedules::fire_due(&ctx.state, la(2026, 9, 28, 8, 59))
            .await
            .unwrap(),
        0
    );
    let at = la(2026, 9, 28, 9, 0);
    assert_eq!(schedules::fire_due(&ctx.state, at).await.unwrap(), 1);
    assert_eq!(
        schedules::fire_due(&ctx.state, at + 5_000).await.unwrap(),
        0,
        "no refire"
    );

    let jobs = jobs_of(&ctx, &s.id).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].state,
        TaskState::Assigned,
        "idle target: delivered at firing"
    );
    assert_eq!(jobs[0].occurrence_at, Some(at));
    let s = schedules::get_schedule(&ctx.state, &s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wall(s.next_run_at.unwrap()), "09-29 09:00");
    let trail = tasks::trail(&ctx.state, &jobs[0].id).await.unwrap();
    let assigned = trail
        .iter()
        .find(|e| e.kind.as_str() == "task.assigned")
        .unwrap();
    assert_eq!(assigned.payload["by"], format!("schedule:{}", s.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn racing_sweeps_fire_exactly_once() {
    let ctx = common::boot().await;
    let t0 = la(2026, 9, 28, 8, 0);
    let s = schedules::create(&ctx.state, daily("race", "09:00", None), t0)
        .await
        .unwrap();
    let at = la(2026, 9, 28, 9, 0);
    let mut hs = Vec::new();
    for _ in 0..16 {
        let st = ctx.state.clone();
        hs.push(tokio::spawn(async move {
            schedules::fire_due(&st, at).await.unwrap()
        }));
    }
    let mut total = 0;
    for h in hs {
        total += h.await.unwrap();
    }
    assert_eq!(total, 1, "the next_run_at CAS admits one firing");
    assert_eq!(jobs_of(&ctx, &s.id).await.len(), 1);
    assert_eq!(ctx.event_count("schedule.fired").await, 1);
}

#[tokio::test]
async fn missed_firings_coalesce_into_one() {
    let ctx = common::boot().await;
    let t0 = la(2026, 9, 28, 8, 0);
    let s = schedules::create(&ctx.state, daily("audit", "09:00", None), t0)
        .await
        .unwrap();
    // server "down" from 09-28 08:00 until 10-01 12:00 (4 firings due)
    let back = la(2026, 10, 1, 12, 0);
    assert_eq!(schedules::fire_due(&ctx.state, back).await.unwrap(), 1);

    let jobs = jobs_of(&ctx, &s.id).await;
    assert_eq!(jobs.len(), 1, "one job, not four");
    assert_eq!(
        wall(jobs[0].occurrence_at.unwrap()),
        "10-01 09:00",
        "most recent due time"
    );
    let s2 = schedules::get_schedule(&ctx.state, &s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wall(s2.next_run_at.unwrap()), "10-02 09:00");
    let fired: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM events WHERE type = 'schedule.fired'")
            .fetch_one(&ctx.state.pool)
            .await
            .map(|p: String| serde_json::from_str(&p).unwrap())
            .unwrap();
    assert_eq!(fired["missed"], 3, "09-28, 09-29, 09-30 folded into 10-01");
}

#[tokio::test]
async fn skips_while_the_previous_run_is_being_worked() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    let t0 = la(2026, 9, 28, 8, 0);
    let s = schedules::create(&ctx.state, daily("audit", "09:00", Some("backend")), t0)
        .await
        .unwrap();
    schedules::fire_due(&ctx.state, la(2026, 9, 28, 9, 0))
        .await
        .unwrap();
    let first = jobs_of(&ctx, &s.id).await.remove(0);
    tasks::start(&ctx.state, &first.id, "backend", la(2026, 9, 28, 9, 1))
        .await
        .unwrap();
    // keep its lease alive across the next firing
    tasks::heartbeat(&ctx.state, &first.id, "backend", la(2026, 9, 29, 8, 59))
        .await
        .unwrap();

    schedules::fire_due(&ctx.state, la(2026, 9, 29, 9, 0))
        .await
        .unwrap();
    assert_eq!(jobs_of(&ctx, &s.id).await.len(), 1, "skipped, not stacked");
    assert_eq!(ctx.event_count("schedule.skipped").await, 1);
    let s2 = schedules::get_schedule(&ctx.state, &s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wall(s2.next_run_at.unwrap()),
        "09-30 09:00",
        "cadence continues"
    );
}

#[tokio::test]
async fn undelivered_occurrence_is_replaced_by_the_next() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    // backend is busy with something else all day
    let other = tasks::create(
        &ctx.state,
        tasks::CreateTask {
            title: "long".into(),
            description: None,
            priority: None,
            tags: vec![],
            assign: Some("backend".into()),
            when_available: false,
            not_before: None,
            lease_s: Some(10 * 86_400),
            max_attempts: None,
        },
        la(2026, 9, 28, 7, 0),
    )
    .await
    .unwrap();
    assert_eq!(other.state, TaskState::Assigned);

    let s = schedules::create(
        &ctx.state,
        daily("audit", "09:00", Some("backend")),
        la(2026, 9, 28, 8, 0),
    )
    .await
    .unwrap();
    schedules::fire_due(&ctx.state, la(2026, 9, 28, 9, 0))
        .await
        .unwrap();
    let first = jobs_of(&ctx, &s.id).await.remove(0);
    assert_eq!(first.state, TaskState::Queued, "busy target: waits");

    schedules::fire_due(&ctx.state, la(2026, 9, 29, 9, 0))
        .await
        .unwrap();
    let jobs = jobs_of(&ctx, &s.id).await;
    let old = jobs.iter().find(|t| t.id == first.id).unwrap();
    assert_eq!(old.state, TaskState::Canceled);
    assert_eq!(
        old.result,
        Some(serde_json::json!({"error": "occurrence_expired"}))
    );
    let pending: Vec<_> = jobs
        .iter()
        .filter(|t| t.state == TaskState::Queued)
        .collect();
    assert_eq!(pending.len(), 1, "at most one pending occurrence");
    assert_eq!(wall(pending[0].occurrence_at.unwrap()), "09-29 09:00");
}

#[tokio::test]
async fn untargeted_schedule_feeds_the_general_queue() {
    let ctx = common::boot().await;
    let s = schedules::create(
        &ctx.state,
        daily("triage", "09:00", None),
        la(2026, 9, 28, 8, 0),
    )
    .await
    .unwrap();
    schedules::fire_due(&ctx.state, la(2026, 9, 28, 9, 0))
        .await
        .unwrap();
    let j = jobs_of(&ctx, &s.id).await.remove(0);
    assert_eq!(j.state, TaskState::Queued);
    assert!(j.target_agent_id.is_none());
    assert_eq!(j.origin, "schedule");
}

#[tokio::test]
async fn removing_the_target_pauses_the_schedule() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    let s = schedules::create(
        &ctx.state,
        daily("audit", "09:00", Some("backend")),
        la(2026, 9, 28, 8, 0),
    )
    .await
    .unwrap();
    tower_server::sessions::stop(&ctx.state, "backend", true)
        .await
        .unwrap();

    let s2 = schedules::get_schedule(&ctx.state, &s.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!s2.enabled);
    assert!(s2.target_agent_id.is_none());
    assert_eq!(
        schedules::fire_due(&ctx.state, la(2026, 9, 30, 9, 0))
            .await
            .unwrap(),
        0
    );
    let reason: String = sqlx::query_scalar(
        "SELECT json_extract(payload, '$.reason') FROM events WHERE type = 'schedule.paused'",
    )
    .fetch_one(&ctx.state.pool)
    .await
    .unwrap();
    assert_eq!(reason, "target_removed");
}

#[tokio::test]
async fn pause_then_resume_does_not_catch_up() {
    let ctx = common::boot().await;
    let s = schedules::create(
        &ctx.state,
        daily("audit", "09:00", None),
        la(2026, 9, 28, 8, 0),
    )
    .await
    .unwrap();
    schedules::pause(&ctx.state, &s.id, "operator", la(2026, 9, 28, 8, 30))
        .await
        .unwrap();
    assert_eq!(
        schedules::fire_due(&ctx.state, la(2026, 9, 30, 12, 0))
            .await
            .unwrap(),
        0
    );

    let r = schedules::resume(&ctx.state, &s.id, la(2026, 9, 30, 12, 0))
        .await
        .unwrap();
    assert_eq!(
        wall(r.next_run_at.unwrap()),
        "10-01 09:00",
        "from now, no catch-up"
    );
    assert_eq!(
        schedules::fire_due(&ctx.state, la(2026, 9, 30, 12, 1))
            .await
            .unwrap(),
        0
    );
    assert!(jobs_of(&ctx, &s.id).await.is_empty());
}

#[tokio::test]
async fn run_now_adds_an_occurrence_without_moving_the_cadence() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    let s = schedules::create(
        &ctx.state,
        daily("audit", "09:00", Some("backend")),
        la(2026, 9, 28, 8, 0),
    )
    .await
    .unwrap();
    let now = la(2026, 9, 28, 8, 30);
    let f = schedules::run_now(&ctx.state, &s.id, now).await.unwrap();
    assert!(matches!(f, schedules::Fired::Created { .. }), "{f:?}");
    let s2 = schedules::get_schedule(&ctx.state, &s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s2.next_run_at, s.next_run_at, "cadence untouched");

    // the manual run is being worked → a second run-now is skipped
    let job = jobs_of(&ctx, &s.id).await.remove(0);
    assert_eq!(job.state, TaskState::Assigned);
    let f = schedules::run_now(&ctx.state, &s.id, now + 1_000)
        .await
        .unwrap();
    assert!(matches!(f, schedules::Fired::Skipped { .. }), "{f:?}");
}

/// America/Los_Angeles 2026: spring forward 03-08 02:00→03:00, fall back
/// 11-01 02:00→01:00 (croner rules, as documented in the decision).
#[tokio::test]
async fn dst_gap_runs_at_the_first_valid_time_and_overlap_runs_once() {
    let ctx = common::boot().await;
    let gap = schedules::create(
        &ctx.state,
        daily("gap", "02:30", None),
        la(2026, 3, 7, 12, 0),
    )
    .await
    .unwrap();
    let mut seen = Vec::new();
    let mut now;
    for _ in 0..3 {
        let s = schedules::get_schedule(&ctx.state, &gap.id)
            .await
            .unwrap()
            .unwrap();
        now = s.next_run_at.unwrap();
        schedules::fire_due(&ctx.state, now).await.unwrap();
        seen.push(wall(now));
    }
    assert_eq!(seen, ["03-08 03:00", "03-09 02:30", "03-10 02:30"]);

    let overlap = schedules::create(
        &ctx.state,
        daily("overlap", "01:30", None),
        la(2026, 10, 31, 12, 0),
    )
    .await
    .unwrap();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let s = schedules::get_schedule(&ctx.state, &overlap.id)
            .await
            .unwrap()
            .unwrap();
        now = s.next_run_at.unwrap();
        schedules::fire_due(&ctx.state, now).await.unwrap();
        seen.push(now);
    }
    assert_eq!(wall(seen[0]), "11-01 01:30");
    assert_eq!(
        wall(seen[1]),
        "11-02 01:30",
        "the repeated 01:30 fired once"
    );
    assert!(seen[1] - seen[0] > DAY, "25h day");
}

#[tokio::test]
async fn invalid_schedules_are_rejected() {
    let ctx = common::boot().await;
    for req in [
        CreateSchedule {
            cron: Some("not a cron".into()),
            daily: None,
            ..daily("x", "09:00", None)
        },
        CreateSchedule {
            timezone: Some("Mars/Olympus".into()),
            ..daily("x", "09:00", None)
        },
        daily("x", "25:00", None),
        CreateSchedule {
            cron: Some("0 9 * * *".into()),
            ..daily("both", "09:00", None)
        },
        daily("x", "09:00", Some("ghost")),
    ] {
        let e = schedules::create(&ctx.state, req, 0).await.unwrap_err();
        assert!(
            matches!(code(&e), ErrorCode::Invalid | ErrorCode::NotFound),
            "{e}"
        );
    }
}

#[tokio::test]
async fn schedules_are_operator_only_over_mcp_and_listed_over_rest() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "omp").await;
    let r = ctx
        .tool(
            Some("backend"),
            "tower_schedule_create",
            serde_json::json!({"title": "x", "daily": "09:00"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["error"]["code"], "unauthorized");

    let r = ctx
        .tool(
            None,
            "tower_schedule_create",
            serde_json::json!({"title": "x", "daily": "09:00", "target": "backend"}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let (_, v) = ctx.req("GET", "/v1/schedules", None).await;
    assert_eq!(v["schedules"].as_array().unwrap().len(), 1);
}
