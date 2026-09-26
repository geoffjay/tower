---
type: Concept
title: Design §5 — Core objects and state machines
description: Agent, Task (shared pool with claim/lease/heartbeat), Message, Event, and Machine objects and their state machines.
tags:
  - design
  - design-s5
  - data-model
  - state-machine
  - task-pool
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §5 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
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

A task is a unit of delegated work with an owner agent. Task states follow
A2A exactly (so the edge needs no translation):

```
queued ──claim──► working ──► input-required ──► completed
   ▲                │              └──────────► failed
   │                │◄── (answer arrives)         │
   │                └──lease expired──────────────┘
   └── cancel / reject (from any non-terminal state)
```

`queued` = unowned, pick-uppable. `working` = owned, lease active.
`input-required` = owned, blocked on a human answer (lease keeps ticking;
the sweeper pauses expiry while `input-required` so a human's slow reply
doesn't requeue work mid-question — the question message's own
`deadline_at` governs that path instead). Terminal: `completed / failed /
canceled / rejected`. Lease-expired tasks return to `queued` with an
`attempt_count` bumped (see §5.2.1); a max-attempts guard (default 3)
routes exhausted tasks to `failed` instead of infinite requeue.

Mapping: agent state changes emit `task-status` events
(`working` ↔ `working`, `blocked` ↔ `input-required`, terminal states map
directly). A task may also be created without an agent (queued work).

## 5.2.1 Shared task pool and ownership

Tasks are pick-uppable from a shared list. Ownership is exclusive and
time-bound:

- **`owner` (nullable)**: the agent that currently owns the task. `NULL` means
  unowned and pick-uppable.
- **`lease_expires_at`**: ownership expires if not renewed. Owner must
  heartbeat via `POST /v1/tasks/{id}/heartbeat` (default window: 60s lease,
  heartbeat at 20-30s; configurable per task and globally).
- **Claim protocol**: `POST /v1/tasks/{id}/claim` — a single transaction:
  `UPDATE tasks SET owner = :agent, state='working', lease_expires_at = now
  + lease WHERE id = :id AND (owner IS NULL OR lease_expires_at < now)`.
  Affected-rows == 1 → claimed; 0 → `conflict` error returned to the losing
  agent. No coordinator arbitration needed — SQLite row state is the lock
  (the single-writer SQLite design from [§6](06-data-model.md) makes the check-and-set atomic).
- **Lease expiry sweeper** (part of the tasks module, runs every ~10s):
  expired leases → `owner = NULL`, `state = 'queued'`, `attempt_count =
  attempt_count + 1`, emit `task.leased_out` event. The work returns to the
  pool; another agent (or the same one after its crash) can re-claim it.
  When `attempt_count` reaches `max_attempts` (default 3), the sweeper moves
  the task to `failed` with a `lease_exhausted` result instead of requeueing
  (no infinite crash-loop churn).
- **Completion**: only the owner may move the task to a terminal state;
  non-owner terminal writes return `conflict`.
- **Priority/queueing**: `priority INTEGER` (default 0, higher = sooner);
  pool queries (`GET /v1/tasks?state=queued`) order by `priority DESC,
  created_at ASC`. Agents may pull the next matching task atomically:
  `POST /v1/tasks/pull {tags?, capacity: 1}` claims the highest-priority
  unowned task matching a filter — same CAS semantics as claim.
- **Tags**: `tags TEXT` (JSON array) on tasks for routing/filtering
  (`["backend", "rust"]`); pull filters on tags. Pods/teams from openrig map
  to tag filters, not new tables.
- **Reclaim, not orphan**: lease expiry is the crash story — an agent that
  dies mid-task has its task auto-requeued within one lease window. There is
  no "stuck forever" state; `agent.state=dead` + expired lease = clean pickup
  by a survivor.
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
| `task.claimed` | task_id, owner_id, lease_expires_at |
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
