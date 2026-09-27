---
type: Concept
title: Design §5 — Core objects and state machines
description: Agent, Task (job queue with assignment, lease, heartbeat), Message, Event, and Machine objects and their state machines.
tags:
  - design
  - design-s5
  - data-model
  - state-machine
  - job-queue
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §5 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5.5
  at: "2026-09-26T22:59:25Z"
---

# 5. Core objects and state machines

## 5.1 Agent

An agent is a named, durable identity bound to a harness instance in a herdr
pane. The process may come and go; the agent row persists (openrig's "seat").

```
AgentState:
  unknown ──► launching ──► idle ──► working ──► done
                  │           │  ▲       │
                  │           ▼  │       ▼
                  └────────► blocked ──► (idle, after response)
                  │
                  └─────────────────────► dead   (pane/process gone)
```

States come from herdr detection manifests (`idle/working/blocked/done`,
plus our own `launching/dead/unknown`). `blocked` is the human-attention
signal: the messaging module turns it into a question/approval inbox item.

## 5.2 Task

A task is a unit of delegated work owned by at most one agent at a time.
Task states stay A2A-shaped (so the edge needs no translation), with one
addition over A2A: an explicit **`assigned`** state between queued and
working (decision: [job queue](../../decisions/job-queue.md)):

```
queued ──assign──► assigned ──start──► working ──► input-required ──► completed
   ▲                  │                     │              └──────────► failed
   │                  │                     │◄── (answer arrives)
   │                  └──lease expired──────┼──────────────────────────┘
   └── cancel / reject (from any non-terminal state)
```

`queued` = unowned, awaiting assignment. No agent can take a job from the
queue itself — jobs are assigned by a principal with the assign capability
(phase 2: the operator; later: an orchestrator agent with a router — see
[§5.2.1](#521-job-queue-and-ownership)). `assigned` = owner set, lease
running, agent has not yet declared work started. `working` = owner
declared start (or detection saw `working`). `input-required` = owned,
blocked on a human answer (lease keeps ticking; the sweeper pauses expiry
while `input-required` so a human's slow reply doesn't requeue work
mid-question — the question message's own `deadline_at` governs that path
instead). Terminal: `completed / failed / canceled / rejected`.
Lease-expired tasks return to `queued` with an `attempt_count` bumped
(see §5.2.1); a max-attempts guard (default 3) routes exhausted tasks to
`failed` instead of infinite requeue.

Mapping: agent state changes emit `task-status` events
(`working` ↔ `working`, `blocked` ↔ `input-required`, terminal states map
directly). A task may also be created unassigned (queued work).

## 5.2.1 Job queue and ownership

Tasks form a job queue, not a self-serve pool (decision:
[job queue](../../decisions/job-queue.md)). Ownership is exclusive,
assignment-only, and time-bound:

- **`owner` (nullable)**: the agent that currently owns the task. `NULL` means
  unowned, awaiting assignment.
- **Assign capability**: only a principal with the assign capability may
  set `owner`. Phase 2: the operator (CLI/MCP). Later: an orchestrator
  agent (`role: orchestrator`) that reads the queue and assigns via an
  agent router (a decision model — candidates: jev or laya, both with
  Rust libraries; laya can run locally as a GGUF via ollama/llama.cpp).
  Router choice is deliberately deferred ([decision](../../decisions/job-queue.md)).
- **Assignment protocol**: `POST /v1/tasks/{id}/assign {to: <agent>,
  lease_s?}` — a single transaction: `UPDATE tasks SET owner = :agent,
  state='assigned', lease_expires_at = now + lease WHERE id = :id AND
  (owner IS NULL OR lease_expires_at < now)`. Affected-rows == 1 →
  assigned; 0 → `conflict`. A job with a live owner can never be assigned
  to another agent. No coordinator arbitration needed — SQLite row state
  is the lock (the single-writer SQLite design from [§6](06-data-model.md)
  makes the check-and-set atomic).
- **Start declaration**: the owner declares work started with
  `POST /v1/tasks/{id}/status {state: working}` (or `task start`), →
  `working`. Until then, `assigned` + no heartbeat is the visible stall
  signal.
- **`lease_expires_at`**: ownership expires if not renewed. Owner must
  heartbeat via `POST /v1/tasks/{id}/heartbeat` (default window: 60s
  lease, heartbeat at 20-30s; configurable per task and globally).
- **Lease expiry sweeper** (part of the tasks module, runs every ~10s):
  expired leases → `owner = NULL`, `state = 'queued'`, `attempt_count =
  attempt_count + 1`, emit `task.leased_out` event. The job returns to
  the queue for reassignment. When `attempt_count` reaches
  `max_attempts` (default 3), the sweeper moves the task to `failed`
  with a `lease_exhausted` result instead of requeueing (no infinite
  crash-loop churn).
- **Completion**: only the owner may move the task to a terminal state;
  non-owner terminal writes return `conflict`.
- **Priority/queueing**: `priority INTEGER` (default 0, higher = sooner);
  queue queries (`GET /v1/tasks?state=queued`) order by `priority DESC,
  created_at ASC` — ranking for the assigner, not a pull menu.
- **Tags**: `tags TEXT` (JSON array) on tasks for routing/filtering
  (`["backend", "rust"]`); the assigner filters on tags (the future
  router's input signal). Pods/teams from openrig map to tag filters,
  not new tables.
- **Reassign, not orphan**: lease expiry is the crash story — an agent
  that dies mid-task has its task auto-requeued within one lease window.
  There is no "stuck forever" state; `agent.state=dead` + expired lease =
  clean reassignment by the operator (or orchestrator).
- **Orchestrator liveness checks**: an orchestrator may ask a
  `working`/`assigned` owner "still working on it?" — but rate-limited,
  and mostly unnecessary: the lease already answers the question
  mechanically. Heartbeats piggyback on the agent loop.
- **Status reporting**: owners report progress with
  `POST /v1/tasks/{id}/status` (state transitions + `result` updates); every
  transition emits `task.status` events so the UI and other agents watch
  without polling the row.

Why lease + heartbeat instead of session-bound ownership: agentd/hermes
kanban's PID-liveness checks fail across machines (cross-host PIDs aren't
comparable, and herdr-panes outlive client sessions). A lease is machine-
agnostic: heartbeat renewal is just another event write, and expiry is a
simple timestamp comparison. Heartbeats piggyback on the agent loop (MCP tool
call between prompt turns) — see the MCP tools in [§7](07-server-api.md).

## 5.3 Message

One row per communication act, any direction:

- `from`: human (operator id, `me`), agent (agent id), service, external (A2A)
- `to`: human, agent, room (named group), external
- `kind`: `prompt | question | answer | approval | approval-response |
  notice | delegation | broadcast`
- `parts`: A2A Part array — `[{text:...} | {data:...} | {raw:...} | {url:...},
  media_type?, filename?, metadata?]`
- `status`: `pending | delivered | answered | expired | failed`
- `deadline_at`: questions/approvals expire (default 5 min, configurable)

## 5.4 Event

Append-only log row. Every state change anywhere is one event. Event types:

| type | payload (summary) |
|---|---|
| `server.started` | version, pid |
| `agent.created` / `agent.removed` | agent summary |
| `agent.state` | agent_id, from, to, detection detail |
| `agent.output` | agent_id, chunk (cursor into artifacts) |
| `task.created` / `task.status` | task_id, state, result ref |
| `task.assigned` | task_id, owner_id, lease_expires_at |
| `task.leased_out` | task_id, prior owner, state → queued |
| `task.completed` / `task.failed` | task_id, owner_id, result ref |
| `message.created` / `message.status` | message summary, status transitions |
| `approval.expired` | message_id, agent_id |
| `machine.state` | machine_id, status |
| `node.registered` / `node.disconnected` | machine summary |

Envelope: `{seq, ts, type, subject_type, subject_id, payload}` where `seq` is
the global monotonic cursor.

## 5.5 Machine

A registered host. `role: coordinator | node`, `status: online | offline |
degraded`, `last_seen_at`. Local machine is always present (`local`).
