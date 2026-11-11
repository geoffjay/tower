//! Reserved delivery + one job per agent (phase 2b M1, D§5.2.2; decision
//! scheduled-jobs). Clock-injected; triggers exercised directly.

mod common;

use axum::http::StatusCode;
use tower_core::{ErrorCode, TaskId, TaskState, TowerError};
use tower_driver::HarnessState;
use tower_server::tasks::{self, CreateTask};

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

fn code(e: &anyhow::Error) -> ErrorCode {
    e.downcast_ref::<TowerError>().expect("TowerError").code
}

async fn state_of(ctx: &common::Ctx, id: &TaskId) -> (TaskState, Option<String>) {
    let t = tasks::get_task(&ctx.state, id).await.unwrap().unwrap();
    (t.state, t.owner_id.map(|o| o.0))
}

async fn agent_id(ctx: &common::Ctx, name: &str) -> tower_core::AgentId {
    tower_server::inventory::get_agent(&ctx.state, name)
        .await
        .unwrap()
        .unwrap()
        .id
}

// ---- one job per agent ---------------------------------------------------

#[tokio::test]
async fn assigning_a_busy_agent_reserves_the_job_and_says_why() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let j1 = tasks::create(&ctx.state, job("first"), T0).await.unwrap();
    let j2 = tasks::create(&ctx.state, job("second"), T0).await.unwrap();
    tasks::assign(&ctx.state, &j1.id, "a", None, "me", T0)
        .await
        .unwrap();

    let t = tasks::assign(&ctx.state, &j2.id, "a", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(t.state, TaskState::Queued, "not delivered to a busy agent");
    assert_eq!(t.target_agent_id.unwrap(), agent_id(&ctx, "a").await);
    let t = tasks::get_task(&ctx.state, &j2.id).await.unwrap().unwrap();
    let w = tasks::waiting(&ctx.state, &t).await.unwrap().unwrap();
    assert_eq!(w.busy_with.unwrap(), j1.id);
    assert!(w.reason.contains("busy with job"), "{}", w.reason);

    tasks::report(
        &ctx.state,
        &j1.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0 + 1,
    )
    .await
    .unwrap();
    assert_eq!(state_of(&ctx, &j2.id).await.0, TaskState::Assigned);
}

