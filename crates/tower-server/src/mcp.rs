//! MCP endpoint (D§7, plan T4.1): hand-rolled streamable HTTP, tools only.
//!
//! JSON-RPC 2.0 over `POST /mcp`, answered with `application/json` (no
//! server-initiated stream; `GET /mcp` → 405). Every tool is a thin wrapper
//! over the same service fn the REST route calls — one code path, same CAS.
//!
//! Identity: owner tools take `as`, defaulting to the `X-Tower-Agent`
//! header. A caller identified as an agent may not assign (phase-2 assign
//! capability = operator).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_core::{MessageId, MessageKind, MessageStatus, Part, PartyKind, TaskId, TowerError};

use crate::state::AppState;

pub const AGENT_HEADER: &str = "x-tower-agent";
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub fn router() -> Router<AppState> {
    Router::new().route("/mcp", post(handle))
}

async fn handle(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
    };
    let Some(method) = req["method"].as_str() else {
        return rpc_error(req["id"].clone(), -32600, "invalid request: missing method");
    };
    // notifications (no id) are acknowledged without a body
    if req.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let id = req["id"].clone();
    let caller = headers
        .get(AGENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    match method {
        "initialize" => {
            let asked = req["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&PROTOCOL_VERSIONS[0]);
            rpc_ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "tower", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "tower: agent fleet control. Jobs are assigned to you — never \
                        take work from the queue. Work loop: tower_task_list {mine:true} → \
                        tower_task_start → tower_task_heartbeat (every lease_s/3 s) → \
                        tower_task_status {state: completed|failed}.",
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"]["arguments"].clone();
            let args = if args.is_null() { json!({}) } else { args };
            match call_tool(&state, name, &args, caller.as_deref()).await {
                Err(ToolError::Protocol(msg)) => rpc_error(id, -32602, &msg),
                Err(ToolError::Service(e)) => {
                    let err = crate::http::to_tower_error(e);
                    let env = json!({ "error": err });
                    rpc_ok(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": env.to_string() }],
                            "structuredContent": env,
                            "isError": true,
                        }),
                    )
                }
                Ok(v) => rpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": v.to_string() }],
                        "structuredContent": v,
                        "isError": false,
                    }),
                ),
            }
        }
        other => rpc_error(id, -32601, &format!("method not found: {other}")),
    }
}

fn rpc_ok(id: Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
        .into_response()
}

enum ToolError {
    /// Unknown tool / malformed arguments → JSON-RPC error.
    Protocol(String),
    /// The operation failed → `isError` result with the D§7 envelope.
    Service(anyhow::Error),
}

impl From<anyhow::Error> for ToolError {
    fn from(e: anyhow::Error) -> Self {
        ToolError::Service(e)
    }
}

fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| ToolError::Protocol(format!("argument `{key}`: {e}"))),
    }
}

fn req_arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, ToolError> {
    arg(args, key)?.ok_or_else(|| ToolError::Protocol(format!("missing argument `{key}`")))
}

/// The acting agent: explicit `as`, else the caller header.
fn acting(args: &Value, caller: Option<&str>) -> Result<String, ToolError> {
    arg::<String>(args, "as")?
        .or_else(|| caller.map(str::to_string))
        .ok_or_else(|| {
            ToolError::Protocol("missing `as` (or X-Tower-Agent header) naming the agent".into())
        })
}

fn operator_only(caller: Option<&str>, action: &str) -> Result<(), ToolError> {
    match caller {
        Some(agent) => Err(ToolError::Service(
            TowerError::new(
                tower_core::ErrorCode::Unauthorized,
                format!("{agent} is an agent; only the operator may {action}"),
            )
            .into(),
        )),
        None => Ok(()),
    }
}

