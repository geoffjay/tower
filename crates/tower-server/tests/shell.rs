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
use tower_server::AppState;
use tower_server::{Config, Paths};

static TEST_N: AtomicU32 = AtomicU32::new(0);

async fn test_state() -> (AppState, tempfile::TempDir) {
    let n = TEST_N.fetch_add(1, Ordering::SeqCst);
    let dir = tempfile::Builder::new()
        .prefix(&format!("tower-it-{n}"))
        .tempdir()
        .unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
        agents_dir: dir.path().join("agents"),
    };
    paths.ensure_dirs().unwrap();
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let state = AppState::new(
        pool,
        events,
        Config::default(),
        "test-token".into(),
        std::sync::Arc::new(tower_driver::fake::FakeHarness::new()),
    );
    (state, dir)
}

fn router(state: AppState) -> Router {
    tower_server::serve::router(state)
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

    // exclude drops matching types; the batch id still covers them
    let no_agents = read_sse_bounded(&app, "/v1/events?cursor=0&exclude=type:agent", 2).await;
    assert!(no_agents.contains("task.created"), "{no_agents}");
    assert!(!no_agents.contains("agent.state"), "{no_agents}");
    assert!(no_agents.contains("id: 3"), "{no_agents}");
}

async fn get_json(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn seqs(page: &serde_json::Value) -> Vec<i64> {
    page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect()
}

/// Seqs 1..=12: every third event an agent screen (`agent.output`), the
/// rest task events for t_1 (seq 1..=6) and t_2 (seq 7..=12).
async fn history_log(state: &AppState) {
    for seq in 1..=12 {
        let (kind, st, id) = if seq % 3 == 0 {
            (EventKind::AgentOutput, "agent", "a_1")
        } else {
            (
                EventKind::TaskCreated,
                "task",
                if seq <= 6 { "t_1" } else { "t_2" },
            )
        };
        state
            .events
            .append(kind, Some(st), Some(id), serde_json::json!({ "n": seq }))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn history_pages_newest_first_without_gaps() {
    let (state, _d) = test_state().await;
    history_log(&state).await;
    let app = router(state);

    let (status, first) = get_json(&app, "/v1/events/history?limit=5").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(seqs(&first), vec![12, 11, 10, 9, 8]);
    assert_eq!(first["head"], 12);
    assert_eq!(first["more"], true);
    assert_eq!(first["events"][0]["kind"], "agent.output");
    assert_eq!(first["events"][0]["payload"]["n"], 12);

    // paging on from the oldest seq held: no overlap, no gap, and the last
    // page says there is nothing older
    let (_, second) = get_json(&app, "/v1/events/history?limit=5&before=8").await;
    assert_eq!(seqs(&second), vec![7, 6, 5, 4, 3]);
    assert_eq!(second["more"], true);
    let (_, last) = get_json(&app, "/v1/events/history?limit=5&before=3").await;
    assert_eq!(seqs(&last), vec![2, 1]);
    assert_eq!(last["more"], false);
    // a page that ends exactly at the start is not "more" either
    let (_, exact) = get_json(&app, "/v1/events/history?limit=2&before=3").await;
    assert_eq!(seqs(&exact), vec![2, 1]);
    assert_eq!(exact["more"], false);

    // the default page is 50: the whole 12-event log
    let (_, all) = get_json(&app, "/v1/events/history").await;
    assert_eq!(seqs(&all), (1..=12).rev().collect::<Vec<_>>());
    assert_eq!(all["more"], false);
    // `before` past the head reads from the head
    let (_, past) = get_json(&app, "/v1/events/history?limit=1&before=999").await;
    assert_eq!(seqs(&past), vec![12]);
}

#[tokio::test]
async fn history_filters_match_the_stream() {
    let (state, _d) = test_state().await;
    history_log(&state).await;
    let app = router(state);

    // screens excluded: the newest page skips every third seq, and `more`
    // counts only matching events
    let (_, no_screens) =
        get_json(&app, "/v1/events/history?limit=4&exclude=type:agent.output").await;
    assert_eq!(seqs(&no_screens), vec![11, 10, 8, 7]);
    assert_eq!(no_screens["more"], true);
    let (_, rest) = get_json(
        &app,
        "/v1/events/history?limit=4&exclude=agent.output&before=7",
    )
    .await;
    assert_eq!(seqs(&rest), vec![5, 4, 2, 1]);
    assert_eq!(rest["more"], false);

    let (_, screens) = get_json(&app, "/v1/events/history?filter=type:agent").await;
    assert_eq!(seqs(&screens), vec![12, 9, 6, 3]);

    // subject: exact type:id, ASCII case-insensitive, combined with filters
    let (_, t1) = get_json(&app, "/v1/events/history?subject=TASK:t_1").await;
    assert_eq!(seqs(&t1), vec![5, 4, 2, 1]);
    let (_, none) = get_json(
        &app,
        "/v1/events/history?subject=task:t_1&filter=type:agent",
    )
    .await;
    assert_eq!(seqs(&none), Vec::<i64>::new());
    assert_eq!(none["more"], false);
    assert_eq!(none["head"], 12);

    // an empty exclude is no filter, not "exclude everything"
    let (_, empty) = get_json(&app, "/v1/events/history?limit=1&exclude=").await;
    assert_eq!(seqs(&empty), vec![12]);
}

#[tokio::test]
async fn history_head_hands_off_to_the_stream() {
    let (state, _d) = test_state().await;
    history_log(&state).await;
    let app = router(state.clone());

    let (_, page) = get_json(&app, "/v1/events/history?limit=3").await;
    let head = page["head"].as_i64().unwrap();
    state
        .events
        .append(
            EventKind::TaskCompleted,
            Some("task"),
            Some("t_9"),
            serde_json::json!({}),
        )
        .await
        .unwrap();

    // the stream from head carries exactly what came after the page
    let live = read_sse_bounded(&app, &format!("/v1/events?cursor={head}"), 2).await;
    assert!(live.contains("t_9"), "{live}");
    assert!(!live.contains("\"seq\":12"), "{live}");
    assert!(live.contains("id: 13"), "{live}");
}

#[tokio::test]
async fn history_rejects_bad_limits() {
    let (state, _d) = test_state().await;
    let app = router(state);
    for uri in [
        "/v1/events/history?limit=0",
        "/v1/events/history?limit=501",
        "/v1/events/history?limit=x",
        "/v1/events/history?before=x",
    ] {
        let (status, v) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {v}");
        assert_eq!(v["error"]["code"], "invalid", "{uri}: {v}");
    }
    // an empty log: no events, head 0
    let (status, v) = get_json(&app, "/v1/events/history").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["head"], 0);
    assert_eq!(v["more"], false);
    assert_eq!(seqs(&v), Vec::<i64>::new());
}
