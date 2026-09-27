//! MCP endpoint tests (plan T4.1/T4.2 verify, D§7): JSON-RPC protocol,
//! the agent work loop from docs/agent-loop.md driven over MCP against
//! FakeHarness, the assign capability, and the schema-registry contract.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;

#[tokio::test]
async fn initialize_negotiates_version_and_lists_tools() {
    let ctx = common::boot().await;
    let v = ctx
        .mcp(
            None,
            "initialize",
            json!({"protocolVersion": "2025-03-26", "capabilities": {}}),
        )
        .await;
    assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    let v = ctx
        .mcp(None, "initialize", json!({"protocolVersion": "1999-01-01"}))
        .await;
    assert_eq!(
        v["result"]["protocolVersion"], "2025-06-18",
        "unknown → latest"
    );

    let v = ctx.mcp(None, "tools/list", json!({})).await;
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in [
        "tower_ps",
        "tower_spawn",
        "tower_prompt",
        "tower_send",
        "tower_ask",
        "tower_approve",
        "tower_task_list",
        "tower_task_show",
        "tower_task_create",
        "tower_task_assign",
        "tower_task_start",
        "tower_task_heartbeat",
        "tower_task_status",
        "tower_task_release",
        "tower_machine_list",
    ] {
        assert!(names.contains(&want), "missing tool {want}");
    }
    assert!(
        !names
            .iter()
            .any(|n| n.contains("claim") || n.contains("pull")),
        "no self-serve tools: {names:?}"
    );
}

#[tokio::test]
async fn protocol_errors_are_json_rpc_errors() {
    let ctx = common::boot().await;
    let v = ctx.mcp(None, "resources/list", json!({})).await;
    assert_eq!(v["error"]["code"], -32601);
    let v = ctx
        .mcp(
            None,
            "tools/call",
            json!({"name": "tower_task_claim", "arguments": {}}),
        )
        .await;
    assert_eq!(v["error"]["code"], -32602);
    let v = ctx
        .mcp(
            None,
            "tools/call",
            json!({"name": "tower_task_start", "arguments": {}}),
        )
        .await;
    assert_eq!(v["error"]["code"], -32602, "missing task_id: {v}");

    // notification (no id) → 202, no body
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ))
        .unwrap();
    let resp = tower::ServiceExt::oneshot(ctx.router.clone(), req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let (status, _) = ctx.req("POST", "/mcp", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "parse error is still a JSON-RPC reply"
    );
}

/// The docs/agent-loop.md loop, step by step, as an MCP-speaking agent.
#[tokio::test]
async fn agent_work_loop_over_mcp() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;

    // operator (no agent identity) queues and assigns
    let r = ctx
        .tool(
            None,
            "tower_task_create",
            json!({"title": "csv errors", "tags": ["rust"]}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let id = r["structuredContent"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = ctx
        .tool(
            None,
            "tower_task_assign",
            json!({"task_id": id, "to": "backend"}),
        )
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "assigned", "{r}");
    assert_eq!(
        ctx.harness.prompts().len(),
        1,
        "delegation notice delivered"
    );

    // 1. the agent finds its assigned job
    let me = Some("backend");
    let r = ctx.tool(me, "tower_task_list", json!({"mine": true})).await;
    let mine = r["structuredContent"]["tasks"].as_array().unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["id"], id.as_str());
    // 2. declares start
    let r = ctx
        .tool(me, "tower_task_start", json!({"task_id": id}))
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "working", "{r}");
    // 3. heartbeats
    let r = ctx
        .tool(me, "tower_task_heartbeat", json!({"task_id": id}))
        .await;
    assert_eq!(r["isError"], false, "{r}");
    // 4. reports the outcome
    let r = ctx
        .tool(
            me,
            "tower_task_status",
            json!({"task_id": id, "state": "completed", "result": {"summary": "done"}}),
        )
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "completed", "{r}");

    // nothing left in the agent's queue view
    let r = ctx.tool(me, "tower_task_list", json!({"mine": true})).await;
    assert!(r["structuredContent"]["tasks"]
        .as_array()
        .unwrap()
        .is_empty());
    let r = ctx
        .tool(None, "tower_task_show", json!({"task_id": id}))
        .await;
    let kinds: Vec<&str> = r["structuredContent"]["trail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "task.created",
            "task.assigned",
            "task.status",
            "task.status",
            "task.completed"
        ]
    );
}

