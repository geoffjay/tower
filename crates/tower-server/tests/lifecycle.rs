//! Job lifecycle prompts and editing (phase 2d, D§5.2.3; decision
//! job-lifecycle-hooks): pre-run before the delegation, post-run after the
//! owner's close with the agent busy until it settles, failure/timeout and
//! restart handling, and the PATCH rules for jobs and schedules.

mod common;

use std::future::Future;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use tower_core::{HookPhase, Task, TaskId, TaskState};
use tower_driver::HarnessState;
use tower_server::tasks::{self, CreateTask};
use tower_server::Config;

const T0: i64 = 1_000_000;

fn job(title: &str) -> CreateTask {
    CreateTask {
        title: title.into(),
        description: None,
        priority: None,
        tags: vec![],
        assign: None,
        not_before: None,
        lease_s: None,
        max_attempts: None,
        pre_run: None,
        post_run: None,
    }
}

async fn wait_for<F, Fut>(what: &str, mut done: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !done().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn task(ctx: &common::Ctx, id: &TaskId) -> Task {
    tasks::get_task(&ctx.state, id).await.unwrap().unwrap()
}

fn texts(ctx: &common::Ctx) -> Vec<String> {
    ctx.harness.prompts().into_iter().map(|p| p.1).collect()
}

/// `task.hook` statuses on the job's trail, as `phase:status`.
async fn hook_trail(ctx: &common::Ctx, id: &TaskId) -> Vec<String> {
    tasks::trail(&ctx.state, id)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind.as_str() == "task.hook")
        .map(|e| {
            format!(
                "{}:{}",
                e.payload["phase"].as_str().unwrap(),
                e.payload["status"].as_str().unwrap()
            )
        })
        .collect()
}

/// Let a sent lifecycle prompt settle: wait until the runner reports it
/// running (and saw the agent take it up), then the agent goes idle.
async fn settle_after_running(ctx: &common::Ctx, id: &TaskId, phase: &str, agent: &str) {
    let running = format!("{phase}:running");
    wait_for("the prompt to run", || async {
        hook_trail(ctx, id).await.contains(&running)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    ctx.harness.set_state(agent, HarnessState::Idle);
}

// ---- pre-run ---------------------------------------------------------------

#[tokio::test]
async fn pre_run_goes_first_and_the_delegation_waits_for_the_agent_to_settle() {
    let ctx = common::boot().await;
    ctx.spawn("a", "claude").await;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            pre_run: Some("  /clear ".into()),
            ..job("refactor the parser")
        },
        T0,
    )
    .await
    .unwrap();
    assert_eq!(j.state, TaskState::Assigned);
    assert_eq!(j.hook, Some(HookPhase::Pre));
    assert_eq!(j.pre_run.as_deref(), Some("/clear"), "stored trimmed");

    wait_for("the pre-run prompt", || async { texts(&ctx).len() == 1 }).await;
    assert_eq!(
        texts(&ctx),
        ["/clear"],
        "typed verbatim, before anything else"
    );

    // still compacting/clearing: no delegation, and the lease does not run out
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(texts(&ctx).len(), 1, "the job prompt waits for the agent");
    assert_eq!(
        tasks::sweep_leases(&ctx.state, T0 + 3_600_000)
            .await
            .unwrap(),
        0,
        "lease expiry pauses during pre-run"
    );
    assert_eq!(task(&ctx, &j.id).await.state, TaskState::Assigned);

    settle_after_running(&ctx, &j.id, "pre", "a").await;
    wait_for("the delegation", || async { texts(&ctx).len() == 2 }).await;
    assert!(
        texts(&ctx)[1].starts_with(&format!("Job {} is assigned to you", j.id)),
        "{:?}",
        texts(&ctx)
    );
    let t = task(&ctx, &j.id).await;
    assert_eq!(t.hook, None);
    assert!(
        t.lease_expires_at.unwrap() > tower_core::now_ms(),
        "the full lease starts with the delegation"
    );
    assert_eq!(hook_trail(&ctx, &j.id).await, ["pre:running", "pre:done"]);

    // the prompt is on the record, linked to the job, before the delegation
    let (_, v) = ctx.req("GET", &format!("/v1/tasks/{}", j.id), None).await;
    let kinds: Vec<(&str, &str)> = v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["kind"].as_str().unwrap(),
                m["parts"][0]["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(kinds[0], ("prompt", "/clear"));
    assert_eq!(kinds[1].0, "delegation");
}

#[tokio::test]
async fn cancel_during_pre_run_sends_no_delegation() {
    let ctx = common::boot().await;
    ctx.spawn("a", "claude").await;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            pre_run: Some("/compact".into()),
            ..job("x")
        },
        T0,
    )
    .await
    .unwrap();
    wait_for("the pre-run prompt", || async { texts(&ctx).len() == 1 }).await;

    let t = tasks::cancel(&ctx.state, &j.id, T0 + 1).await.unwrap();
    assert_eq!(t.state, TaskState::Canceled);
    assert_eq!(t.hook, None, "cancel takes the delivery away");
    settle_after_running(&ctx, &j.id, "pre", "a").await;
    wait_for("the runner to finish", || async {
        hook_trail(&ctx, &j.id).await.len() == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        texts(&ctx),
        ["/compact"],
        "no job prompt for a canceled job"
    );
}

