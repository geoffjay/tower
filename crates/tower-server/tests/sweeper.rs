//! Deadline sweeper tests (plan T2.3 verify): clock-injected boundary,
//! per-kind agent notification, respond-vs-expiry race.

mod common;

use axum::http::StatusCode;
use tower_server::sweeper::{sweep_deadlines, EXPIRED_QUESTION_PROMPT};

#[tokio::test]
async fn expiry_boundary_is_deadline_inclusive() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 60).await;
    let deadline = ctx.deadline_of(&id).await;

    assert_eq!(sweep_deadlines(&ctx.state, deadline - 1).await.unwrap(), 0);
    assert_eq!(ctx.status_of(&id).await, "pending");

    assert_eq!(sweep_deadlines(&ctx.state, deadline).await.unwrap(), 1);
    assert_eq!(ctx.status_of(&id).await, "expired");

    // idempotent: nothing left to expire
    assert_eq!(sweep_deadlines(&ctx.state, deadline + 1).await.unwrap(), 0);
}

#[tokio::test]
async fn expired_question_prompts_agent_to_proceed() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 1).await;
    sweep_deadlines(&ctx.state, ctx.deadline_of(&id).await)
        .await
        .unwrap();

    let prompts = ctx.harness.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].0, "backend");
    assert_eq!(prompts[0].1, EXPIRED_QUESTION_PROMPT);
    assert!(ctx.harness.keys().is_empty());
    assert_eq!(ctx.event_count("approval.expired").await, 1);
}

#[tokio::test]
async fn expired_approval_is_denied_never_granted() {
    let ctx = common::boot().await;
    ctx.spawn("coder", "claude").await;
    let id = ctx.ask_operator("coder", "approval", 1).await;
    sweep_deadlines(&ctx.state, ctx.deadline_of(&id).await)
        .await
        .unwrap();

    assert_eq!(
        ctx.harness.keys(),
        vec![("coder".to_string(), vec!["esc".to_string()])]
    );
    assert!(ctx.harness.prompts().is_empty());
    assert_eq!(ctx.event_count("approval.expired").await, 1);
}

#[tokio::test]
async fn operator_question_to_agent_expires_without_notification() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages",
            Some(serde_json::json!({
                "to": "backend", "kind": "question",
                "parts": [{"text": "status?"}], "deadline_s": 1,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["message"]["id"].as_str().unwrap().to_string();
    let prompts_before = ctx.harness.prompts().len(); // the delivery itself

    sweep_deadlines(&ctx.state, ctx.deadline_of(&id).await)
        .await
        .unwrap();
    assert_eq!(ctx.status_of(&id).await, "expired");
    assert_eq!(ctx.harness.prompts().len(), prompts_before);
    assert_eq!(ctx.event_count("approval.expired").await, 0);
}

#[tokio::test]
async fn respond_after_expiry_conflicts() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 1).await;
    sweep_deadlines(&ctx.state, ctx.deadline_of(&id).await)
        .await
        .unwrap();

    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"parts": [{"text": "too late"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    // only the expiry prompt reached the agent, never the late answer
    assert_eq!(ctx.harness.prompts().len(), 1);
}
