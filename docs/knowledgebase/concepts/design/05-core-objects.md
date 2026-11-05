---
type: Concept
title: Design §5 — Core objects and state machines
description: Agent, Task (job queue with assignment, lease, heartbeat, lifecycle prompts, editing), Message, Event, and Machine objects and their state machines.
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
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
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
running, agent has not yet declared work started (while a pre-run prompt
runs, lease expiry pauses and the delegation has not gone out —
[§5.2.3](#523-lifecycle-prompts-and-editing)). `working` = owner
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
  lease_s?}`. An expired lease on the task is first swept exactly as the
  lease sweeper would (requeue + attempt bump, or `failed` when
  exhausted), then: `UPDATE tasks SET owner = :agent, state='assigned',
  lease_expires_at = now + lease WHERE id = :id AND owner IS NULL AND
  state = 'queued'`. That is the `(owner IS NULL OR lease_expires_at <
  now)` check with one code path for expiry accounting — an exhausted job
  is never reassigned and a paused `input-required` lease is never
  stolen. The same `UPDATE` also requires that the agent owns **no other
  open job** (one job per agent at a time — [§5.2.2](#522-reservations-and-schedules)).
  Affected-rows == 1 → assigned; 0 → `conflict`. A job with a
  live owner can never be assigned to another agent. No coordinator
  arbitration needed — SQLite row state is the lock (the single-writer
  SQLite design from [§6](06-data-model.md) makes the check-and-set
  atomic).
- **Assignment notice**: a successful assign sends the owner a
  `delegation` message (task id, title, description, the work-loop
  reminder) — to-agent delivery is a prompt, so an idle agent learns it
  has work. A job with a pre-run prompt sends that first and the
  delegation only after it settles ([§5.2.3](#523-lifecycle-prompts-and-editing)).
  A failed notice does not undo the assignment; the lease
  expires if the agent never starts.
- **Start declaration**: the owner declares work started with
  `POST /v1/tasks/{id}/start` (or `status {state: working}`), →
  `working`. Until then, `assigned` + no heartbeat is the visible stall
  signal.
- **`lease_expires_at`**: ownership expires if not renewed. Owner must
  heartbeat via `POST /v1/tasks/{id}/heartbeat`; every renewal extends by
  the task's `lease_s` (default 60s, set at create/assign; heartbeat at
  20-30s).
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
  clean reassignment by the operator (or orchestrator). Removing an agent
  (`stop --remove`) stops its pane first, then releases its open jobs
  back to the queue at once (no attempt bump — operator action) and
  detaches its finished ones; their history stays in the event log.
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

## 5.2.2 Reservations and schedules

Decision: [scheduled jobs](../../decisions/scheduled-jobs.md) — the policy
table there is normative.

- **Reserved delivery**: a queued job may carry `target_agent_id`. It stays
  `queued`; the **dispatcher** assigns it (normal CAS, lease starts at
  delivery) when the target is *available* — row state `idle`/`done` and no
  open job. Triggers: the target's row entering `idle`/`done`, its open job
  closing, and the ~10s sweep. Highest priority, then oldest, first.
  `task.assigned` records `by: "schedule:<id>"` for schedule jobs, else
  `"dispatch"`.
- **One job per agent**: every assignment (manual or dispatched) is refused
  with `conflict` while the agent owns an open job, or one of its closed
  jobs still runs its post-run prompt ([§5.2.3](#523-lifecycle-prompts-and-editing));
  the check lives in the assignment `UPDATE`, so it is atomic.
- **`not_before`** holds a reserved job back from the dispatcher until a
  time (`task create --at`). Immediate manual `assign` ignores it and may
  take a reserved job for any agent — explicit operator action wins.
- **Schedules**: a job template (title, description, lifecycle prompts) +
  cron expression + IANA timezone. Each
  firing creates an ordinary job (`origin='schedule'`, `schedule_id`,
  `occurrence_at`), reserved for the schedule's target if it has one.
  Firing is exactly-once: CAS on `next_run_at` plus a unique
  `(schedule_id, occurrence_at)`. Overlap → skip; undelivered previous
  occurrence → canceled (`occurrence_expired`); missed firings → coalesced
  into one; target removed → schedule paused, reserved jobs fall back to
  the general queue. Schedules are operator-only and editable in place
  ([§5.2.3](#523-lifecycle-prompts-and-editing)).
- **Workflow schedules**: a schedule with `workflow_id` belongs to a
  workflow's schedule trigger ([§5.6](#56-workflow-run-step)). Each firing
  starts a run instead of creating a job; it skips (`previous_running`)
  while the run it started last is still `running`. `apply` creates,
  updates, and deletes these rows; edit and remove answer `conflict`;
  pause, resume, and run work as for any schedule.

## 5.2.3 Lifecycle prompts and editing

Decision: [job lifecycle prompts](../../decisions/job-lifecycle-hooks.md).

- **`pre_run` / `post_run`**: optional prompts on a job; a schedule's pair
  is copied into every job it fires. Each is typed into the harness
  verbatim as its own prompt, so a slash command (`/compact`, `/clear`,
  `/reflect`) stays a command. Blank text is stored as none.
- **Pre-run, every delivery**: assigning a job that has `pre_run` sets
  `hook = 'pre'` in the assignment `UPDATE`. A background runner waits for
  the agent to be `idle`/`done`, sends the pre-run prompt, waits up to 5 s
  to see `working`/`blocked` (an instant command may never show it), then
  waits for `idle`/`done`. It then clears `hook`, renews the lease in full,
  and sends the delegation — the job prompt never overtakes the pre-run
  prompt. Lease expiry pauses while `hook = 'pre'`. A job requeued and
  delivered again runs its pre-run prompt again.
- **Post-run, after the owner closes the job**: a terminal owner report
  (`completed`/`failed`) sets `hook = 'post'` in the closing `UPDATE` when
  `post_run` is set. The runner waits for `idle`/`done`, sends the prompt,
  waits for it to settle, clears `hook`, and runs the dispatcher for the
  agent. Cancel, lease expiry, and release never run it.
- **Busy until it settles**: an agent with a `hook = 'post'` job is busy
  (§5.2.2) — manual assign answers `conflict`, the dispatcher waits.
- **Never blocking**: a send error, or no settle within
  `[jobs] hook_timeout_s` ([§4](04-paths-and-configuration.md), default
  600), records the failure and the sequence goes on (the delegation still
  goes out; the agent is released).
- **Records**: `task.hook {task_id, phase: pre|post, status:
  running|done|failed|interrupted, error?}`; each sent prompt is a `prompt`
  message from `me` with the job's `task_id` (job detail, agent history).
- **Restart**: boot clears every `hook` (`status: interrupted`) before
  reconcile. A `pre` job's lease then runs out as usual and requeues it.
- **Editing a job** (`PATCH /v1/tasks/{id}`): a `queued` job takes
  `title`, `description`, `pre_run`, `post_run`, `not_before` (only with a
  reservation; a new time re-runs dispatch for the target). A delivered
  job (`assigned`/`working`/`input-required`) takes `post_run` only; a
  closed job nothing (`conflict`). `null` or `""` clears an optional field.
  The `UPDATE` is a CAS on the state it was checked against; emits
  `task.updated {task_id, fields}`.
- **Editing a schedule** (`PATCH /v1/schedules/{id}`): `title`,
  `description`, `pre_run`, `post_run`, cadence (`cron` | `daily`,
  `timezone`). A cadence change recomputes `next_run_at` from now (no
  catch-up). Only later firings see the edit; an open occurrence keeps its
  copy. Emits `schedule.updated {schedule_id, fields, next_run_at}`.
- Edits are operator-only, like assignment (MCP refuses agent callers).

## 5.3 Message

One row per communication act, any direction:

- `from`: human (operator id, `me`), agent (agent id), service, external (A2A)
- `to`: human, agent, room (named group), external
- `kind`: `prompt | question | answer | approval | approval-response |
  notice | delegation | broadcast`
- `parts`: A2A Part array — `[{text:...} | {data:...} | {raw:...} | {url:...},
  media_type?, filename?, metadata?]`
- `status`: `pending | delivered | answered | dismissed | expired | failed`
- `deadline_at`: questions/approvals expire (default 5 min, configurable);
  blocked-agent inbox items use `[inbox].blocked_deadline_s` (default 24 h)

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
| `task.updated` | task_id, changed field names ([§5.2.3](#523-lifecycle-prompts-and-editing)) |
| `task.hook` | task_id, owner_id, phase (`pre`/`post`), status (`running`/`done`/`failed`/`interrupted`), error |
| `schedule.updated` | schedule_id, changed field names, next_run_at |
| `workflow.applied` / `removed` / `enabled` / `disabled` | workflow_id, name, version ([§5.6](#56-workflow-run-step)) |
| `workflow.run.started` / `completed` / `failed` / `canceled` | run_id, workflow, trigger, error |
| `workflow.step.started` / `completed` / `failed` / `canceled` | run_id, step_id, node, visit, task_id, outcome, error |
| `artifact.created` | task_id, artifact_id, name, size (subject: the task) |
| `message.created` / `message.status` | message summary, status transitions |
| `approval.expired` | message_id, agent_id |
| `machine.state` | machine_id, status |
| `node.registered` / `node.disconnected` | machine summary |

Envelope: `{seq, ts, type, subject_type, subject_id, payload}` where `seq` is
the global monotonic cursor.

## 5.5 Machine

A registered host. `role: coordinator | node`, `status: online | offline |
degraded`, `last_seen_at`. Local machine is always present (`local`).

## 5.6 Workflow, run, step

Decision: [workflows](../../decisions/workflows.md) — its §3–§6 are
normative for routing, hand-off, triggers, and the engine.

- **Workflow**: a named graph of `job` nodes and edges, stored in
  versions (`tower workflow apply <file.toml>` adds one when the
  definition changed). Enabled or disabled. Operator-defined only.
- **Run**: one execution of one version, with an `input` object and its
  trigger (manual, schedule, event). A run's id is the `context_id` of
  every job it creates (A2A context grouping).

```
RunState:  running ──► completed
              │  └───► failed     (a step failed with no `on = "failed"` edge)
              └──────► canceled   (operator)
```

- **Step**: one execution of one node; a loop visit is a new step
  (`visit` + 1, at most the node's `max_visits`). A job step owns exactly
  one job (origin `workflow`), reserved for the node's agent — static, or
  mapped from an earlier node's outcome.

```
StepState:  pending ──► ready ──► running ──► completed
            (join=all       │          └────► failed
             collecting)    └───────────────► failed   (no agent, max_visits)
            any open state ─────────────────► canceled  (run failed or canceled)
```

- **Routing**: an edge fires on the source step's declared outcome,
  `completed` (default), or `failed`. `join = "any"` (default) starts a
  step per fired edge; `join = "all"` collects in a `pending` step and
  runs when every incoming edge fired — or when the run has nothing else
  `ready`/`running`, with the edges that did fire.
- **Result contract** (workflow jobs only): a `completed` report carries
  `{summary ≤ 2000 chars, outcome ∈ node outcomes, data ≤ 16 KB}`;
  otherwise 400 `invalid` with `detail.reason`. A `failed` report needs
  no summary.
- **Hand-off**: the job description is the node prompt plus the run
  input, the context steps' outcome, summary, and artifact ids, the
  node's own earlier visits, and the contract. Artifacts (text ≤ 1 MiB,
  attached by the job's owner while open) are read on demand.