async fn call_tool(
    state: &AppState,
    name: &str,
    args: &Value,
    caller: Option<&str>,
) -> Result<Value, ToolError> {
    let now = tower_core::now_ms();
    Ok(match name {
        "tower_ps" => json!({ "agents": crate::inventory::list_agents(state).await? }),
        "tower_machine_list" => {
            json!({ "machines": crate::inventory::list_machines(state).await? })
        }
        "tower_spawn" => {
            let req: crate::sessions::SpawnRequest = serde_json::from_value(args.clone())
                .map_err(|e| ToolError::Protocol(format!("tower_spawn arguments: {e}")))?;
            json!({ "agent": crate::sessions::spawn(state, req).await? })
        }
        "tower_stop" => {
            operator_only(caller, "stop agents")?;
            let target: String = req_arg(args, "name")?;
            let remove = arg::<bool>(args, "remove")?.unwrap_or(false);
            let o = crate::sessions::stop(state, &target, remove).await?;
            let mut v = json!({ "ok": true });
            if let Some(w) = o.worktree {
                v["worktree"] = json!(w);
            }
            v
        }
        "tower_inbox" => {
            // an agent reads what was sent to it; the operator reads `me`
            let q = crate::messaging::ListQuery {
                to: Some(caller.unwrap_or("me").to_string()),
                status: Some(
                    arg::<MessageStatus>(args, "status")?.unwrap_or(MessageStatus::Pending),
                ),
                ..Default::default()
            };
            json!({ "messages": crate::messaging::list(state, &q).await? })
        }
        "tower_prompt" => {
            let target: String = req_arg(args, "name")?;
            let text: String = req_arg(args, "text")?;
            let wait = arg::<bool>(args, "wait")?.unwrap_or(false);
            let from = match caller {
                Some(c) => (tower_core::PartyKind::Agent, c),
                None => crate::sessions::OPERATOR,
            };
            let o = crate::sessions::prompt(state, &target, &text, wait, from).await?;
            json!({ "ok": true, "state": o.state, "stalled": o.stalled })
        }
        "tower_send" => {
            let body = crate::messaging::SendBody {
                to: req_arg(args, "to")?,
                to_kind: None,
                from: caller.map(str::to_string),
                from_kind: caller.map(|_| PartyKind::Agent),
                kind: arg(args, "kind")?,
                parts: vec![Part::text(req_arg::<String>(args, "text")?)],
                task_id: arg(args, "task_id")?,
                deadline_s: arg(args, "deadline_s")?,
            };
            json!({ "message": crate::messaging::send(state, body).await? })
        }
        "tower_ask" => {
            // an agent asks the operator; the operator asks a named agent
            let text: String = req_arg(args, "text")?;
            let (to, to_kind) = match caller {
                Some(_) => ("me".to_string(), PartyKind::Human),
                None => (req_arg(args, "to")?, PartyKind::Agent),
            };
            let body = crate::messaging::SendBody {
                to,
                to_kind: Some(to_kind),
                from: caller.map(str::to_string),
                from_kind: caller.map(|_| PartyKind::Agent),
                kind: Some(MessageKind::Question),
                parts: vec![Part::text(text)],
                task_id: arg(args, "task_id")?,
                deadline_s: arg(args, "deadline_s")?,
            };
            json!({ "message": crate::messaging::send(state, body).await? })
        }
        "tower_approve" => {
            let id: String = req_arg(args, "message_id")?;
            let parts = arg::<String>(args, "answer")?
                .map(|a| vec![Part::text(a)])
                .unwrap_or_default();
            let approve = arg::<bool>(args, "approve")?;
            let m = crate::messaging::respond(state, &MessageId::from(id), &parts, approve).await?;
            json!({ "message": m })
        }
        "tower_dismiss" => {
            let id: String = req_arg(args, "message_id")?;
            let m = crate::messaging::dismiss(state, &MessageId::from(id)).await?;
            json!({ "message": m })
        }
        "tower_task_list" => {
            let mine = match args.get("mine") {
                Some(Value::Bool(true)) => Some(acting(args, caller)?),
                Some(Value::String(s)) => Some(s.clone()),
                _ => None,
            };
            let f = crate::tasks::ListFilter {
                state: arg(args, "state")?,
                tags: arg::<Vec<String>>(args, "tags")?.map(|t| t.join(",")),
                owner: arg(args, "owner")?,
                mine,
                since: arg(args, "since")?,
            };
            json!({ "tasks": crate::tasks::list(state, &f).await? })
        }
        "tower_task_show" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let task = crate::tasks::get_task(state, &id)
                .await?
                .ok_or_else(|| TowerError::not_found(format!("task {id} not found")))
                .map_err(anyhow::Error::from)?;
            json!({ "task": task, "trail": crate::tasks::trail(state, &id).await? })
        }
        "tower_task_create" => {
            let req = crate::tasks::CreateTask {
                title: req_arg(args, "title")?,
                description: arg(args, "description")?,
                priority: arg(args, "priority")?,
                tags: arg(args, "tags")?.unwrap_or_default(),
                assign: arg(args, "assign")?,
                lease_s: arg(args, "lease_s")?,
                max_attempts: arg(args, "max_attempts")?,
                not_before: arg(args, "not_before")?,
                pre_run: arg(args, "pre_run")?,
                post_run: arg(args, "post_run")?,
            };
            if req.assign.is_some() {
                operator_only(caller, "assign jobs")?;
            }
            let task = crate::tasks::create(state, req, now).await?;
            crate::tasks::assignment_json(state, task).await?
        }
        "tower_task_edit" => {
            operator_only(caller, "edit jobs")?;
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let req: crate::tasks::UpdateTask = serde_json::from_value(args.clone())
                .map_err(|e| ToolError::Protocol(format!("tower_task_edit arguments: {e}")))?;
            json!({ "task": crate::tasks::update(state, &id, req, now).await? })
        }
        "tower_task_assign" => {
            operator_only(caller, "assign jobs")?;
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let to: String = req_arg(args, "to")?;
            let task =
                crate::tasks::assign(state, &id, &to, arg(args, "lease_s")?, "me", now).await?;
            crate::tasks::assignment_json(state, task).await?
        }
        "tower_task_cancel" => {
            operator_only(caller, "cancel jobs")?;
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            json!({ "task": crate::tasks::cancel(state, &id, now).await? })
        }
        "tower_task_start" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let who = acting(args, caller)?;
            json!({ "task": crate::tasks::start(state, &id, &who, now).await? })
        }
        "tower_task_heartbeat" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let who = acting(args, caller)?;
            json!({ "task": crate::tasks::heartbeat(state, &id, &who, now).await? })
        }
        "tower_task_status" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let who = acting(args, caller)?;
            let task = crate::tasks::report(
                state,
                &id,
                &who,
                arg(args, "state")?,
                arg(args, "result")?,
                now,
            )
            .await?;
            json!({ "task": task })
        }
        "tower_task_release" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let who = acting(args, caller)?;
            let reason = arg(args, "reason")?;
            json!({ "task": crate::tasks::release(state, &id, &who, reason, now).await? })
        }
        "tower_schedule_list" => json!({ "schedules": crate::schedules::list(state).await? }),
        "tower_schedule_show" => {
            let id = tower_core::ScheduleId::from(req_arg::<String>(args, "schedule_id")?);
            crate::schedules::show(state, &id).await?
        }
        "tower_schedule_create" => {
            operator_only(caller, "manage schedules")?;
            let req = crate::schedules::CreateSchedule {
                title: req_arg(args, "title")?,
                description: arg(args, "description")?,
                tags: arg(args, "tags")?.unwrap_or_default(),
                priority: arg(args, "priority")?,
                lease_s: arg(args, "lease_s")?,
                max_attempts: arg(args, "max_attempts")?,
                target: arg(args, "target")?,
                cron: arg(args, "cron")?,
                daily: arg(args, "daily")?,
                timezone: arg(args, "timezone")?,
                pre_run: arg(args, "pre_run")?,
                post_run: arg(args, "post_run")?,
            };
            json!({ "schedule": crate::schedules::create(state, req, now).await? })
        }
        "tower_schedule_edit" => {
            operator_only(caller, "manage schedules")?;
            let id = tower_core::ScheduleId::from(req_arg::<String>(args, "schedule_id")?);
            let req: crate::schedules::UpdateSchedule = serde_json::from_value(args.clone())
                .map_err(|e| ToolError::Protocol(format!("tower_schedule_edit arguments: {e}")))?;
            json!({ "schedule": crate::schedules::update(state, &id, req, now).await? })
        }
        "tower_schedule_pause"
        | "tower_schedule_resume"
        | "tower_schedule_run"
        | "tower_schedule_remove" => {
            operator_only(caller, "manage schedules")?;
            let id = tower_core::ScheduleId::from(req_arg::<String>(args, "schedule_id")?);
            match name {
                "tower_schedule_pause" => {
                    json!({ "schedule": crate::schedules::pause(state, &id, "operator", now).await? })
                }
                "tower_schedule_resume" => {
                    json!({ "schedule": crate::schedules::resume(state, &id, now).await? })
                }
                "tower_schedule_run" => {
                    json!({ "fired": crate::schedules::run_now(state, &id, now).await? })
                }
                _ => {
                    crate::schedules::remove(state, &id).await?;
                    json!({ "ok": true })
                }
            }
        }
        "tower_workflow_list" => json!({ "workflows": crate::workflows::list(state).await? }),
        "tower_workflow_show" => {
            crate::workflows::show(state, &req_arg::<String>(args, "name")?).await?
        }
        "tower_workflow_apply" => {
            operator_only(caller, "define workflows")?;
            let source: String = req_arg(args, "source")?;
            json!(crate::workflows::apply(state, &source, now).await?)
        }
        "tower_workflow_remove" => {
            operator_only(caller, "remove workflows")?;
            crate::workflows::remove(state, &req_arg::<String>(args, "name")?, now).await?;
            json!({ "ok": true })
        }
        "tower_workflow_run" => {
            let name: String = req_arg(args, "name")?;
            let input = arg::<serde_json::Map<String, Value>>(args, "input")?.unwrap_or_default();
            let trigger = json!({ "kind": "manual", "by": caller.unwrap_or("me") });
            json!({ "run": crate::workflows::start_run(state, &name, input, trigger, now).await? })
        }
        "tower_run_list" => {
            let f = crate::workflows::RunFilter {
                workflow: arg(args, "workflow")?,
                state: arg(args, "state")?,
                limit: arg(args, "limit")?,
            };
            json!({ "runs": crate::workflows::list_runs(state, &f).await? })
        }
        "tower_run_show" => {
            let id = tower_core::RunId::from(req_arg::<String>(args, "run_id")?);
            crate::workflows::show_run(state, &id).await?
        }
        "tower_run_cancel" => {
            operator_only(caller, "cancel runs")?;
            let id = tower_core::RunId::from(req_arg::<String>(args, "run_id")?);
            json!({ "run": crate::workflows::cancel_run(state, &id, now).await? })
        }
        "tower_artifact_put" => {
            let id = TaskId::from(req_arg::<String>(args, "task_id")?);
            let req = crate::artifacts::PutArtifact {
                as_agent: acting(args, caller)?,
                name: req_arg(args, "name")?,
                content: req_arg(args, "content")?,
                media_type: arg(args, "media_type")?,
            };
            json!({ "artifact": crate::artifacts::put(state, &id, req, now).await? })
        }
        "tower_artifact_get" => {
            let id = tower_core::ArtifactId::from(req_arg::<String>(args, "artifact_id")?);
            let (artifact, content) = crate::artifacts::fetch(state, &id).await?;
            json!({ "artifact": artifact, "content": content })
        }
        other => return Err(ToolError::Protocol(format!("unknown tool: {other}"))),
    })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

