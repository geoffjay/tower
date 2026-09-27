//! Job queue tests (plan T3.1-T3.3 verify, D§5.2.1): assignment CAS and
//! exclusivity, owner-only writes, lease sweeper branches, routes + trail.

mod common;

use axum::http::StatusCode;
use tower_core::{AgentState, ErrorCode, TaskId, TaskState, TowerError};
use tower_server::tasks::{self, CreateTask, ListFilter};

fn job(title: &str) -> CreateTask {
    CreateTask {
        title: title.into(),
        description: None,
        priority: None,
        tags: vec![],
        assign: None,
        when_available: false,
        not_before: None,
        lease_s: None,
        max_attempts: None,
    }
}

fn code(e: &anyhow::Error) -> ErrorCode {
    e.downcast_ref::<TowerError>().expect("TowerError").code
}

const T0: i64 = 1_000_000;

// ---- T3.1: assignment CAS + exclusivity --------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_assigns_have_exactly_one_winner() {
    let ctx = common::boot().await;
    let agents = ["a0", "a1", "a2", "a3"];
    for a in agents {
        ctx.spawn(a, "pi").await;
    }
    let task = tasks::create(&ctx.state, job("contended"), T0)
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..32 {
        let state = ctx.state.clone();
        let id = task.id.clone();
        let to = agents[i % agents.len()];
        handles.push(tokio::spawn(async move {
            tasks::assign(&state, &id, to, None, "me", T0).await
        }));
    }
    let mut winners = Vec::new();
    let mut conflicts = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(t) => winners.push(t),
            Err(e) => {
                assert_eq!(code(&e), ErrorCode::Conflict, "{e}");
                conflicts += 1;
            }
        }
    }
    assert_eq!(winners.len(), 1, "exactly one winner");
    assert_eq!(conflicts, 31);

    let owner = winners[0].owner_id.clone().unwrap();
    let row = tasks::get_task(&ctx.state, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, TaskState::Assigned);
    assert_eq!(row.owner_id, Some(owner));
    assert_eq!(ctx.event_count("task.assigned").await, 1);
    // only the winner was notified
    assert_eq!(ctx.harness.prompts().len(), 1);
}

#[tokio::test]
async fn live_owned_job_is_never_reassigned() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", None, "me", T0)
        .await
        .unwrap();

    // lease still live (60s default) → conflict naming the owner
    let e = tasks::assign(&ctx.state, &t.id, "b", None, "me", T0 + 59_000)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
    // same agent again is also a conflict (no silent re-lease)
    let e = tasks::assign(&ctx.state, &t.id, "a", None, "me", T0 + 1)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

#[tokio::test]
async fn assign_after_expiry_sweeps_then_reassigns_with_attempt_bump() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", Some(10), "me", T0)
        .await
        .unwrap();

    let t2 = tasks::assign(&ctx.state, &t.id, "b", None, "me", T0 + 10_001)
        .await
        .unwrap();
    let b = tower_server::inventory::get_agent(&ctx.state, "b")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t2.owner_id, Some(b.id));
    assert_eq!(t2.attempt_count, 1);
    assert_eq!(ctx.event_count("task.leased_out").await, 1);
    // the prior owner lost ownership: its writes now conflict
    let e = tasks::heartbeat(&ctx.state, &t.id, "a", T0 + 10_002)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

#[tokio::test]
async fn assign_validates_agent_and_task() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    let e = tasks::assign(&ctx.state, &t.id, "ghost", None, "me", T0)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let e = tasks::assign(&ctx.state, &TaskId::from("t_nope"), "a", None, "me", T0)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);

    tower_server::sessions::stop(&ctx.state, "a", false)
        .await
        .unwrap();
    let e = tasks::assign(&ctx.state, &t.id, "a", None, "me", T0)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Invalid, "dead agents can't take work");

    // terminal jobs can't be assigned
    ctx.spawn("b", "pi").await;
    tasks::cancel(&ctx.state, &t.id, T0).await.unwrap();
    let e = tasks::assign(&ctx.state, &t.id, "b", None, "me", T0)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