#[tokio::test]
async fn agents_cannot_assign_or_touch_others_jobs() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let r = ctx
        .tool(None, "tower_task_create", json!({"title": "x"}))
        .await;
    let id = r["structuredContent"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // an agent can't dispatch — not even to itself
    for (tool, args) in [
        ("tower_task_assign", json!({"task_id": id, "to": "a"})),
        ("tower_task_create", json!({"title": "y", "assign": "a"})),
    ] {
        let r = ctx.tool(Some("a"), tool, args).await;
        assert_eq!(r["isError"], true, "{tool}: {r}");
        assert_eq!(r["structuredContent"]["error"]["code"], "unauthorized");
    }
    let r = ctx
        .tool(Some("a"), "tower_task_start", json!({"task_id": id}))
        .await;
    assert_eq!(r["isError"], true, "unassigned job can't be started");

    ctx.tool(None, "tower_task_assign", json!({"task_id": id, "to": "a"}))
        .await;
    let r = ctx
        .tool(
            Some("b"),
            "tower_task_status",
            json!({"task_id": id, "state": "completed"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["error"]["code"], "conflict");
    // explicit `as` overrides the header (same owner rules apply)
    let r = ctx
        .tool(
            Some("b"),
            "tower_task_start",
            json!({"task_id": id, "as": "a"}),
        )
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "working", "{r}");
}

#[tokio::test]
async fn agent_ask_reaches_the_operator_inbox() {
    let ctx = common::boot().await;
    ctx.spawn("backend", "pi").await;
    let r = ctx
        .tool(
            Some("backend"),
            "tower_ask",
            json!({"text": "sqlite or postgres?"}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let (_, v) = ctx
        .req("GET", "/v1/messages?to=me&status=pending", None)
        .await;
    let m = &v["messages"][0];
    assert_eq!(m["from_id"], "backend");
    assert_eq!(m["kind"], "question");

    let r = ctx
        .tool(
            None,
            "tower_approve",
            json!({"message_id": m["id"], "answer": "sqlite"}),
        )
        .await;
    assert_eq!(
        r["structuredContent"]["message"]["status"], "answered",
        "{r}"
    );
    assert_eq!(ctx.harness.prompts().last().unwrap().1, "sqlite");
}

/// `/v1/schema` is the introspectable contract: every route it lists must
/// be mounted (a phantom route answers with axum's bare 404/405).
#[tokio::test]
async fn schema_lists_only_mounted_routes() {
    let ctx = common::boot().await;
    let (_, v) = ctx.req("GET", "/v1/schema", None).await;
    let routes = v["routes"].as_array().unwrap();
    assert!(routes.len() > 20);
    for r in routes {
        let method = r["method"].as_str().unwrap();
        let path = r["path"].as_str().unwrap().replace("{id}", "zz_missing");
        if path == "/v1/events" {
            continue; // SSE stream never ends; mounted by construction
        }
        let req = Request::builder()
            .method(method)
            .uri(&path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(ctx.router.clone(), req)
            .await
            .unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} not mounted"
        );
        assert!(
            !(status == StatusCode::NOT_FOUND && body.is_empty()),
            "{method} {path} is listed in /v1/schema but not mounted"
        );
    }
}

// ---- operator tools the skills rely on (stop, cancel, inbox, schedule show)

#[tokio::test]
async fn only_the_operator_stops_agents_and_cancels_jobs() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    let r = ctx
        .tool(
            None,
            "tower_task_create",
            json!({"title": "x", "assign": "a"}),
        )
        .await;
    let id = r["structuredContent"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    for (tool, args) in [
        ("tower_stop", json!({"name": "a", "remove": true})),
        ("tower_task_cancel", json!({"task_id": id})),
    ] {
        let r = ctx.tool(Some("b"), tool, args).await;
        assert_eq!(r["isError"], true, "{tool}: {r}");
        assert_eq!(r["structuredContent"]["error"]["code"], "unauthorized");
    }

    // removing the owner returns its open job to the queue
    let r = ctx
        .tool(None, "tower_stop", json!({"name": "a", "remove": true}))
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let r = ctx.tool(None, "tower_ps", json!({})).await;
    let names: Vec<&str> = r["structuredContent"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["b"]);
    let r = ctx
        .tool(None, "tower_task_show", json!({"task_id": id}))
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "queued");
    assert!(r["structuredContent"]["task"]["owner_id"].is_null());

    let r = ctx
        .tool(None, "tower_task_cancel", json!({"task_id": id}))
        .await;
    assert_eq!(r["structuredContent"]["task"]["state"], "canceled", "{r}");
    let r = ctx
        .tool(None, "tower_task_cancel", json!({"task_id": id}))
        .await;
    assert_eq!(r["structuredContent"]["error"]["code"], "conflict", "{r}");
}

#[tokio::test]
async fn inbox_is_scoped_to_the_caller() {
    let ctx = common::boot().await;
    ctx.spawn("a", "pi").await;
    ctx.spawn("b", "pi").await;
    ctx.tool(Some("a"), "tower_ask", json!({"text": "which db?"}))
        .await;

    let r = ctx.tool(None, "tower_inbox", json!({})).await;
    let msgs = r["structuredContent"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1, "{r}");
    assert_eq!(msgs[0]["from_id"], "a");
    assert_eq!(msgs[0]["status"], "pending");

    let r = ctx.tool(Some("b"), "tower_inbox", json!({})).await;
    assert!(r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    let r = ctx
        .tool(None, "tower_inbox", json!({"status": "answered"}))
        .await;
    assert!(r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn schedule_show_includes_its_jobs() {
    let ctx = common::boot().await;
    let r = ctx
        .tool(
            None,
            "tower_schedule_create",
            json!({"title": "audit", "daily": "09:00", "timezone": "UTC"}),
        )
        .await;
    let sid = r["structuredContent"]["schedule"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ctx.tool(None, "tower_schedule_run", json!({"schedule_id": sid}))
        .await;
    let r = ctx
        .tool(None, "tower_schedule_show", json!({"schedule_id": sid}))
        .await;
    assert_eq!(r["structuredContent"]["schedule"]["title"], "audit", "{r}");
    let jobs = r["structuredContent"]["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["title"], "audit");

    let r = ctx
        .tool(
            None,
            "tower_schedule_show",
            json!({"schedule_id": "s_missing"}),
        )
        .await;
    assert_eq!(r["structuredContent"]["error"]["code"], "not_found", "{r}");
}
