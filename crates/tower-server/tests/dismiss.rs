//! Dismiss (D§9.3): close a pending question/approval without notifying the
//! sender — the item stays in history, marked so an agent reading it back
//! knows not to act on it.
mod common;

use tower_driver::HarnessState;

#[tokio::test]
async fn dismiss_closes_pending_without_notifying_sender() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness.set_state("backend", HarnessState::Blocked);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    let id: String =
        sqlx::query_scalar("SELECT id FROM messages WHERE from_id='backend' AND status='pending'")
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();

    let (status, v) = ctx
        .req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{v}");
    assert_eq!(v["message"]["status"], "dismissed");
    assert!(v["message"]["responded_at"].is_i64(), "responded_at set");

    // closed: no prompt went to the agent, nothing else is pending
    assert!(ctx.harness.prompts().is_empty());
    assert_eq!(ctx.status_of(&id).await, "dismissed");
}

#[tokio::test]
async fn dismissed_item_stays_in_agent_history() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness.set_state("backend", HarnessState::Blocked);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    let id: String =
        sqlx::query_scalar("SELECT id FROM messages WHERE from_id='backend' AND status='pending'")
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
    ctx.req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;

    // agent history (`?agent=`) keeps the row with status=dismissed
    let (status, v) = ctx.req("GET", "/v1/messages?agent=backend", None).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let items = v["messages"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id);
    assert_eq!(items[0]["status"], "dismissed");
}

#[tokio::test]
async fn dismiss_conflicts_on_non_pending() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 60).await;
    ctx.req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;
    // second dismiss: already closed → 409, no state change
    let (status, _) = ctx
        .req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn dismissed_never_expires_or_notifies() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness.set_state("backend", HarnessState::Blocked);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    let id: String =
        sqlx::query_scalar("SELECT id FROM messages WHERE from_id='backend' AND status='pending'")
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
    ctx.req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;
    // backdate the deadline: the sweeper must skip dismissed rows
    sqlx::query("UPDATE messages SET deadline_at=0 WHERE id=?1")
        .bind(&id)
        .execute(&ctx.state.pool)
        .await
        .unwrap();
    assert_eq!(
        tower_server::sweeper::sweep_deadlines(&ctx.state, 1)
            .await
            .unwrap(),
        0
    );
    assert_eq!(ctx.status_of(&id).await, "dismissed");
    assert!(ctx.harness.prompts().is_empty());
    assert_eq!(ctx.event_count("approval.expired").await, 0);
}

#[tokio::test]
async fn dismiss_then_unblock_does_not_resurrect() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness.set_state("backend", HarnessState::Blocked);
    tower_server::inventory::reconcile(&ctx.state)
        .await
        .unwrap();
    let id: String =
        sqlx::query_scalar("SELECT id FROM messages WHERE from_id='backend' AND status='pending'")
            .fetch_one(&ctx.state.pool)
            .await
            .unwrap();
    ctx.req("POST", &format!("/v1/messages/{id}/dismiss"), None)
        .await;

    // leaving blocked closes open items only; dismissed stays dismissed
    ctx.harness.set_state("backend", HarnessState::Working);
    ctx.harness
        .push_event(tower_driver::HarnessEvent::StateChange {
            name: "backend".into(),
            from: HarnessState::Blocked,
            to: HarnessState::Working,
            detail: None,
        });
    let _pump = tower_server::pump::spawn(ctx.state.clone());
    for _ in 0..50 {
        let agent = tower_server::inventory::get_agent(&ctx.state, "backend")
            .await
            .unwrap()
            .unwrap();
        if agent.state == tower_core::AgentState::Working {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(ctx.status_of(&id).await, "dismissed");
}
