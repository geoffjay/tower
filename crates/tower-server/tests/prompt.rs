//! Prompt route semantics (D§8.1): a `--wait` that sees no state change is a
//! delivered prompt, not an error; failures carry their real error code.

mod common;

use axum::http::StatusCode;
use serde_json::json;

/// Seen live: omp answered "OK" before herdr detection ever saw `working`,
/// so `herdr agent prompt --wait` reported agent_prompt_stalled and tower
/// turned a delivered prompt into a 404.
#[tokio::test]
async fn stalled_wait_is_a_delivered_prompt() {
    let ctx = common::boot().await;
    ctx.spawn("w1", "omp").await;
    ctx.harness.stall_prompts(true);
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/agents/w1/prompt",
            Some(json!({"text": "hi", "wait": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["stalled"], true);
    assert_eq!(v["state"], "idle");
    assert_eq!(ctx.harness.prompts().len(), 1, "delivered exactly once");
}

#[tokio::test]
async fn prompt_errors_use_their_real_code() {
    let ctx = common::boot().await;
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/agents/ghost/prompt",
            Some(json!({"text": "hi"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");

    ctx.spawn("w1", "omp").await;
    ctx.harness.kill("w1"); // row exists, pane gone → a driver failure
    let (status, v) = ctx
        .req("POST", "/v1/agents/w1/prompt", Some(json!({"text": "hi"})))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["error"]["code"], "driver");
}