#[tokio::test]
async fn a_prompt_that_never_settles_is_recorded_and_the_job_still_goes_out() {
    let mut config = Config::default();
    config.jobs.hook_timeout_s = 1;
    let ctx = common::boot_with_config(config).await;
    ctx.spawn("a", "claude").await;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            pre_run: Some("/compact".into()),
            ..job("x")
        },
        T0,
    )
    .await
    .unwrap();
    // the agent never goes idle again
    wait_for("the delegation after the timeout", || async {
        texts(&ctx).len() == 2
    })
    .await;
    assert_eq!(texts(&ctx)[0], "/compact");
    let trail = tasks::trail(&ctx.state, &j.id).await.unwrap();
    let failed = trail
        .iter()
        .find(|e| e.kind.as_str() == "task.hook" && e.payload["status"] == "failed")
        .expect("failure recorded");
    assert!(
        failed.payload["error"]
            .as_str()
            .unwrap()
            .contains("did not settle"),
        "{}",
        failed.payload
    );
    assert_eq!(task(&ctx, &j.id).await.hook, None);
}

// ---- post-run --------------------------------------------------------------

#[tokio::test]
async fn post_run_holds_the_next_reserved_job_until_it_settles() {
    let ctx = common::boot().await;
    ctx.spawn("a", "claude").await;
    let j1 = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            post_run: Some("/reflect".into()),
            ..job("first")
        },
        T0,
    )
    .await
    .unwrap();
    let j2 = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            ..job("next")
        },
        T0,
    )
    .await
    .unwrap();
    assert_eq!(j2.state, TaskState::Queued, "busy with j1");

    let closed = tasks::report(
        &ctx.state,
        &j1.id,
        "a",
        Some(TaskState::Completed),
        Some(json!({"summary": "done"})),
        T0 + 10,
    )
    .await
    .unwrap();
    assert_eq!(closed.hook, Some(HookPhase::Post));
    assert_eq!(
        task(&ctx, &j2.id).await.state,
        TaskState::Queued,
        "post-run first"
    );
    let t = tasks::assign(&ctx.state, &j2.id, "a", None, "me", T0 + 11)
        .await
        .unwrap();
    assert_eq!(
        t.state,
        TaskState::Queued,
        "still waits for the post-run prompt"
    );
    let w = tasks::waiting(&ctx.state, &t).await.unwrap().unwrap();
    assert!(w.reason.contains("post-run"), "{}", w.reason);

    // the agent finishes its closing turn, then gets the post-run prompt
    ctx.harness.set_state("a", HarnessState::Idle);
    wait_for("the post-run prompt", || async { texts(&ctx).len() == 2 }).await;
    assert_eq!(texts(&ctx)[1], "/reflect");
    assert_eq!(task(&ctx, &j2.id).await.state, TaskState::Queued);

    settle_after_running(&ctx, &j1.id, "post", "a").await;
    // the assignment is stored before its delegation prompt goes out: wait
    // for both
    wait_for("the next job", || async {
        task(&ctx, &j2.id).await.state == TaskState::Assigned && texts(&ctx).len() == 3
    })
    .await;
    let prompts = texts(&ctx);
    assert_eq!(prompts.len(), 3);
    assert!(prompts[2].starts_with(&format!("Job {} is assigned", j2.id)));
    assert_eq!(task(&ctx, &j1.id).await.hook, None);
    assert_eq!(
        hook_trail(&ctx, &j1.id).await,
        ["post:running", "post:done"]
    );
}

