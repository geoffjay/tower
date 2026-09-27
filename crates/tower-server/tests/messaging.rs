//! Messaging integration tests (plan T1.2 verify, D§9.3):
//! both delivery directions against FakeHarness — to-human (inbox row +
//! event) and to-agent (driver prompt).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower_driver::fake::FakeHarness;
use tower_server::{AppState, Config, Paths};

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

async fn boot(_n: &str) -> (axum::Router, FakeHarness, tempdir::TempDir) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (paths, dir) = temp_paths(&format!("tower-msg-{n}"));
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let harness = FakeHarness::new();
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
    let router = tower_server::messaging::router()
        .merge(tower_server::agents_api::router())
        .with_state(state);
    (router, harness, dir)
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

async fn spawn_agent(router: &axum::Router, name: &str) {
    let (status, v) = json_req(
        router,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": name, "kind": "pi", "workdir": "/tmp"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {v}");
}

#[tokio::test]
async fn to_human_message_lands_in_inbox_pending() {
    let (router, _h, _d) = boot("inbox").await;
    // a question to the human waits as pending with a deadline
    let (status, v) = json_req(
        &router,
        "POST",
        "/v1/messages",
        Some(serde_json::json!({
            "to": "me",
            "to_kind": "human",
            "kind": "question",
            "parts": [{"text": "which database?"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["message"]["status"], "pending");
    assert!(v["message"]["deadline_at"].is_i64());

    let (status, v) = json_req(&router, "GET", "/v1/messages?to=me", None).await;
    assert_eq!(status, StatusCode::OK);
    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "question");
    assert_eq!(msgs[0]["status"], "pending");
}

#[tokio::test]
async fn to_agent_message_prompts_the_agent() {
    let (router, harness, _d) = boot("agent-dest").await;
    spawn_agent(&router, "backend").await;

    let (status, v) = json_req(
        &router,
        "POST",
        "/v1/messages",
        Some(serde_json::json!({
            "to": "backend",
            "kind": "prompt",
            "parts": [{"text": "do the thing"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // recipient kind auto-resolved to agent
    assert_eq!(v["message"]["to_kind"], "agent");
    assert_eq!(v["message"]["status"], "delivered");

    let prompts = harness.prompts();
    assert_eq!(prompts.len(), 1, "driver must have been prompted");
    assert_eq!(prompts[0].0, "backend");
    assert_eq!(prompts[0].1, "do the thing");
}

#[tokio::test]
async fn respond_answers_and_unblocks_agent_sender() {
    let (router, harness, _d) = boot("respond").await;
    spawn_agent(&router, "backend").await;

    // agent asks the operator
    let (status, v) = json_req(
        &router,
        "POST",
        "/v1/messages",
        Some(serde_json::json!({
            "to": "me", "to_kind": "human",
            "from": "backend", "from_kind": "agent",
            "kind": "question",
            "parts": [{"text": "which approach?"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["message"]["id"].as_str().unwrap().to_string();

    // answer before responding: prompt count unchanged so far (send to human
    // doesn't prompt the agent)
    assert_eq!(harness.prompts().len(), 0);

    let (status, v) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"parts": [{"text": "use sqlite"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["message"]["status"], "answered");
    assert!(v["message"]["responded_at"].is_i64());

    // the answer was delivered to the agent as a prompt
    let prompts = harness.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].1, "use sqlite");

    // double-respond conflicts
    let (status, v) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"parts": [{"text": "again"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "conflict");
    // no duplicate prompt
    assert_eq!(harness.prompts().len(), 1);
}

#[tokio::test]
async fn respond_unknown_message_404s() {
    let (router, _h, _d) = boot("missing").await;
    let (status, v) = json_req(
        &router,
        "POST",
        "/v1/messages/m_nope/respond",
        Some(serde_json::json!({"parts": [{"text": "hi"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "not_found");
}

/// Agent → operator approval; returns the message id.
async fn approval_from(router: &axum::Router, agent: &str) -> String {
    let (status, v) = json_req(
        router,
        "POST",
        "/v1/messages",
        Some(serde_json::json!({
            "to": "me", "to_kind": "human",
            "from": agent, "from_kind": "agent",
            "kind": "approval",
            "parts": [{"text": "Allow cargo publish?"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    v["message"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn approval_respond_sends_keys_once() {
    let (router, harness, _d) = boot("approve").await;
    spawn_agent(&router, "backend").await;
    let id = approval_from(&router, "backend").await;

    // approve → keys `1` (D§8.2), not a text prompt
    let (status, v) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"approve": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["message"]["status"], "answered");
    assert_eq!(
        harness.keys(),
        vec![("backend".to_string(), vec!["1".to_string()])]
    );
    assert!(harness.prompts().is_empty());

    // a second respond (e.g. deny racing approve) conflicts; no second key
    let (status, _) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"approve": false})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(harness.keys().len(), 1);
}

#[tokio::test]
async fn approval_deny_sends_key_two() {
    let (router, harness, _d) = boot("deny").await;
    spawn_agent(&router, "backend").await;
    let id = approval_from(&router, "backend").await;
    let (status, v) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"approve": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        harness.keys(),
        vec![("backend".to_string(), vec!["2".to_string()])]
    );
}

#[tokio::test]
async fn approval_without_decision_is_invalid_and_stays_pending() {
    let (router, harness, _d) = boot("undecided").await;
    spawn_agent(&router, "backend").await;
    let id = approval_from(&router, "backend").await;
    let (status, v) = json_req(
        &router,
        "POST",
        &format!("/v1/messages/{id}/respond"),
        Some(serde_json::json!({"parts": [{"text": "sure"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"]["code"], "invalid");
    assert!(harness.keys().is_empty());
    let (_, v) = json_req(&router, "GET", "/v1/messages?to=me&status=pending", None).await;
    assert_eq!(v["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_agent_delivery_marks_message_failed() {
    let (router, harness, _d) = boot("fail").await;
    spawn_agent(&router, "backend").await;
    harness.kill("backend"); // row survives, pane gone → driver NotFound
    let (status, v) = json_req(
        &router,
        "POST",
        "/v1/messages",
        Some(serde_json::json!({"to": "backend", "parts": [{"text": "hi"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["error"]["code"], "driver");
    let (_, v) = json_req(
        &router,
        "GET",
        "/v1/messages?to=backend&status=failed",
        None,
    )
    .await;
    assert_eq!(v["messages"].as_array().unwrap().len(), 1);
}
