//! Pump integration tests (plan T2.1 verify): blocked detection → exactly
//! one inbox item per episode; claude agents get an approval.

mod common;

use std::time::Duration;

use tower_driver::{HarnessEvent, HarnessState};

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
