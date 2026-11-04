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
  at: "2026-10-31T12:00:00Z"
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
| `POST /v1/agents` | `{name, kind, workdir?, worktree?, permissions?, machine?, prompt?, task_title?, model?, args?, adopt?, resume?, continue?}` | Spawn agent in a herdr pane on `machine` (default local); returns agent. `adopt` re-attaches a `dead` row in place ([§8.4](08-harness-layer.md)); `resume`/`continue` relaunch a `dead` row's harness with its previous session |
| `POST /v1/agents/{id}/prompt` | `{text, wait?}` | `herdr agent prompt --wait`; creates `prompt` message + task if `task_title` |
| `POST /v1/agents/{id}/interrupt` | — | `ctrl+c` via send-keys |
| `POST /v1/agents/{id}/send-keys` | `{keys}` | escape hatch (power users) |
| `POST /v1/agents/{id}/stop` | `{remove?}` | stop session; keep agent row (seat). `remove` deletes the row and its worktree when clean; the response carries `worktree: {path, removed, reason?}` when the agent had one ([§9.2](09-server-modules.md)) |
| `POST /v1/messages` | `{to, kind, parts, task_id?, deadline_s?}` | unified send (any direction) |
| `POST /v1/messages/{id}/respond` | `{parts}` | answer/question or approval-response; sets `responded_at` |
| `POST /v1/messages/{id}/dismiss` | — | operator-only: close as `dismissed` without notifying the sender; sets `responded_at`; CAS on `pending` (409 otherwise) |
| `POST /v1/tasks` | `{title, description?, priority?, tags?, assign?, when_available?, not_before?, pre_run?, post_run?}` | create task; `assign: <agent>` pre-assigns (with `when_available`/`not_before`: reserves it for delivery when the agent is free), else it waits in the queue for assignment. `pre_run`/`post_run` are lifecycle prompts ([§5.2.3](05-core-objects.md)) |
| `PATCH /v1/tasks/{id}` | `{title?, description?, pre_run?, post_run?, not_before?}` | edit (operator-only); `null`/`""` clears; queued jobs take every field, delivered jobs only `post_run`, closed jobs none (409); emits `task.updated` |
| `POST /v1/tasks/{id}/assign` | `{to: <agent_id>, lease_s?, when_available?}` | assign to an agent (assign-capable principals only); atomic CAS (conflict on 0 rows); state → `assigned` |
| `POST /v1/tasks/{id}/start` | `{as: <agent_id>}` | owner declares work started; `assigned` → `working` (renews lease) |
| `POST /v1/tasks/{id}/heartbeat` | `{as: <agent_id>}` | renew lease (owner-only, 409 otherwise) |
| `POST /v1/tasks/{id}/status` | `{as: <agent_id>, state?, result?}` | owner status report; emits `task.status`; terminal states close the task |
| `POST /v1/tasks/{id}/release` | `{as: <agent_id>, reason?}` | voluntary release → back to the queue (`queued`) |
| `POST /v1/tasks/{id}/cancel` | — | cancel + interrupt owning agent |
| `POST /v1/schedules` | `{title, cron \| daily, timezone?, target?, description?, tags?, priority?, lease_s?, max_attempts?, pre_run?, post_run?}` | create a recurring job template (operator-only) |
| `PATCH /v1/schedules/{id}` | `{title?, description?, cron? \| daily?, timezone?, pre_run?, post_run?}` | edit in place; a cadence change recomputes `next_run_at` from now; later firings only; emits `schedule.updated` |
| `POST /v1/schedules/{id}/pause` · `/resume` · `/run` | — | stop firing / resume from now / fire one extra occurrence now |
| `DELETE /v1/schedules/{id}` | — | remove (open jobs it created are left alone) |

## Query (GET)

| Route | Notes |
|---|---|
| `GET /v1/agents` | list with state, machine, task summary |
| `GET /v1/agents/{id}` | full detail incl. recent output ref |
| `GET /v1/agents/{id}/read` | `?source=visible\|recent\|detection&format=text\|ansi` — proxied herdr read |
| `GET /v1/tasks` | `?state=queued&tags=&owner=&since=&mine=<agent>` queue/inventory queries (agents use `mine`) |
| `GET /v1/tasks/{id}` | task detail + message trail + assignment/lease history |
| `GET /v1/messages` | `?to=&status=&since=` inbox queries; `?agent=<name\|id>` — messages sent by or addressed to that agent (history, [D§11.1](11-tui.md)) |
| `GET /v1/machines` | inventory |
| `GET /v1/schedules` · `GET /v1/schedules/{id}` | schedules with next run; detail adds the jobs it created |
| `GET /v1/events/history` | the event log newest first, one page at a time: `?before=<seq>&limit=<n>&filter=type:...&exclude=type:...&subject=agent:<id>` (same filters as the stream). Returns `{events, head, more}`: at most `limit` events (default 50, 1–500) with `seq < before` (no `before`: from the head), `more` when older matches exist, and `head`, the log's latest seq when the page was read. A client shows the page, pages back with `before` = the oldest seq it holds, and follows live with `GET /v1/events?cursor=<head>`: nothing between the two is lost or repeated |
| `GET /healthz`, `GET /v1/schema` | health, contract |
| `GET /v1/ui/token` | the scoped read-only UI token + login path, for `tower ui` (bearer only, [D§13](13-security.md)) |

