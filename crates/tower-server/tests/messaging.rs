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
async fn agent_filter_is_the_agents_history_both_directions() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    ctx.spawn("writer", "pi").await;
    let send = |to: &'static str, text: &'static str| serde_json::json!({"to": to, "parts": [{"text": text}]});
    ctx.req("POST", "/v1/messages", Some(send("backend", "to backend")))
        .await;
    ctx.req("POST", "/v1/messages", Some(send("writer", "to writer")))
        .await;
    let asked = ctx.ask_operator("backend", "question", 300).await;

    let (_, agent) = ctx.req("GET", "/v1/agents/backend", None).await;
    let id = agent["agent"]["id"].as_str().unwrap().to_string();
    for who in ["backend".to_string(), id] {
        let (status, v) = ctx
            .req("GET", &format!("/v1/messages?agent={who}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let msgs = v["messages"].as_array().unwrap();
        let texts: Vec<_> = msgs
            .iter()
            .map(|m| m["parts"][0]["text"].as_str().unwrap())
            .collect();
        // newest first: its question to me, then the prompt it received
        assert_eq!(texts, ["need input", "to backend"], "agent={who}");
        assert_eq!(msgs[0]["id"], asked);
    }
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

/// claude's tool-permission menu; option 2 widens the grant.
const PERMISSION_MENU: &str = " Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for echo commands in /tmp
   3. No, and tell Claude what to do differently (esc)";

#[tokio::test]
async fn approval_respond_sends_keys_once() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    ctx.harness.append_output("backend", PERMISSION_MENU);
    let id = ctx.ask_operator("backend", "approval", 300).await;

    // approve → confirm the plain "Yes" (D§8.2), not a text prompt
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
        vec![("backend".into(), vec!["enter".into()])]
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
async fn approval_deny_is_escape_never_option_two() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    ctx.harness.append_output("backend", PERMISSION_MENU);
    let id = ctx.ask_operator("backend", "approval", 300).await;
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    // `2` would be "Yes, and don't ask again" — deny must be esc
    assert_eq!(
        ctx.harness.keys(),
        vec![("backend".into(), vec!["esc".into()])]
    );
}

#[tokio::test]
async fn approve_without_a_safe_yes_on_screen_fails_instead_of_guessing() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "claude").await;
    ctx.harness.append_output(
        "backend",
        " Allow edits?\n ❯ 1. Yes, allow all edits during this session\n   2. No",
    );
    let id = ctx.ask_operator("backend", "approval", 300).await;
    let (status, v) = ctx
        .req(
            "POST",
            &format!("/v1/messages/{id}/respond"),
            Some(serde_json::json!({"approve": true})),
        )
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["error"]["code"], "driver");
    assert!(ctx.harness.keys().is_empty(), "no key sent");
    assert_eq!(ctx.status_of(&id).await, "failed");
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