#[tokio::test]
async fn queue_orders_by_priority_then_age_and_filters_tags() {
    let ctx = common::boot().await;
    let mk = |title: &str, priority: i64, tags: &[&str]| CreateTask {
        priority: Some(priority),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..job(title)
    };
    tasks::create(&ctx.state, mk("old-low", 0, &["rust"]), T0)
        .await
        .unwrap();
    tasks::create(&ctx.state, mk("high", 5, &["writing"]), T0 + 1)
        .await
        .unwrap();
    tasks::create(&ctx.state, mk("new-low", 0, &["rust", "backend"]), T0 + 2)
        .await
        .unwrap();

    let all = tasks::list(&ctx.state, &ListFilter::default())
        .await
        .unwrap();
    let titles: Vec<_> = all.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["high", "old-low", "new-low"]);

    let f = ListFilter {
        tags: Some("rust,backend".into()),
        ..Default::default()
    };
    let tagged = tasks::list(&ctx.state, &f).await.unwrap();
    assert_eq!(tagged.len(), 1, "must carry every requested tag");
    assert_eq!(tagged[0].title, "new-low");
}

#[tokio::test]
async fn mine_lists_only_open_owned_jobs() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let open = tasks::create(&ctx.state, job("open"), T0).await.unwrap();
    let done = tasks::create(&ctx.state, job("done"), T0).await.unwrap();
    tasks::create(&ctx.state, job("unowned"), T0).await.unwrap();
    // one job at a time: finish `done` before `open` can be assigned
    tasks::assign(&ctx.state, &done.id, "a", None, "me", T0)
        .await
        .unwrap();
    tasks::report(
        &ctx.state,
        &done.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0,
    )
    .await
    .unwrap();
    tasks::assign(&ctx.state, &open.id, "a", None, "me", T0)
        .await
        .unwrap();

    let f = ListFilter {
        mine: Some("a".into()),
        ..Default::default()
    };
    let mine = tasks::list(&ctx.state, &f).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, open.id);
}

#[tokio::test]
async fn no_claim_or_pull_endpoint_exists() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    for path in ["claim", "pull"] {
        let (status, _) = ctx
            .req(
                "POST",
                &format!("/v1/tasks/{}/{path}", t.id),
                Some(serde_json::json!({"as": "a"})),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "/{path} must not exist");
    }
    let row = tasks::get_task(&ctx.state, &t.id).await.unwrap().unwrap();
    assert_eq!(row.state, TaskState::Queued);
    assert!(row.owner_id.is_none());
}

// ---- owner-only writes -------------------------------------------------

#[tokio::test]
async fn owner_lifecycle_start_heartbeat_complete() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", Some(60), "me", T0)
        .await
        .unwrap();

    let s = tasks::start(&ctx.state, &t.id, "a", T0 + 1_000)
        .await
        .unwrap();
    assert_eq!(s.state, TaskState::Working);
    assert_eq!(s.lease_expires_at, Some(T0 + 61_000));

    let h = tasks::heartbeat(&ctx.state, &t.id, "a", T0 + 30_000)
        .await
        .unwrap();
    assert_eq!(h.lease_expires_at, Some(T0 + 90_000), "renews by lease_s");

    let done = tasks::report(
        &ctx.state,
        &t.id,
        "a",
        Some(TaskState::Completed),
        Some(serde_json::json!({"summary": "ok"})),
        T0 + 40_000,
    )
    .await
    .unwrap();
    assert_eq!(done.state, TaskState::Completed);
    assert_eq!(done.lease_expires_at, None);
    assert_eq!(done.result, Some(serde_json::json!({"summary": "ok"})));
    assert_eq!(ctx.event_count("task.completed").await, 1);

    // closed: no further owner writes
    let e = tasks::heartbeat(&ctx.state, &t.id, "a", T0 + 41_000)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

#[tokio::test]
async fn non_owner_writes_conflict() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();

    // unassigned: nobody may start it
    let e = tasks::start(&ctx.state, &t.id, "a", T0).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);

    tasks::assign(&ctx.state, &t.id, "a", None, "me", T0)
        .await
        .unwrap();
    let st = &ctx.state;
    let id = &t.id;
    for e in [
        tasks::start(st, id, "b", T0).await.unwrap_err(),
        tasks::heartbeat(st, id, "b", T0).await.unwrap_err(),
        tasks::report(st, id, "b", Some(TaskState::Completed), None, T0)
            .await
            .unwrap_err(),
        tasks::release(st, id, "b", None, T0).await.unwrap_err(),
    ] {
        assert_eq!(code(&e), ErrorCode::Conflict, "{e}");
    }
    let row = tasks::get_task(st, id).await.unwrap().unwrap();
    assert_eq!(row.state, TaskState::Assigned, "untouched by non-owner");
}

