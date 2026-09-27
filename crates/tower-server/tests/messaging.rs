//! Messaging integration tests (plan T1.2/T2.2 verify, D§9.3): both
//! delivery directions against FakeHarness, respond semantics, approvals.

mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn to_human_message_lands_in_inbox_pending() {
    let ctx = common::boot().await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages",
            Some(serde_json::json!({
                "to": "me", "to_kind": "human", "kind": "question",
                "parts": [{"text": "which database?"}],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["message"]["status"], "pending");
    assert!(v["message"]["deadline_at"].is_i64());

    let (status, v) = ctx.req("GET", "/v1/messages?to=me", None).await;
    assert_eq!(status, StatusCode::OK);
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "question");
    assert_eq!(msgs[0]["status"], "pending");
}

#[tokio::test]
async fn to_agent_message_prompts_the_agent() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages",
            Some(serde_json::json!({"to": "backend", "parts": [{"text": "do the thing"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // recipient kind auto-resolved to agent; default kind is prompt
    assert_eq!(v["message"]["to_kind"], "agent");
    assert_eq!(v["message"]["kind"], "prompt");
    assert_eq!(v["message"]["status"], "delivered");
    assert_eq!(
        ctx.harness.prompts(),
        vec![("backend".into(), "do the thing".into(), false)]
    );
}

#[tokio::test]
async fn explicit_agent_recipient_must_exist() {
    let ctx = common::boot().await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages",
            Some(serde_json::json!({
                "to": "ghost", "to_kind": "agent", "parts": [{"text": "hi"}],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    let (_, v) = ctx.req("GET", "/v1/messages", None).await;
    assert!(
        v["messages"].as_array().unwrap().is_empty(),
        "no row written"
    );
}

#[tokio::test]
async fn respond_answers_and_unblocks_agent_sender() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 300).await;
    assert!(
        ctx.harness.prompts().is_empty(),
        "to-human send never prompts"
    );

    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"parts": [{"text": "use sqlite"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["message"]["status"], "answered");
    assert!(v["message"]["responded_at"].is_i64());
    assert_eq!(
        ctx.harness.prompts(),
        vec![("backend".into(), "use sqlite".into(), false)]
    );

    // double respond conflicts and never re-delivers
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"parts": [{"text": "again"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "conflict");
    assert_eq!(ctx.harness.prompts().len(), 1);
}

#[tokio::test]
async fn question_respond_needs_text() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let id = ctx.ask_operator("backend", "question", 300).await;
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": true})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(ctx.status_of(&id).await, "pending");
}

#[tokio::test]
async fn respond_unknown_message_404s() {
    let ctx = common::boot().await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages/m_nope/respond",
            Some(serde_json::json!({"parts": [{"text": "hi"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "not_found");
}

#[tokio::test]
async fn approval_respond_sends_keys_once() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    let id = ctx.ask_operator("backend", "approval", 300).await;

    // approve → keys `1` (D§8.2), not a text prompt
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["message"]["status"], "answered");
    assert_eq!(
        ctx.harness.keys(),
        vec![("backend".into(), vec!["1".into()])]
    );
    assert!(ctx.harness.prompts().is_empty());

    // a deny racing the approve conflicts; no second key
    let (status, _) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": false})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(ctx.harness.keys().len(), 1);
}

#[tokio::test]
async fn approval_deny_sends_key_two() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    let id = ctx.ask_operator("backend", "approval", 300).await;
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        ctx.harness.keys(),
        vec![("backend".into(), vec!["2".into()])]
    );
}

#[tokio::test]
async fn approval_without_decision_is_invalid_and_stays_pending() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    let id = ctx.ask_operator("backend", "approval", 300).await;
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"parts": [{"text": "sure"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"]["code"], "invalid");
    assert!(ctx.harness.keys().is_empty());
    assert_eq!(ctx.status_of(&id).await, "pending");
}

#[tokio::test]
async fn failed_agent_delivery_marks_message_failed() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.harness.kill("backend"); // row survives, pane gone → driver NotFound
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/messages",
            Some(serde_json::json!({"to": "backend", "parts": [{"text": "hi"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["error"]["code"], "driver");
    let (_, v) = ctx
        .req("GET", "/v1/messages?to=backend&status=failed", None)
        .await;
    assert_eq!(v["messages"].as_array().unwrap().len(), 1);
}