#[tokio::test]
async fn assigning_an_agent_working_on_a_prompt_waits_until_it_settles() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    // a prompt, not a job: the agent owns nothing but its pane is busy
    ctx.harness.set_state("a", HarnessState::Working);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    let before = ctx.harness.prompts().len();

    let j = tasks::create(&ctx.state, job("next"), T0).await.unwrap();
    let t = tasks::assign(&ctx.state, &j.id, "a", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(t.state, TaskState::Queued);
    assert_eq!(
        ctx.harness.prompts().len(),
        before,
        "nothing typed mid-turn"
    );
    let w = tasks::waiting(&ctx.state, &t).await.unwrap().unwrap();
    assert_eq!(w.agent_state.as_str(), "working");
    assert!(w.busy_with.is_none());

    ctx.harness.set_state("a", HarnessState::Idle);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    assert_eq!(state_of(&ctx, &j.id).await.0, TaskState::Assigned);
    assert_eq!(ctx.harness.prompts().len(), before + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_assigns_deliver_one_job_and_reserve_the_rest() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let mut ids = Vec::new();
    for i in 0..16 {
        ids.push(
            tasks::create(&ctx.state, job(&format!("j{i}")), T0)
                .await
                .unwrap()
                .id,
        );
    }
    let mut handles = Vec::new();
    for id in ids {
        let state = ctx.state.clone();
        handles.push(tokio::spawn(async move {
            tasks::assign(&state, &id, "a", None, "me", T0).await
        }));
    }
    let (mut owned, mut reserved) = (0, 0);
    for h in handles {
        let t = h.await.unwrap().unwrap();
        let t = tasks::get_task(&ctx.state, &t.id).await.unwrap().unwrap();
        match t.state {
            TaskState::Assigned => owned += 1,
            TaskState::Queued if t.target_agent_id.is_some() => reserved += 1,
            s => panic!("unexpected {s:?}"),
        }
    }
    assert_eq!(owned, 1, "the free check is atomic with the owner CAS");
    assert_eq!(reserved, 15);
}

// ---- reserved delivery ---------------------------------------------------

#[tokio::test]
async fn reserved_job_waits_while_busy_and_is_delivered_when_the_job_closes() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let j1 = tasks::create(&ctx.state, job("current"), T0).await.unwrap();
    let j2 = tasks::create(&ctx.state, job("next"), T0).await.unwrap();
    tasks::assign(&ctx.state, &j1.id, "a", None, "me", T0)
        .await
        .unwrap();

    let r = tasks::reserve(&ctx.state, &j2.id, "a", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(r.state, TaskState::Queued, "busy: stays queued");
    assert_eq!(
        r.target_agent_id.as_ref().unwrap().0,
        agent_id(&ctx, "a").await.0
    );
    assert!(r.lease_expires_at.is_none(), "no lease until delivery");

    tasks::report(
        &ctx.state,
        &j1.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0 + 5,
    )
    .await
    .unwrap();
    let (st, owner) = state_of(&ctx, &j2.id).await;
    assert_eq!(st, TaskState::Assigned);
    assert_eq!(owner.unwrap(), agent_id(&ctx, "a").await.0);
    let t = tasks::get_task(&ctx.state, &j2.id).await.unwrap().unwrap();
    assert_eq!(
        t.lease_expires_at,
        Some(T0 + 5 + 60_000),
        "lease starts at delivery"
    );
    let trail = tasks::trail(&ctx.state, &j2.id).await.unwrap();
    let assigned = trail
        .iter()
        .find(|e| e.kind.as_str() == "task.assigned")
        .unwrap();
    assert_eq!(assigned.payload["by"], "dispatch");
}

#[tokio::test]
async fn working_agent_gets_its_reservation_on_the_idle_transition() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    ctx.harness.set_state("a", HarnessState::Working);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();

    let j = tasks::create(&ctx.state, job("later"), T0).await.unwrap();
    tasks::reserve(&ctx.state, &j.id, "a", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(
        state_of(&ctx, &j.id).await.0,
        TaskState::Queued,
        "working: not available"
    );
    assert_eq!(tasks::dispatch_all(&ctx.state, T0).await.unwrap(), 0);

    ctx.harness.set_state("a", HarnessState::Idle);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap(); // row → idle → dispatch
    assert_eq!(state_of(&ctx, &j.id).await.0, TaskState::Assigned);
    assert_eq!(ctx.harness.prompts().len(), 1, "delivery notice sent once");
}

#[tokio::test]
async fn not_before_holds_the_job_until_its_time() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let j = tasks::create(
        &ctx.state,
        CreateTask {
            assign: Some("a".into()),
            not_before: Some(T0 + 60_000),
            ..job("tonight")
        },
        T0,
    )
    .await
    .unwrap();
    assert_eq!(j.state, TaskState::Queued);
    assert_eq!(
        tasks::dispatch_all(&ctx.state, T0 + 59_999).await.unwrap(),
        0
    );
    assert_eq!(
        tasks::dispatch_all(&ctx.state, T0 + 60_000).await.unwrap(),
        1
    );
    assert_eq!(state_of(&ctx, &j.id).await.0, TaskState::Assigned);
}

#[tokio::test]
async fn highest_priority_reservation_goes_first() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let busy = tasks::create(&ctx.state, job("busy"), T0).await.unwrap();
    tasks::assign(&ctx.state, &busy.id, "a", None, "me", T0)
        .await
        .unwrap();
    let low = tasks::create(&ctx.state, job("low"), T0).await.unwrap();
    let high = tasks::create(
        &ctx.state,
        CreateTask {
            priority: Some(9),
            ..job("high")
        },
        T0 + 1,
    )
    .await
    .unwrap();
    for t in [&low, &high] {
        tasks::reserve(&ctx.state, &t.id, "a", None, "me", T0)
            .await
            .unwrap();
    }
    tasks::report(
        &ctx.state,
        &busy.id,
        "a",
        Some(TaskState::Completed),
        None,
        T0 + 2,
    )
    .await
    .unwrap();
    assert_eq!(state_of(&ctx, &high.id).await.0, TaskState::Assigned);
    assert_eq!(
        state_of(&ctx, &low.id).await.0,
        TaskState::Queued,
        "one at a time"
    );
}