#[tokio::test]
async fn owners_cannot_report_operator_states() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", None, "me", T0)
        .await
        .unwrap();
    for s in [TaskState::Queued, TaskState::Canceled, TaskState::Assigned] {
        let e = tasks::report(&ctx.state, &t.id, "a", Some(s), None, T0)
            .await
            .unwrap_err();
        assert_eq!(code(&e), ErrorCode::Invalid);
    }
}

#[tokio::test]
async fn release_returns_job_to_queue_without_attempt_bump() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", None, "me", T0)
        .await
        .unwrap();
    let r = tasks::release(&ctx.state, &t.id, "a", Some("wrong skills".into()), T0)
        .await
        .unwrap();
    assert_eq!(r.state, TaskState::Queued);
    assert!(r.owner_id.is_none());
    assert_eq!(r.attempt_count, 0);
    tasks::assign(&ctx.state, &t.id, "b", None, "me", T0)
        .await
        .unwrap();
}

#[tokio::test]
async fn late_but_unswept_owner_can_still_complete() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", Some(10), "me", T0)
        .await
        .unwrap();
    // lease lapsed in wall time, but no sweep ran and nobody else took it
    let done = tasks::report(
        &ctx.state,
        &t.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0 + 60_000,
    )
    .await
    .unwrap();
    assert_eq!(done.state, TaskState::Completed);
}

#[tokio::test]
async fn cancel_interrupts_nothing_when_unowned_and_closes_the_job() {
    let ctx = common::boot().await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    let c = tasks::cancel(&ctx.state, &t.id, T0).await.unwrap();
    assert_eq!(c.state, TaskState::Canceled);
    let e = tasks::cancel(&ctx.state, &t.id, T0).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

// ---- T3.2: lease sweeper ------------------------------------------------

#[tokio::test]
async fn sweeper_requeues_expired_lease() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", Some(10), "me", T0)
        .await
        .unwrap();

    // boundary: at exactly the deadline the lease is still live
    assert_eq!(
        tasks::sweep_leases(&ctx.state, T0 + 10_000).await.unwrap(),
        0
    );
    assert_eq!(
        tasks::sweep_leases(&ctx.state, T0 + 10_001).await.unwrap(),
        1
    );

    let row = tasks::get_task(&ctx.state, &t.id).await.unwrap().unwrap();
    assert_eq!(row.state, TaskState::Queued);
    assert!(row.owner_id.is_none() && row.lease_expires_at.is_none());
    assert_eq!(row.attempt_count, 1);
    assert_eq!(ctx.event_count("task.leased_out").await, 1);
}

#[tokio::test]
async fn sweeper_fails_job_when_attempts_exhausted() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    let t = tasks::create(
        &ctx.state,
        CreateTask {
            max_attempts: Some(2),
            lease_s: Some(10),
            ..job("flaky")
        },
        T0,
    )
    .await
    .unwrap();
    let mut now = T0;
    for _ in 0..2 {
        tasks::assign(&ctx.state, &t.id, "a", None, "me", now)
            .await
            .unwrap();
        now += 10_001;
        assert_eq!(tasks::sweep_leases(&ctx.state, now).await.unwrap(), 1);
    }
    let row = tasks::get_task(&ctx.state, &t.id).await.unwrap().unwrap();
    assert_eq!(row.state, TaskState::Failed);
    assert_eq!(row.attempt_count, 2);
    assert_eq!(
        row.result,
        Some(serde_json::json!({"error": "lease_exhausted"}))
    );
    assert_eq!(ctx.event_count("task.failed").await, 1);
    // exhausted jobs are never reassigned
    let e = tasks::assign(&ctx.state, &t.id, "a", None, "me", now)
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
}

#[tokio::test]
async fn input_required_pauses_lease_expiry() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let t = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::assign(&ctx.state, &t.id, "a", Some(10), "me", T0)
        .await
        .unwrap();
    tasks::start(&ctx.state, &t.id, "a", T0).await.unwrap();

    // agent blocks on a human → job paused
    let a = tower_server::inventory::get_agent(&ctx.state, "a")
        .await
        .unwrap()
        .unwrap();
    tasks::sync_agent_state(&ctx.state, &a.id, AgentState::Blocked, T0 + 1)
        .await
        .unwrap();
    let far = T0 + 3_600_000;
    assert_eq!(tasks::sweep_leases(&ctx.state, far).await.unwrap(), 0);
    let e = tasks::assign(&ctx.state, &t.id, "b", None, "me", far)
        .await
        .unwrap_err();
    assert_eq!(
        code(&e),
        ErrorCode::Conflict,
        "paused lease is never stolen"
    );

    // unblocked → working with a fresh lease from `now`
    tasks::sync_agent_state(&ctx.state, &a.id, AgentState::Working, far)
        .await
        .unwrap();
    let row = tasks::get_task(&ctx.state, &t.id).await.unwrap().unwrap();
    assert_eq!(row.state, TaskState::Working);
    assert_eq!(row.lease_expires_at, Some(far + 10_000));
}

