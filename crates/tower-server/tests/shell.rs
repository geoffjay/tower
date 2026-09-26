//! Integration test for the server shell: routes, auth, SSE (plan T3.1/T3.3).
//!
//! Builds the router directly against a temp-home database (no listeners),
//! exercising the same code paths as `tower serve`. SSE tests read with a
//! time bound — the stream itself is endless by design.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use futures::StreamExt;
use tower_core::EventKind;
use tower_server::{api, sse, AppState};
use tower_server::{Config, Paths};

static TEST_N: AtomicU32 = AtomicU32::new(0);

async fn test_state() -> (AppState, tempdir::TempDir) {
    let n = TEST_N.fetch_add(1, Ordering::SeqCst);
    let dir = tempdir::TempDir::new(&format!("tower-it-{n}")).unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
    };
    paths.ensure_dirs().unwrap();
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let state = AppState::new(pool, events, Config::default(), "test-token".into());
    (state, dir)
}

fn router(state: AppState) -> Router {
    api::router()
        .merge(axum::Router::new().route("/v1/events", axum::routing::get(sse::events)))
        .with_state(state)
}

async fn read_sse_bounded(router: &Router, uri: &str, secs: u64) -> String {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();
    let mut out = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let next = tokio::time::timeout_at(deadline, stream.next()).await;
        match next {
            Ok(Some(chunk)) => out.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap()),
            Ok(None) | Err(_) => break,
        }
    }
    out
}

#[tokio::test]
async fn healthz_and_schema() {
    let (state, _d) = test_state().await;
    let app = router(state);
    let resp = tower::ServiceExt::oneshot(
        app,
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn schema_lists_event_types() {
    let (state, _d) = test_state().await;
    let app = router(state);
    let resp = tower::ServiceExt::oneshot(
        app,
        Request::builder()
            .uri("/v1/schema")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["event_types"].as_array().unwrap().len() >= 10);
}

#[tokio::test]
async fn sse_replays_from_cursor_and_filters() {
    let (state, _d) = test_state().await;
    state
        .events
        .append(
            EventKind::AgentStateChange,
            Some("agent"),
            Some("a_1"),
            serde_json::json!({}),
        )
        .await
        .unwrap();
    state
        .events
        .append(
            EventKind::TaskCreated,
            Some("task"),
            Some("t_1"),
            serde_json::json!({}),
        )
        .await
        .unwrap();
    state
        .events
        .append(
            EventKind::AgentStateChange,
            Some("agent"),
            Some("a_2"),
            serde_json::json!({}),
        )
        .await
        .unwrap();

    let app = router(state.clone());

    let subject = read_sse_bounded(&app, "/v1/events?cursor=0&subject=agent:a_1", 2).await;
    assert!(subject.contains("a_1"), "must contain a_1: {subject}");
    assert!(!subject.contains("a_2"), "must exclude a_2: {subject}");
    assert!(
        !subject.contains("task.created"),
        "must exclude tasks: {subject}"
    );

    let by_type = read_sse_bounded(&app, "/v1/events?cursor=0&filter=type:task", 2).await;
    assert!(by_type.contains("task.created"));
    assert!(!by_type.contains("agent.state"));

    // cursor > 0: only later events
    let after_one = read_sse_bounded(&app, "/v1/events?cursor=1", 2).await;
    assert!(!after_one.contains("a_1"), "cursor=1 excludes first event");
}
