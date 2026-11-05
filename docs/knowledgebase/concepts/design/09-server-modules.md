---
type: Concept
title: Design §9 — Server modules
description: Responsibilities of the inventory, sessions, messaging, tasks, machine hub, event bus, and workflows modules.
tags:
  - design
  - design-s9
  - server
  - modules
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §9 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
---

# 9. Server modules

## 9.1 inventory

Owns `agents` + `machines` rows. Boot sequence: ensure `local` machine row →
driver snapshot → reconcile → emit `agent.state` events for drift.
Reconcile matches a live harness to a row by pane id or name, so a harness
restarted by hand in a known pane revives its `dead` row. `adopt` on an
existing `dead` row re-attaches it in place — the row keeps its id, jobs,
and schedules, and the harness agent is renamed to the row's name
([§8.4](08-harness-layer.md)); non-`dead` rows still refuse.

## 9.2 sessions

Owns spawn/prompt/interrupt/stop flow, translating API calls to driver calls
and emitting events. One task per in-flight driver call per agent (serialized
per agent; herdr prompts wait on detection anyway). A `dead` row can be
relaunched without losing its identity: `resume` (a given session, or the
one herdr reports for the pane) and `continue` start the harness in the
row's pane — or a fresh pane if it is gone — with the per-kind resume args
([§8.4](08-harness-layer.md)); the row is updated in place.

**Worktree isolation** (`spawn --worktree`, issue #2). The worktree is
`<workdir>/.worktrees/<name>`, created *before* the harness starts, and the
harness starts in it (`AgentSpec.workdir` = the worktree; the row keeps
`workdir` = the checkout and `worktree` = the path). A new worktree starts
on a detached HEAD at the workdir's current commit. An existing worktree at
that path is reused, so a re-created agent keeps its files and branches.
If the worktree can't be created (no workdir, not a git checkout, a
non-worktree directory already at the path, a name that isn't one path
component), the spawn fails with git's message — there is no fallback to
the main checkout. `stop --remove` removes the worktree with
`git worktree remove` when it is clean; one with modified or untracked
files is kept, and the stop response says so with git's reason.
`resume`/`continue` relaunch in the worktree.

## 9.3 messaging

Unified message store + delivery. "Delivery" to a human = the message appears
in inbox queries + TUI + web UI (they poll/SSE; no push channel in v1).
"Delivery" to an agent = driver `prompt`; approval responses answer the
dialog on screen ([§8.2](08-harness-layer.md): deny = `esc`, approve = the first plain
"Yes"). A failed driver delivery marks the
message `failed`. Blocked-agent inbox items include the captured pane context
and use `[inbox].blocked_deadline_s` (default 24 hours); `tower inbox` shows
that context, and the TUI shows it in the selected item's detail. When an
agent leaves `blocked`, its tagged pending inbox item is closed as `answered`
without a prompt. The state transition and deadline notification share the
per-agent driver lock, so the sweeper rechecks the current state before acting.
For a due blocked-inbox item, only a still-blocked agent is notified; other
questions/approvals keep the general expiry behavior. The sweeper marks due
`pending` questions/approvals `expired` at `deadline_at` and emits an
`approval.expired` event: an expired question prompts the agent
("proceed with defaults or stop"); an expired approval is **denied** (keys
`esc`) — an unattended permission is never granted, and text typed into a
permission menu is undefined.

`POST /v1/messages/{id}/dismiss` (operator-only) closes a pending
question/approval as `dismissed`: no prompt or dialog goes to the sender,
`responded_at` is set, and the sweeper ignores it from then on. It is the
"disregard — another process made this stale" action: the item stays in the
store, and an agent reading its history (`GET /v1/messages?agent=<name>`)
sees `status: "dismissed"` and must not act on it. CAS on `pending` (409 if
already handled); emits a `message.status` event. The TUI inbox binds `d`
to it; the CLI verb is `tower dismiss <msg_id>`.

## 9.4 tasks

Task rows + job-queue semantics ([§5.2.1](05-core-objects.md)): assign
(atomic CAS, assign-capable principals only), start/heartbeat/status
(owner-only), lease sweeper (10s tick: expiry → requeue +
`task.leased_out`, max-attempts → `failed`). Emits `task.*` events.
Task completion is owner-reported (prompt response or detection `done`) —
never inferred from a delivered message alone (openrig's epistemics rule).
The sweeper also pauses lease expiry during `input-required` (see [§5.2](05-core-objects.md)
state machine note) and while a pre-run prompt runs. The **dispatcher** delivers reserved jobs to available
targets (on the agent's `idle`/`done` transition, when its job closes, and
each sweep); the **scheduler** fires due schedules each sweep
([§5.2.2](05-core-objects.md)). The **hooks runner** (`hooks.rs`,
[§5.2.3](05-core-objects.md)) sends a job's pre-run prompt before its
delegation and its post-run prompt after the owner closes it. Each
sequence runs in its own background task — never inline in the pump, a
route, or the sweep, which must not wait minutes on a `/compact`. Driver
calls inside it still take the per-agent lock, but only for the send; the
waits for the agent to settle hold no lock. Boot clears `hook` marks a
crash left behind before the first reconcile.

## 9.5 machine hub

Node connection manager: accepts outbound websocket upgrades at
`/nodes` (node → coordinator), authenticates per-machine tokens, multiplexes
driver calls to node agents, relays their events into the local log with
`machine_id` tagging. Node protocol: length-prefixed JSON frames over one WS
connection (upstream events; downstream RPC). Coordinator loss = nodes idle;
agents keep running (herdr); node reconnects and resyncs from snapshot.

## 9.6 event bus

Fan-out of the events table to SSE subscribers; pruning per retention config;
owns cursor semantics and heartbeats. In-process consumers follow the
same table: a consumer keeps a durable cursor in `event_consumers`, wakes
on the log's `watch` head, and registers its position with
`track_cursor` so pruning never passes it. The workflows engine is the
first one ([§9.7](#97-workflows)). Events are notifications, not the only
record: a state write and its event are separate statements, so a
consumer pairs the cursor with a reconciliation pass over the rows.

## 9.7 workflows

Owns `workflows`, `workflow_versions`, `workflow_runs`, `workflow_steps`
([§5.6](05-core-objects.md), [decision](../../decisions/workflows.md)).
`apply` parses and validates the TOML definition, stores a version when
it changed, and syncs the managed schedules of its schedule triggers.
The **engine** writes every run/step change under one async mutex
(`AppState::workflow_lock`), each a CAS on the step's state:

- **start**: write the run, then a step for each node without incoming
  edges. A job step chooses its job id, writes the row `ready`, inserts
  the job (origin `workflow`, `context_id` = run id, reserved for the
  resolved agent; a duplicate id inserts nothing), then marks it
  `running`.
- **consumer** (`workflows::spawn`): after each wake, reads events past
  its cursor: a step's job reaching a terminal state finishes the step
  and routes its edges; an event matching an enabled event trigger starts
  a run. Saves the cursor per batch.
- **reconcile** (boot and every sweeper tick): finishes running steps
  whose job is terminal, creates missing jobs for `ready` steps, ends
  quiet runs. A lost event or a crash mid-transition ends in the same
  state as the event path.
- **contract check**: `tasks::report` asks the module whether a
  `completed` report satisfies the step's result contract before it
  writes anything.

The tasks module stays unaware of graphs: a workflow job is an ordinary
reserved job with a longer description.
