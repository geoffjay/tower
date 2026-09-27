//! Server integration tests: inventory reconcile + control routes with
//! FakeHarness (plan T5.1/T5.2 verify, D§16).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower_driver::fake::FakeHarness;
use tower_driver::HarnessState;
use tower_server::AppState;
use tower_server::{Config, Paths};

fn temp_paths(n: &str) -> (Paths, tempdir::TempDir) {
    let dir = tempdir::TempDir::new(n).unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
    };
    paths.ensure_dirs().unwrap();
    (paths, dir)
}

async fn boot(harness: FakeHarness) -> (AppState, FakeHarness, tempdir::TempDir) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (paths, dir) = temp_paths(&format!("tower-m5-{n}"));
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let state = AppState::new(
        pool,
        events,
        Config::default(),
        "t".into(),
        Arc::new(harness.clone()),
    );
    tower_server::inventory::ensure_local_machine(&state)
        .await
        .unwrap();
    (state, harness, dir)
}

fn router(state: AppState) -> axum::Router {
    tower_server::serve::router(state)
}

async fn json_req(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = match body {
        Some(v) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    };
    let resp = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_default()
    };
    (status, v)
}

#[tokio::test]
async fn reconcile_marks_dead_and_finds_adoptable() {
    let (state, harness, _d) =
        boot(FakeHarness::new().with_agent("stray", "pi", HarnessState::Idle)).await;

    // reconcile sees the stray as adoptable
    let candidates = tower_server::inventory::adoption_candidates(&state)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["name"], "stray");

    // adopt it
    let adopted = tower_server::inventory::adopt(&state, "stray")
        .await
        .unwrap();
    assert!(adopted.adopted);
    assert_eq!(adopted.state, tower_core::AgentState::Idle);

    // it's no longer adoptable
    let candidates = tower_server::inventory::adoption_candidates(&state)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 0);

    // harness agent disappears → reconcile marks the row dead
    harness.kill("stray");
    tower_server::inventory::reconcile(&state).await.unwrap();
    let agent = tower_server::inventory::get_agent(&state, "stray")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, tower_core::AgentState::Dead);
}

#[tokio::test]
async fn spawn_prompt_stop_routes() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state.clone());

    // spawn
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "writer", "kind": "pi"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["name"], "writer");
    assert_eq!(v["agent"]["kind"], "pi");

    // duplicate spawn → conflict
    let (status, _) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "writer", "kind": "pi"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // list
    let (status, v) = json_req(&app, "GET", "/v1/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["agents"].as_array().unwrap().len(), 1);

    // prompt
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents/writer/prompt",
        Some(serde_json::json!({"text": "do the thing", "wait": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);

    // the harness saw the prompt
    let prompts = harness.prompts();
    assert_eq!(
        prompts,
        vec![("writer".to_string(), "do the thing".into(), false)]
    );

    // show
    let (status, v) = json_req(&app, "GET", "/v1/agents/writer", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["agent"]["name"], "writer");
    // prompt flipped the fake's state to Working
    assert_eq!(v["agent"]["state"], "working");

    // read
    let (status, v) = json_req(&app, "GET", "/v1/agents/writer/read", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["output"].is_string());

    // unknown agent → 404 envelope
    let (status, v) = json_req(&app, "GET", "/v1/agents/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");

    // stop (keep row)
    let (status, _) = json_req(
        &app,
        "POST",
        "/v1/agents/writer/stop",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let agent = tower_server::inventory::get_agent(&state, "writer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, tower_core::AgentState::Dead);
}

#[tokio::test]
async fn adopt_via_spawn_route() {
    let (state, _harness, _d) =
        boot(FakeHarness::new().with_agent("found", "claude", HarnessState::Working)).await;
    let app = router(state);

    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "found", "adopt": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["adopted"], true);
    assert_eq!(v["agent"]["state"], "working");
}
