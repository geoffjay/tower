---
type: Concept
title: Design §7 — Server API
description: REST control/query routes, SSE streams, MCP tools, the A2A edge, and auth conventions.
tags:
  - design
  - design-s7
  - api
  - sse
  - mcp
  - a2a
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §7 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 7. Server API

REST-style JSON over one port. JSON-RPC appears only at the MCP and A2A
boundaries (decision: REST internally for debuggability with curl; the
research protocols where they are standards).

Conventions:

- Errors: `{"error": {"code": "not_found|conflict|timeout|driver|invalid",
  "message": "...", "detail": {...}}}` with proper HTTP status
- All timestamps ms epoch; all ids ULIDs
- `GET /v1/schema` returns the route + event-type registry (herdr's
  `api schema` idea — the contract is introspectable)

## Control (POST)

| Route | Body | Effect |
|---|---|---|
| `POST /v1/agents` | `{name, kind, workdir?, worktree?, permissions?, machine?, prompt?, task_title?}` | Spawn agent in a herdr pane on `machine` (default local); returns agent |
| `POST /v1/agents/{id}/prompt` | `{text, wait?}` | `herdr agent prompt --wait`; creates `prompt` message + task if `task_title` |
| `POST /v1/agents/{id}/interrupt` | — | `ctrl+c` via send-keys |
| `POST /v1/agents/{id}/send-keys` | `{keys}` | escape hatch (power users) |
| `POST /v1/agents/{id}/stop` | `{remove?}` | stop session; keep agent row (seat) |
| `POST /v1/messages` | `{to, kind, parts, task_id?, deadline_s?}` | unified send (any direction) |
| `POST /v1/messages/{id}/respond` | `{parts}` | answer/question or approval-response; sets `responded_at` |
| `POST /v1/tasks` | `{title, description?, priority?, tags?, assign?}` | create task; `assign: <agent>` pre-assigns, else it waits in the queue for assignment |
| `POST /v1/tasks/{id}/assign` | `{to: <agent_id>, lease_s?}` | assign to an agent (assign-capable principals only); atomic CAS (conflict on 0 rows); state → `assigned` |
| `POST /v1/tasks/{id}/start` | `{as: <agent_id>}` | owner declares work started; `assigned` → `working` (renews lease) |
| `POST /v1/tasks/{id}/heartbeat` | `{as: <agent_id>}` | renew lease (owner-only, 409 otherwise) |
| `POST /v1/tasks/{id}/status` | `{as: <agent_id>, state?, result?}` | owner status report; emits `task.status`; terminal states close the task |
| `POST /v1/tasks/{id}/release` | `{as: <agent_id>, reason?}` | voluntary release → back to the queue (`queued`) |
| `POST /v1/tasks/{id}/cancel` | — | cancel + interrupt owning agent |

## Query (GET)

| Route | Notes |
|---|---|
| `GET /v1/agents` | list with state, machine, task summary |
| `GET /v1/agents/{id}` | full detail incl. recent output ref |
| `GET /v1/agents/{id}/read` | `?source=visible\|recent\|detection&format=text\|ansi` — proxied herdr read |
| `GET /v1/tasks` | `?state=queued&tags=&owner=&since=&mine=<agent>` queue/inventory queries (agents use `mine`) |
| `GET /v1/tasks/{id}` | task detail + message trail + assignment/lease history |
| `GET /v1/messages` | `?to=&status=&since=` inbox queries |
| `GET /v1/machines` | inventory |
| `GET /healthz`, `GET /v1/schema` | health, contract |

## Streaming (GET, SSE)

| Route | Semantics |
|---|---|
| `GET /v1/events` | global event bus. `?cursor=<seq>&filter=type:...&subject=agent:<id>`. `Last-Event-ID` honored; replay from cursor |
| `GET /v1/agents/{id}/stream` | output chunks as `agent.output` events (15s heartbeat comment) |
| `GET /v1/tasks/{id}/stream` | task-scoped events (A2A `SubscribeToTask` semantics) |

Backpressure: slow SSE consumers get disconnected (with a `resume` hint carrying
their cursor); clients re-request from the log. Output chunks are also appended
to artifacts storage so replay is lossless for subscribed tasks.

## MCP (for agents)

`POST /mcp` — streamable HTTP MCP server exposing the same operations:

tools: `tower_ps`, `tower_spawn`, `tower_prompt`, `tower_send`,
`tower_ask`, `tower_approve`, `tower_task_list`, `tower_task_show`,
`tower_task_create`, `tower_task_assign`, `tower_task_start`,
`tower_task_heartbeat`, `tower_task_status`, `tower_task_release`,
`tower_machine_list`. One management surface for humans and agents
(openrig-proven pattern). Agents have no self-serve claim: `tower_task_list
--mine` / `tower_task_show` / `tower_task_start` / `tower_task_heartbeat`
/ `tower_task_status` / `tower_task_release` are the work loop — find my
assigned job → declare start → heartbeat → report status → complete.
`tower_task_assign` is the dispatch tool, restricted to assign-capable
principals (phase 2: the operator; later the orchestrator role). Server-
side, MCP tools are the same code paths as the REST routes (one
transaction, same CAS) — there is no second implementation to drift.

## A2A (edge)

- `GET /.well-known/agent-card.json` — card: skills derived from agent roster
  (one skill per named agent), `capabilities: {streaming: true}`.
- `POST /a2a` — JSON-RPC 2.0: `message/send` and `message/stream`.
  Inbound message → `prompt` message + task (origin `a2a`); outbound replies and
  task events use A2A shapes (already native, per [§5](05-core-objects.md)).
  Push notifications (`pushNotifications`) deferred (phase 6+).

Auth: all `/a2a` and `/v1` TCP access requires `Authorization: Bearer <token>`
(unix socket access is exempt — filesystem permissions are the auth).
Read-only UI routes accept the same token.