// ---- T3.3: routes, trail, notice ----------------------------------------

#[tokio::test]
async fn routes_drive_the_work_loop_with_trail() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let (s, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "csv errors", "tags": ["rust"], "priority": 2})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["task"]["state"], "queued");
    let id = v["task"]["id"].as_str().unwrap().to_string();

    let (s, v) = ctx
        .req(
            "POST",
            &format!("/v1/tasks/{id}/assign"),
            Some(serde_json::json!({"to": "backend"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["task"]["state"], "assigned");

    // the owner was notified with a delegation prompt naming the job
    let prompts = ctx.harness.prompts();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].1.contains(&id) && prompts[0].1.contains("csv errors"));

    for (path, body) in [
        ("start", serde_json::json!({"as": "backend"})),
        ("heartbeat", serde_json::json!({"as": "backend"})),
        (
            "status",
            serde_json::json!({"as": "backend", "state": "completed", "result": {"ok": true}}),
        ),
    ] {
        let (s, v) = ctx
            .req("POST", &format!("/v1/tasks/{id}/{path}"), Some(body))
            .await;
        assert_eq!(s, StatusCode::OK, "{path}: {v}");
    }

    let (s, v) = ctx.req("GET", &format!("/v1/tasks/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["task"]["state"], "completed");
    let kinds: Vec<&str> = v["trail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "task.created",
            "task.assigned",
            "task.status",
            "task.status",
            "task.completed"
        ]
    );
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "delegation");
}

#[tokio::test]
async fn route_errors_use_the_envelope() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let (_, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "x", "assign": "a"})),
        )
        .await;
    assert_eq!(v["task"]["state"], "assigned", "pre-assign on create");
    let id = v["task"]["id"].as_str().unwrap().to_string();

    let (s, v) = ctx
        .req(
            "POST",
            &format!("/v1/tasks/{id}/assign"),
            Some(serde_json::json!({"to": "b"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"]["code"], "conflict");
    assert!(v["error"]["detail"]["owner_id"].is_string());

    let (s, v) = ctx
        .req(
            "POST",
            &format!("/v1/tasks/{id}/start"),
            Some(serde_json::json!({"as": "b"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    let (s, _) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "  "})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = ctx.req("GET", "/v1/tasks/t_missing", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// ---- agent removal ------------------------------------------------------

#[tokio::test]
async fn removing_an_agent_requeues_open_jobs_and_keeps_history() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let open = tasks::create(&ctx.state, job("open"), T0).await.unwrap();
    let done = tasks::create(&ctx.state, job("done"), T0).await.unwrap();
    // one job at a time: `done` first, then `open` stays in progress
    tasks::assign(&ctx.state, &done.id, "a", None, "me", T0)
        .await
        .unwrap();
    tasks::report(
        &ctx.state,
        &done.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0,
    )
    .await
    .unwrap();
    tasks::assign(&ctx.state, &open.id, "a", None, "me", T0)
        .await
        .unwrap();
    tasks::start(&ctx.state, &open.id, "a", T0).await.unwrap();
    ctx.harness.kill("a"); // pane already gone: removal must still work

    tower_server::sessions::stop(&ctx.state, "a", true)
        .await
        .unwrap();

    assert!(tower_server::inventory::get_agent(&ctx.state, "a")
        .await
        .unwrap()
        .is_none());
    let o = tasks::get_task(&ctx.state, &open.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(o.state, TaskState::Queued);
    assert!(o.owner_id.is_none());
    assert_eq!(
        o.attempt_count, 0,
        "operator removal is not a failed attempt"
    );
    let d = tasks::get_task(&ctx.state, &done.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(d.state, TaskState::Completed);
    assert!(
        tasks::trail(&ctx.state, &done.id).await.unwrap().len() >= 3,
        "history kept"
    );

    tasks::assign(&ctx.state, &open.id, "b", None, "me", T0)
        .await
        .unwrap();
}