fn tool_list() -> Vec<Value> {
    let task_id = json!({ "type": "string", "description": "task id" });
    let as_agent = json!({
        "type": "string",
        "description": "acting agent (defaults to the X-Tower-Agent header)"
    });
    let pre_run = json!({
        "type": "string",
        "description": "lifecycle prompt typed verbatim before the job prompt, every delivery (e.g. /compact)"
    });
    let post_run = json!({
        "type": "string",
        "description": "lifecycle prompt typed verbatim after the owner reports completed/failed (e.g. /reflect)"
    });
    vec![
        tool("tower_ps", "List agents with live states.", json!({}), &[]),
        tool("tower_machine_list", "List machines.", json!({}), &[]),
        tool(
            "tower_stop",
            "Stop an agent's session (operator only). remove=true also deletes the agent: its open jobs go back to the queue and its reservations to the general queue.",
            json!({"name": {"type": "string"}, "remove": {"type": "boolean"}}),
            &["name"],
        ),
        tool(
            "tower_inbox",
            "Messages addressed to the caller (an agent) or to the operator. Default status: pending.",
            json!({"status": {"type": "string", "enum": ["pending", "delivered", "answered", "expired", "failed"]}}),
            &[],
        ),
        tool(
            "tower_spawn",
            "Spawn an agent in a herdr pane. `resume` (a session id, or
             \"\" for the one herdr reports) and `continue` relaunch a dead
             agent's harness with its previous session.",
            json!({
                "name": {"type": "string"}, "kind": {"type": "string"},
                "workdir": {"type": "string"}, "worktree": {"type": "boolean"},
                "prompt": {"type": "string"},
                "model": {"type": "string"},
                "args": {"type": "array", "items": {"type": "string"}},
                "resume": {"type": "string"},
                "continue": {"type": "boolean"},
            }),
            &["name"],
        ),
        tool(
            "tower_prompt",
            "Send a prompt to an agent.",
            json!({"name": {"type": "string"}, "text": {"type": "string"}, "wait": {"type": "boolean"}}),
            &["name", "text"],
        ),
        tool(
            "tower_send",
            "Send a message (any kind) to an agent or human.",
            json!({
                "to": {"type": "string"}, "text": {"type": "string"},
                "kind": {"type": "string", "enum": ["prompt", "question", "answer", "notice", "broadcast"]},
                "task_id": {"type": "string"}, "deadline_s": {"type": "integer"},
            }),
            &["to", "text"],
        ),
        tool(
            "tower_ask",
            "Ask a question: agents ask the operator; the operator asks a named agent (`to`).",
            json!({
                "text": {"type": "string"}, "to": {"type": "string"},
                "task_id": {"type": "string"}, "deadline_s": {"type": "integer"},
            }),
            &["text"],
        ),
        tool(
            "tower_approve",
            "Answer a pending question (`answer`) or approval (`approve`).",
            json!({
                "message_id": {"type": "string"}, "approve": {"type": "boolean"},
                "answer": {"type": "string"},
            }),
            &["message_id"],
        ),
        tool(
            "tower_dismiss",
            "Dismiss a pending question/approval: mark it stale without answering — nothing is sent to the asker.",
            json!({"message_id": {"type": "string"}}),
            &["message_id"],
        ),
        tool(
            "tower_task_list",
            "Job queue / owned jobs. `mine: true` lists your open assigned jobs.",
            json!({
                "state": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}},
                "owner": {"type": "string"}, "mine": {"type": ["boolean", "string"]},
                "since": {"type": "integer"}, "as": as_agent,
            }),
            &[],
        ),
        tool(
            "tower_task_show",
            "Job detail with its assignment/lease trail.",
            json!({"task_id": task_id}),
            &["task_id"],
        ),
        tool(
            "tower_task_create",
            "Queue a job (pre-assign is operator-only). With `assign` it is delivered when that agent is free, else reserved for it; `waiting` says why.",
            json!({
                "title": {"type": "string"}, "description": {"type": "string"},
                "priority": {"type": "integer"}, "tags": {"type": "array", "items": {"type": "string"}},
                "assign": {"type": "string"}, "lease_s": {"type": "integer"},
                "max_attempts": {"type": "integer"},
                "not_before": {"type": "integer", "description": "ms epoch; with assign: held until then"},
                "pre_run": pre_run, "post_run": post_run,
            }),
            &["title"],
        ),
        tool(
            "tower_task_edit",
            "Edit a job (operator only): a queued job takes every field, a delivered one only post_run; null or \"\" clears.",
            json!({
                "task_id": task_id, "title": {"type": "string"},
                "description": {"type": ["string", "null"]},
                "pre_run": {"type": ["string", "null"]}, "post_run": {"type": ["string", "null"]},
                "not_before": {"type": ["integer", "null"], "description": "ms epoch; reserved jobs only"},
            }),
            &["task_id"],
        ),
        tool(
            "tower_task_assign",
            "Assign a queued job to an agent (operator only): delivered when the agent is free, else reserved for it; `waiting` says why.",
            json!({
                "task_id": task_id, "to": {"type": "string"}, "lease_s": {"type": "integer"},
            }),
            &["task_id", "to"],
        ),
        tool(
            "tower_task_cancel",
            "Cancel a job (operator only); interrupts its owner if it is being worked.",
            json!({"task_id": task_id.clone()}),
            &["task_id"],
        ),
        tool(
            "tower_task_start",
            "Declare you started your assigned job (assigned → working).",
            json!({"task_id": task_id, "as": as_agent}),
            &["task_id"],
        ),
        tool(
            "tower_task_heartbeat",
            "Renew your job lease; call every lease_s/3 seconds while working.",
            json!({"task_id": task_id, "as": as_agent}),
            &["task_id"],
        ),
        tool(
            "tower_task_status",
            "Report progress or finish: working | input-required | completed | failed.",
            json!({
                "task_id": task_id, "as": as_agent,
                "state": {"type": "string", "enum": ["working", "input-required", "completed", "failed"]},
                "result": {"description": "summary / structured result"},
            }),
            &["task_id"],
        ),
        tool("tower_schedule_list", "Recurring job schedules with next run.", json!({}), &[]),
        tool(
            "tower_schedule_show",
            "One schedule with its 20 most recent jobs.",
            json!({"schedule_id": {"type": "string"}}),
            &["schedule_id"],
        ),
        tool(
            "tower_schedule_create",
            "Create a recurring job (operator only): `daily` HH:MM or `cron`, optional `target` agent.",
            json!({
                "title": {"type": "string"}, "description": {"type": "string"},
                "daily": {"type": "string", "description": "HH:MM"},
                "cron": {"type": "string"}, "timezone": {"type": "string"},
                "target": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}},
                "priority": {"type": "integer"}, "lease_s": {"type": "integer"},
                "max_attempts": {"type": "integer"},
                "pre_run": pre_run.clone(), "post_run": post_run.clone(),
            }),
            &["title"],
        ),
        tool(
            "tower_schedule_edit",
            "Edit a schedule (operator only); later firings use it. A cadence change (cron/daily/timezone) moves next run; null or \"\" clears.",
            json!({
                "schedule_id": {"type": "string"}, "title": {"type": "string"},
                "description": {"type": ["string", "null"]},
                "pre_run": {"type": ["string", "null"]}, "post_run": {"type": ["string", "null"]},
                "daily": {"type": "string", "description": "HH:MM"},
                "cron": {"type": "string"}, "timezone": {"type": "string"},
            }),
            &["schedule_id"],
        ),
        tool("tower_schedule_pause", "Stop a schedule firing (operator only).", json!({"schedule_id": {"type": "string"}}), &["schedule_id"]),
        tool("tower_schedule_resume", "Resume a schedule from now (operator only).", json!({"schedule_id": {"type": "string"}}), &["schedule_id"]),
        tool("tower_schedule_run", "Fire one extra occurrence now (operator only).", json!({"schedule_id": {"type": "string"}}), &["schedule_id"]),
        tool("tower_schedule_remove", "Delete a schedule (operator only).", json!({"schedule_id": {"type": "string"}}), &["schedule_id"]),
        tool(
            "tower_task_release",
            "Give your job back to the queue.",
            json!({"task_id": task_id, "as": as_agent, "reason": {"type": "string"}}),
            &["task_id"],
        ),
        tool(
            "tower_workflow_list",
            "Workflows with their current version, triggers, and open run count.",
            json!({}),
            &[],
        ),
        tool(
            "tower_workflow_show",
            "One workflow: definition, TOML source, schedules, and its 20 newest runs.",
            json!({"name": {"type": "string", "description": "workflow name or id"}}),
            &["name"],
        ),
        tool(
            "tower_workflow_apply",
            "Validate a TOML workflow definition and store it as a new version when it changed (operator only).",
            json!({"source": {"type": "string", "description": "the TOML definition"}}),
            &["source"],
        ),
        tool(
            "tower_workflow_remove",
            "Remove a workflow and its schedules; refused while a run is running (operator only).",
            json!({"name": {"type": "string"}}),
            &["name"],
        ),
        tool(
            "tower_workflow_run",
            "Start a run of a workflow's current version.",
            json!({
                "name": {"type": "string"},
                "input": {"type": "object", "description": "values for the workflow's [inputs]"},
            }),
            &["name"],
        ),
        tool(
            "tower_run_list",
            "Workflow runs, newest first.",
            json!({
                "workflow": {"type": "string"},
                "state": {"type": "string", "enum": ["running", "completed", "failed", "canceled"]},
                "limit": {"type": "integer"},
            }),
            &[],
        ),
        tool(
            "tower_run_show",
            "One run with its steps: node, visit, agent, state, outcome, summary, job.",
            json!({"run_id": {"type": "string"}}),
            &["run_id"],
        ),
        tool(
            "tower_run_cancel",
            "Cancel a running run and its open jobs (operator only).",
            json!({"run_id": {"type": "string"}}),
            &["run_id"],
        ),
        tool(
            "tower_artifact_put",
            "Attach text (at most 1 MiB) to your own open job: long output a later workflow step reads on demand.",
            json!({
                "task_id": task_id, "as": as_agent,
                "name": {"type": "string"}, "content": {"type": "string"},
                "media_type": {"type": "string"},
            }),
            &["task_id", "name", "content"],
        ),
        tool(
            "tower_artifact_get",
            "Read an artifact's content.",
            json!({"artifact_id": {"type": "string"}}),
            &["artifact_id"],
        ),
    ]
}
