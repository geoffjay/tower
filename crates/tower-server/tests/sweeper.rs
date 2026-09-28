//! Deadline sweeper tests (plan T2.3 verify): clock-injected boundary,
//! per-kind agent notification, respond-vs-expiry race.

mod common;

use axum::http::StatusCode;
use tower_server::sweeper::{sweep_deadlines, sweep_events, EXPIRED_QUESTION_PROMPT};

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

const DAY: i64 = 86_400_000;

/// Append `n` events and backdate them to `ts` (append stamps `now`).
async fn events_at(ctx: &common::Ctx, n: usize, ts: i64) -> Vec<i64> {
    let mut seqs = vec![];
    for _ in 0..n {
        let e = ctx
            .state
            .events
            .append(
                tower_core::EventKind::AgentOutput,
                Some("agent"),
                Some("a_1"),
                serde_json::json!({"text": "x"}),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE events SET ts=?1 WHERE seq=?2")
            .bind(ts)
            .bind(e.seq)
            .execute(&ctx.state.pool)
            .await
            .unwrap();
        seqs.push(e.seq);
    }
    seqs
}

#[tokio::test]
async fn retention_deletes_exactly_the_expired_rows() {
    let ctx = common::boot().await;
    let now = 100 * DAY;
    let horizon = now - 14 * DAY; // default event_retention_days
    let old = events_at(&ctx, 3, horizon - 1).await;
    let edge = events_at(&ctx, 1, horizon).await;
    let fresh = events_at(&ctx, 2, now - DAY).await;

    assert_eq!(sweep_events(&ctx.state, now).await.unwrap(), 3);
    let kept: Vec<i64> = sqlx::query_scalar("SELECT seq FROM events ORDER BY seq")
        .fetch_all(&ctx.state.pool)
        .await
        .unwrap();
    assert_eq!(
        kept,
        [edge.clone(), fresh].concat(),
        "ts == horizon is kept"
    );

    // a cursor from before the horizon resumes at the oldest kept event
    let replay = ctx.state.events.since(old[0] - 1, 10).await.unwrap();
    assert_eq!(replay.first().map(|e| e.seq), Some(edge[0]));

    // idempotent
    assert_eq!(sweep_events(&ctx.state, now).await.unwrap(), 0);
}

#[tokio::test]
async fn retention_never_passes_an_open_stream_cursor() {
    let ctx = common::boot().await;
    let now = 100 * DAY;
    let old = events_at(&ctx, 4, now - 30 * DAY).await;
    // a stream that has sent through old[1] still needs old[2..]
    let stream = ctx.state.events.track_cursor(old[1]);

    assert_eq!(sweep_events(&ctx.state, now).await.unwrap(), 2);
    let replay = ctx.state.events.since(old[1], 10).await.unwrap();
    assert_eq!(replay.iter().map(|e| e.seq).collect::<Vec<_>>(), &old[2..]);

    // the stream closes: the rest goes
    drop(stream);
    assert_eq!(sweep_events(&ctx.state, now).await.unwrap(), 2);
}

#[tokio::test]
async fn pruning_everything_never_reuses_a_seq() {
    let ctx = common::boot().await;
    let now = 100 * DAY;
    let old = events_at(&ctx, 2, now - 30 * DAY).await;
    sweep_events(&ctx.state, now).await.unwrap();
    let next = events_at(&ctx, 1, now).await;
    assert!(next[0] > old[1], "AUTOINCREMENT seq stays monotonic");
}
