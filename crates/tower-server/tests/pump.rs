//! Pump integration tests (plan T2.1 verify): blocked detection → exactly
//! one inbox item per episode; claude agents get an approval.

mod common;

use std::time::Duration;

use tower_driver::{HarnessEvent, HarnessState};
use tower_server::Config;

fn blocked(name: &str, from: HarnessState) -> HarnessEvent {
    HarnessEvent::StateChange {
        name: name.into(),
        from,
        to: HarnessState::Blocked,
        detail: None,
    }
}

/// Poll until `sql` (a COUNT) is non-zero or ~1s passes; returns the count.
async fn wait_count(ctx: &common::Ctx, sql: &str) -> i64 {
    for _ in 0..50 {
        let n: i64 = sqlx::query_scalar(sql)
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
        if n > 0 {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    0
}

async fn wait_status(ctx: &common::Ctx, id: &str, status: &str) -> bool {
    for _ in 0..50 {
        if ctx.status_of(id).await == status {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn blocked_detection_creates_exactly_one_question() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness
        .push_event(blocked("backend", HarnessState::Working));
    // same-episode repeat must NOT open a second question
    ctx.harness
        .push_event(blocked("backend", HarnessState::Blocked));
    let _pump = tower_server::pump::spawn(ctx.state.clone());

    let sql = "SELECT COUNT(*) FROM messages WHERE kind='question' AND status='pending'";
    assert!(wait_count(&ctx, sql).await > 0);
    tokio::time::sleep(Duration::from_millis(100)).await; // let the repeat land
    assert_eq!(wait_count(&ctx, sql).await, 1);
    assert_eq!(ctx.event_count("message.created").await, 1);
}

#[tokio::test]
async fn blocked_claude_agent_opens_an_approval() {
    let ctx = common::boot().await;
    ctx.spawn("coder", "claude").await;
    ctx.harness
        .push_event(blocked("coder", HarnessState::Working));
    let _pump = tower_server::pump::spawn(ctx.state.clone());

    let n = wait_count(
        &ctx,
        "SELECT COUNT(*) FROM messages WHERE from_id='coder' AND kind='approval'",
    )
    .await;
    assert_eq!(n, 1);
}

/// Seen live: claude blocks on its folder-trust dialog before tower's row
/// exists, so the first observers are spawn's reconcile and the pump's
/// `AgentUp` — no `StateChange` ever fires. Each path must still yield
/// exactly one inbox item.
#[tokio::test]
async fn blocked_first_seen_by_reconcile_then_pump_opens_one_item() {
    let ctx = common::boot().await;
    ctx.spawn("coder", "claude").await;
    ctx.harness.set_state("coder", HarnessState::Blocked);

    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    // the pump's first poll reports the same agent as AgentUp(blocked)
    let snap = tower_driver::Harness::snapshot(&ctx.harness).await.unwrap();
    ctx.harness
        .push_event(HarnessEvent::AgentUp(snap[0].clone()));
    let _pump = tower_server::pump::spawn(ctx.state.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;

    let sql = "SELECT COUNT(*) FROM messages WHERE from_id='coder' AND kind='approval'";
    assert_eq!(wait_count(&ctx, sql).await, 1);
    assert_eq!(
        ctx.event_count("agent.state").await,
        2,
        "launching→idle, idle→blocked"
    );
}

#[tokio::test]
async fn blocked_inbox_keeps_pane_context_and_uses_configured_deadline() {
    let mut config = Config::default();
    config.inbox.blocked_deadline_s = 7_200;
    let ctx = common::boot_with_config(config).await;
    ctx.spawn("backend", "pi").await;
    ctx.harness
        .append_output("backend", "Which approach should I take?");
    ctx.harness
        .push_event(blocked("backend", HarnessState::Working));
    let _pump = tower_server::pump::spawn(ctx.state.clone());

    let sql = "SELECT COUNT(*) FROM messages WHERE kind='question' AND status='pending'";
    assert_eq!(wait_count(&ctx, sql).await, 1);
    let id: String =
        sqlx::query_scalar("SELECT id FROM messages WHERE kind='question' AND status='pending'")
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
    let (status, response) = ctx
        .req("GET", "/v1/messages?to=me&status=pending", None)
        .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let message = &response["messages"][0];
    assert_eq!(message["id"], id);
    assert_eq!(
        message["parts"][1]["data"]["context"],
        "Which approach should I take?"
    );
    assert_eq!(message["parts"][1]["metadata"]["source"], "blocked_inbox");
    let (created_at, deadline_at): (i64, i64) =
        sqlx::query_as("SELECT created_at, deadline_at FROM messages WHERE id=?1")
            .bind(&id)
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
    assert_eq!(deadline_at - created_at, 7_200_000);
}

#[tokio::test]
async fn pane_answer_closes_blocked_inbox_before_expiry() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness
        .append_output("backend", "Which approach should I take?");
    ctx.harness
        .push_event(blocked("backend", HarnessState::Working));
    let _pump = tower_server::pump::spawn(ctx.state.clone());

    let sql = "SELECT COUNT(*) FROM messages WHERE from_id='backend' AND kind='question' AND status='pending'";
    assert_eq!(wait_count(&ctx, sql).await, 1);
    let id: String = sqlx::query_scalar(
        "SELECT id FROM messages WHERE from_id='backend' AND kind='question' AND status='pending'",
    )
    .fetch_one(&ctx.state.pool)
    .await
    .unwrap();
    let unrelated = ctx.ask_operator("backend", "question", 3_600).await;

    ctx.harness.set_state("backend", HarnessState::Working);
    ctx.harness.push_event(HarnessEvent::StateChange {
        name: "backend".into(),
        from: HarnessState::Blocked,
        to: HarnessState::Working,
        detail: None,
    });
    // FakeHarness snapshots its queued events when `events()` is called.
    let _unblocked_pump = tower_server::pump::spawn(ctx.state.clone());
    assert!(wait_status(&ctx, &id, "answered").await);

    sqlx::query("UPDATE messages SET deadline_at=0 WHERE id=?1")
        .bind(&id)
        .execute(&ctx.state.pool)
        .await
        .unwrap();
    assert_eq!(
        tower_server::sweeper::sweep_deadlines(&ctx.state, tower_core::now_ms())
            .await
            .unwrap(),
        0
    );
    assert!(ctx.harness.prompts().is_empty());
    assert_eq!(ctx.status_of(&id).await, "answered");
    assert_eq!(ctx.status_of(&unrelated).await, "pending");
}