#[tokio::test]
async fn cancel_release_and_lease_expiry_never_run_post_run() {
    let ctx = common::boot().await;
    let mut ids = Vec::new();
    for agent in ["a", "b", "c"] {
        ctx.spawn(agent, "claude").await;
        let j = tasks::create(
            &ctx.state,
            CreateTask {
                assign: Some(agent.into()),
                post_run: Some("/reflect".into()),
                lease_s: Some(10),
                ..job(agent)
            },
            T0,
        )
        .await
        .unwrap();
        ids.push(j.id);
    }
    tasks::cancel(&ctx.state, &ids[0], T0 + 1).await.unwrap();
    tasks::release(&ctx.state, &ids[1], "b", None, T0 + 1)
        .await
        .unwrap();
    assert!(tasks::expire_lease(&ctx.state, &ids[2], T0 + 60_000)
        .await
        .unwrap());
    for agent in ["a", "b", "c"] {
        ctx.harness.set_state(agent, HarnessState::Idle);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    for id in &ids {
        assert_eq!(task(&ctx, id).await.hook, None);
    }
    assert!(
        !texts(&ctx).iter().any(|t| t == "/reflect"),
        "{:?}",
        texts(&ctx)
    );
}

// ---- restart ---------------------------------------------------------------

#[tokio::test]
async fn boot_clears_marks_a_crash_left_behind() {
    let ctx = common::boot().await;
    ctx.spawn("a", "claude").await;
    ctx.spawn("b", "claude").await;
    let pre = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            ..job("waiting on pre-run")
        },
        T0,
    )
    .await
    .unwrap();
    let post = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("b".into()),
            ..job("closed, post-run running")
        },
        T0,
    )
    .await
    .unwrap();
    tasks::report(
        &ctx.state,
        &post.id,
        "b",
        Some(TaskState::Completed),
        None,
        T0,
    )
    .await
    .unwrap();
    // what the runners had marked when the server died
    for (id, hook) in [(&pre.id, "pre"), (&post.id, "post")] {
        sqlx::query("UPDATE tasks SET hook = ?1 WHERE id = ?2")
            .bind(hook)
            .bind(id.0.as_str())
            .execute(&ctx.state.pool)
            .await
            .unwrap();
    }
    let next = tasks::create(&ctx.state, job("for b"), T0).await.unwrap();
    let held = tasks::assign(&ctx.state, &next.id, "b", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(
        held.state,
        TaskState::Queued,
        "b is held by the stale post-run mark"
    );
    assert_eq!(
        tasks::sweep_leases(&ctx.state, T0 + 3_600_000)
            .await
            .unwrap(),
        0
    );

    assert_eq!(tower_server::hooks::recover(&ctx.state).await.unwrap(), 2);
    assert_eq!(hook_trail(&ctx, &pre.id).await, ["pre:interrupted"]);
    assert_eq!(hook_trail(&ctx, &post.id).await, ["post:interrupted"]);
    // the lease recovers the undelivered job; b is free again
    assert_eq!(
        tasks::sweep_leases(&ctx.state, T0 + 3_600_000)
            .await
            .unwrap(),
        1
    );
    assert_eq!(task(&ctx, &pre.id).await.state, TaskState::Queued);
    let t = tasks::assign(&ctx.state, &next.id, "b", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(t.state, TaskState::Assigned);
}

// ---- editing jobs ------------------------------------------------------------

#[tokio::test]
async fn a_queued_job_takes_every_field_and_null_or_empty_clears() {
    let ctx = common::boot().await;
    let (s, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(json!({"title": "audit", "description": "old", "pre_run": "/clear"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = v["task"]["id"].as_str().unwrap().to_string();

    let (s, v) = ctx
        .req(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"title": "dependency audit", "description": null,
                        "pre_run": "", "post_run": "/reflect"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let t = &v["task"];
    assert_eq!(t["title"], "dependency audit");
    assert!(t["description"].is_null(), "null clears");
    assert!(t["pre_run"].is_null(), "\"\" clears");
    assert_eq!(t["post_run"], "/reflect");

    let trail = tasks::trail(&ctx.state, &TaskId::from(id.clone()))
        .await
        .unwrap();
    let edited = trail
        .iter()
        .find(|e| e.kind.as_str() == "task.updated")
        .unwrap();
    assert_eq!(
        edited.payload["fields"],
        json!(["title", "description", "pre_run", "post_run"])
    );

    // an absent field is left alone
    let (_, v) = ctx
        .req(
            "PATCH",
            &format!("/v1/tasks/{id}"),
            Some(json!({"description": "new"})),
        )
        .await;
    assert_eq!(v["task"]["post_run"], "/reflect");

    for (body, want) in [
        (json!({}), StatusCode::BAD_REQUEST),
        (json!({"title": " "}), StatusCode::BAD_REQUEST),
        (
            json!({"not_before": T0}),
            StatusCode::BAD_REQUEST, // no reservation to hold back
        ),
    ] {
        let (s, v) = ctx
            .req("PATCH", &format!("/v1/tasks/{id}"), Some(body.clone()))
            .await;
        assert_eq!(s, want, "{body} → {v}");
    }
}

#[tokio::test]
async fn a_new_time_on_a_reserved_job_reruns_delivery() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let far = tower_core::now_ms() + 3_600_000;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            not_before: Some(far),
            ..job("tonight")
        },
        T0,
    )
    .await
    .unwrap();
    assert_eq!(j.state, TaskState::Queued);
    let (s, v) = ctx
        .req(
            "PATCH",
            &format!("/v1/tasks/{}", j.id),
            Some(json!({"not_before": null})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["task"]["state"], "assigned", "due now: delivered at once");
}

#[tokio::test]
async fn a_delivered_job_takes_only_post_run_and_a_closed_one_nothing() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            ..job("x")
        },
        T0,
    )
    .await
    .unwrap();
    let path = format!("/v1/tasks/{}", j.id);
    let patch = |body: serde_json::Value| ctx.req("PATCH", &path, Some(body));

    let (s, v) = patch(json!({"description": "too late"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("post_run"));
    let (s, v) = patch(json!({"post_run": "/learn"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["task"]["post_run"], "/learn");

    tasks::report(&ctx.state, &j.id, "a", Some(TaskState::Failed), None, T0)
        .await
        .unwrap();
    let (s, _) = patch(json!({"post_run": "/reflect"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

// ---- editing schedules -------------------------------------------------------

#[tokio::test]
async fn a_schedule_edit_applies_to_later_firings_only() {
    let ctx = common::boot().await;
    let (s, v) = ctx
        .req(
            "POST",
            "/v1/schedules",
            Some(
                json!({"title": "audit", "daily": "09:00", "timezone": "UTC",
                        "pre_run": "/compact", "post_run": "/learn"}),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let sid = v["schedule"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["schedule"]["pre_run"], "/compact");

    let (_, v) = ctx
        .req("POST", &format!("/v1/schedules/{sid}/run"), None)
        .await;
    let first = TaskId::from(v["fired"]["task_id"].as_str().unwrap());
    let fired = task(&ctx, &first).await;
    assert_eq!(
        fired.pre_run.as_deref(),
        Some("/compact"),
        "copied at firing"
    );
    assert_eq!(fired.post_run.as_deref(), Some("/learn"));

    let (s, v) = ctx
        .req(
            "PATCH",
            &format!("/v1/schedules/{sid}"),
            Some(json!({"daily": "10:30", "pre_run": "/clear", "post_run": null})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let sc = &v["schedule"];
    assert_eq!(sc["cron"], "30 10 * * *");
    assert_eq!(sc["pre_run"], "/clear");
    assert!(sc["post_run"].is_null());
    // next run moved to the new cadence: 10:30 UTC, after now
    let next = sc["next_run_at"].as_i64().unwrap();
    assert!(next > tower_core::now_ms());
    assert_eq!((next / 1000) % 86_400, 10 * 3600 + 30 * 60);

    assert_eq!(
        task(&ctx, &first).await.pre_run.as_deref(),
        Some("/compact"),
        "an open occurrence keeps what it was fired with"
    );
    // a later occurrence (run-now keys occurrences by the second)
    let fired = tower_server::schedules::run_now(
        &ctx.state,
        &tower_core::ScheduleId::from(sid.clone()),
        tower_core::now_ms() + 2_000,
    )
    .await
    .unwrap();
    let tower_server::schedules::Fired::Created { task_id, .. } = fired else {
        panic!("expected a new occurrence, got {fired:?}");
    };
    let second = task(&ctx, &task_id).await;
    assert_eq!(second.pre_run.as_deref(), Some("/clear"));
    assert_eq!(second.post_run, None);

    for body in [
        json!({"cron": "not a cron"}),
        json!({"cron": "0 9 * * *", "daily": "09:00"}),
        json!({"timezone": "Mars/Olympus"}),
        json!({}),
    ] {
        let (s, v) = ctx
            .req("PATCH", &format!("/v1/schedules/{sid}"), Some(body.clone()))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} → {v}");
    }
}

#[tokio::test]
async fn mcp_edits_are_operator_only() {
    let ctx = common::boot().await;
    ctx.spawn("a", "claude").await;
    let r = ctx
        .tool(
            None,
            "tower_task_create",
            json!({"title": "x", "pre_run": "/clear"}),
        )
        .await;
    let id = r["structuredContent"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(r["structuredContent"]["task"]["pre_run"], "/clear");

    let r = ctx
        .tool(
            Some("a"),
            "tower_task_edit",
            json!({"task_id": id, "pre_run": null}),
        )
        .await;
    assert_eq!(r["isError"], true, "agents may not edit jobs: {r}");
    let r = ctx
        .tool(
            None,
            "tower_task_edit",
            json!({"task_id": id, "pre_run": null}),
        )
        .await;
    assert!(r["structuredContent"]["task"]["pre_run"].is_null(), "{r}");

    let r = ctx
        .tool(
            None,
            "tower_schedule_create",
            json!({"title": "s", "daily": "09:00", "post_run": "/learn"}),
        )
        .await;
    let sid = r["structuredContent"]["schedule"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = ctx
        .tool(
            Some("a"),
            "tower_schedule_edit",
            json!({"schedule_id": sid, "post_run": ""}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let r = ctx
        .tool(
            None,
            "tower_schedule_edit",
            json!({"schedule_id": sid, "post_run": ""}),
        )
        .await;
    assert!(
        r["structuredContent"]["schedule"]["post_run"].is_null(),
        "{r}"
    );
}