#[tokio::test]
async fn dead_target_keeps_the_reservation_waiting() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    ctx.harness.kill("a");
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
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
    assert_eq!(j.state, TaskState::Queued);
    let w = tasks::waiting(&ctx.state, &j).await.unwrap().unwrap();
    assert!(w.reason.contains("is dead"), "{}", w.reason);
    assert_eq!(
        tasks::dispatch_all(&ctx.state, T0 + 3_600_000)
            .await
            .unwrap(),
        0
    );
    assert!(j.target_agent_id.is_some());
}

#[tokio::test]
async fn release_drops_the_releasers_own_reservation() {
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
    assert_eq!(
        state_of(&ctx, &j.id).await.0,
        TaskState::Assigned,
        "idle: delivered at once"
    );
    let r = tasks::release(&ctx.state, &j.id, "a", Some("can't".into()), T0 + 1)
        .await
        .unwrap();
    assert_eq!(r.state, TaskState::Queued, "not handed straight back");
    assert!(r.target_agent_id.is_none());
}

#[tokio::test]
async fn explicit_assign_to_another_agent_overrides_the_reservation() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    ctx.spawn("b", "omp").await;
    let busy = tasks::create(&ctx.state, job("busy"), T0).await.unwrap();
    tasks::assign(&ctx.state, &busy.id, "a", None, "me", T0)
        .await
        .unwrap();
    let j = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::reserve(&ctx.state, &j.id, "a", None, "me", T0)
        .await
        .unwrap();

    let t = tasks::assign(&ctx.state, &j.id, "b", None, "me", T0)
        .await
        .unwrap();
    assert_eq!(t.owner_id.unwrap(), agent_id(&ctx, "b").await);
    assert!(t.target_agent_id.is_none(), "reservation for a dropped");
}

#[tokio::test]
async fn removing_the_target_returns_reservations_to_the_general_queue() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let busy = tasks::create(&ctx.state, job("busy"), T0).await.unwrap();
    tasks::assign(&ctx.state, &busy.id, "a", None, "me", T0)
        .await
        .unwrap();
    let j = tasks::create(&ctx.state, job("x"), T0).await.unwrap();
    tasks::reserve(&ctx.state, &j.id, "a", None, "me", T0)
        .await
        .unwrap();

    tower_server::sessions::stop(&ctx.state, "a", true)
        .await
        .unwrap();
    let t = tasks::get_task(&ctx.state, &j.id).await.unwrap().unwrap();
    assert_eq!(t.state, TaskState::Queued);
    assert!(t.target_agent_id.is_none());
}

#[tokio::test]
async fn not_before_needs_a_target() {
    let ctx = common::boot().await;
    let req = CreateTask {
        not_before: Some(T0),
        ..job("y")
    };
    let e = tasks::create(&ctx.state, req, T0).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Invalid);
}

#[tokio::test]
async fn routes_answer_waiting_when_the_agent_is_busy() {
    let ctx = common::boot().await;
    ctx.spawn("a", "omp").await;
    let (_, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "busy", "assign": "a"})),
        )
        .await;
    assert_eq!(v["task"]["state"], "assigned");
    assert!(v["waiting"].is_null(), "delivered: {v}");
    let busy = v["task"]["id"].as_str().unwrap().to_string();
    let (_, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "next"})),
        )
        .await;
    let id = v["task"]["id"].as_str().unwrap().to_string();

    let (s, v) = ctx
        .req(
            "POST",
            &format!("/v1/tasks/{id}/assign"),
            Some(serde_json::json!({"to": "a"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["task"]["state"], "queued");
    assert!(v["task"]["target_agent_id"].is_string());
    assert_eq!(v["waiting"]["busy_with"], busy.as_str());
    assert_eq!(v["waiting"]["agent_state"], "idle");

    let (s, v) = ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "third", "assign": "a"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["task"]["state"], "queued");
    assert_eq!(v["waiting"]["busy_with"], busy.as_str());
}