## Streaming (GET, SSE)

| Route | Semantics |
|---|---|
| `GET /v1/events` | global event bus. `?cursor=<seq>&filter=type:...&exclude=type:...&subject=agent:<id>`: `filter` keeps event types containing the text, `exclude` drops them (e.g. `exclude=type:agent.output` hides screens). `Last-Event-ID` honored; replay from cursor |
| `GET /v1/agents/{id}/stream` | output chunks as `agent.output` events (15s heartbeat comment) |
| `GET /v1/tasks/{id}/stream` | task-scoped events (A2A `SubscribeToTask` semantics) |

Backpressure: slow SSE consumers get disconnected (with a `resume` hint carrying
their cursor); clients re-request from the log. Output chunks are also appended
to artifacts storage so replay is lossless for subscribed tasks.

## Web UI (browser)

| Route | Notes |
|---|---|
| `GET /` | redirect to `/ui` |
| `GET /ui/login?token=` | valid UI token → sets the `tower_ui` cookie, redirects to `/ui` |
| `GET /ui` | the agent cloud ([D§12](12-web-ui.md)); the page's Topcoat runtime opens a WebSocket at the same URL and may `POST` re-renders (`X-Topcoat-Runtime: true`) |
| `GET /ui/assets/*` | the vendored Topcoat runtime script, served from memory |

The UI token opens only these (`GET`/`HEAD`, the WebSocket upgrade, the
runtime re-render `POST`); the bearer token opens everything.

## MCP (for agents)

`POST /mcp` — streamable HTTP MCP server exposing the same operations:

tools: `tower_ps`, `tower_spawn`, `tower_stop`, `tower_prompt`, `tower_send`,
`tower_ask`, `tower_approve`, `tower_dismiss`, `tower_inbox`, `tower_task_list`, `tower_task_show`,
`tower_task_create`, `tower_task_edit`, `tower_task_assign`, `tower_task_cancel`, `tower_task_start`,
`tower_task_heartbeat`, `tower_task_status`, `tower_task_release`,
`tower_machine_list`, `tower_schedule_list`, `tower_schedule_show`,
`tower_schedule_create`, `tower_schedule_edit`, `tower_schedule_pause`, `tower_schedule_resume`,
`tower_schedule_run`, `tower_schedule_remove`. Operator-only: assign,
`tower_stop`, `tower_task_cancel`, `tower_task_edit`, and schedule mutations. `tower_inbox`
returns the caller's messages (an agent's own, or the operator's `me`).
One management surface for humans and agents
(openrig-proven pattern). Agents have no self-serve claim: `tower_task_list
--mine` / `tower_task_show` / `tower_task_start` / `tower_task_heartbeat`
/ `tower_task_status` / `tower_task_release` are the work loop — find my
assigned job → declare start → heartbeat → report status → complete.
`tower_task_assign` is the dispatch tool, restricted to assign-capable
principals (phase 2: the operator; later the orchestrator role). Server-
side, MCP tools are the same code paths as the REST routes (one
transaction, same CAS) — there is no second implementation to drift.

Transport (phase 2): hand-rolled streamable HTTP, tools only — JSON-RPC
2.0 over `POST /mcp` answered with `application/json` (no server-initiated
stream; `GET /mcp` → 405). Methods: `initialize`, `ping`, `tools/list`,
`tools/call`. Tool failures return `isError: true` results carrying the
D§7 error envelope; unknown tools/bad params are JSON-RPC errors.

Agent identity: owner tools take `as` (agent name/id), defaulting to the
`X-Tower-Agent` request header an agent's MCP config sets. A caller that
identifies as an agent may not call `tower_task_assign` (phase-2 assign
capability = operator). Agents without MCP (e.g. pi) run the same loop
through the CLI (§10); spawn exports `TOWER_AGENT=<name>` into the
agent's pane so the CLI knows who is calling.
`tower_spawn` takes the same `model`/`args` as the REST body and applies
the same `[harness.<kind>] args` defaults.

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
