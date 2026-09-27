//! Shared integration-test harness: temp db + FakeHarness + full router.
#![allow(dead_code)] // each test binary uses a subset

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower_driver::fake::FakeHarness;
use tower_server::{AppState, Config, Paths};

pub struct Ctx {
    pub state: AppState,
    pub harness: FakeHarness,
    pub router: axum::Router,
    _dir: tempfile::TempDir,
}

pub async fn boot() -> Ctx {
    let dir = tempfile::Builder::new()
        .prefix("tower-it")
        .tempdir()
        .unwrap();
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
    let router = tower_server::serve::router(state.clone());
    Ctx {
        state,
        harness,
        router,
        _dir: dir,
    }
}

impl Ctx {
    pub async fn req(
        &self,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let builder = Request::builder().method(method).uri(uri);
        let req = match body {
            Some(v) => builder
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = tower::ServiceExt::oneshot(self.router.clone(), req)
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

    /// One MCP JSON-RPC call; `agent` sets the X-Tower-Agent identity.
    pub async fn mcp(
        &self,
        agent: Option<&str>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json");
        if let Some(a) = agent {
            b = b.header("x-tower-agent", a);
        }
        let body =
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let resp = tower::ServiceExt::oneshot(
            self.router.clone(),
            b.body(Body::from(body.to_string())).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// `tools/call` → the tool result object (`content`, `structuredContent`, `isError`).
    pub async fn tool(
        &self,
        agent: Option<&str>,
        name: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        let v = self
            .mcp(
                agent,
                "tools/call",
                serde_json::json!({"name": name, "arguments": args}),
            )
            .await;
        assert!(
            v.get("error").is_none(),
            "protocol error calling {name}: {v}"
        );
        v["result"].clone()
    }

    /// Spawn an agent row + fake pane.
    pub async fn spawn(&self, name: &str, kind: &str) {
        tower_server::sessions::spawn(
            &self.state,
            tower_server::sessions::SpawnRequest {
                name: name.into(),
                kind: Some(kind.into()),
                workdir: Some("/tmp".into()),
                worktree: false,
                adopt: false,
                prompt: None,
                permissions: None,
            },
        )
        .await
        .unwrap();
    }

    /// Agent → operator question/approval; returns the message id.
    pub async fn ask_operator(&self, agent: &str, kind: &str, deadline_s: i64) -> String {
        let (status, v) = self
            .req(
                "POST",
                "/v1/messages",
                Some(serde_json::json!({
                    "to": "me", "to_kind": "human",
                    "from": agent, "from_kind": "agent",
                    "kind": kind,
                    "parts": [{"text": "need input"}],
                    "deadline_s": deadline_s,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        v["message"]["id"].as_str().unwrap().to_string()
    }

    pub async fn deadline_of(&self, id: &str) -> i64 {
        sqlx::query_scalar("SELECT deadline_at FROM messages WHERE id=?1")
            .bind(id)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    pub async fn status_of(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM messages WHERE id=?1")
            .bind(id)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    pub async fn event_count(&self, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type=?1")
            .bind(kind)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }
}
