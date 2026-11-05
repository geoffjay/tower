---
type: Decision
title: Workflows — graphs of jobs, routed by outcomes, driven by the event log
description: "Accepted: a workflow is a versioned graph of job nodes. Edges route on a node's reported outcome, with fan-out, join, and bounded loops. Each step hands a bounded summary (and its artifacts) to the next. Runs start manually, from a schedule, or from an event. An in-process engine follows the events table from a durable cursor. A2A is not needed inside one deployment. Exec nodes and on-demand agents are follow-ups."
tags:
  - decision
  - workflows
  - job-queue
  - events
  - a2a
status: accepted
generated:
  by: omp/claude-opus-5-5
  at: "2026-11-04T12:00:00Z"
sources:
  - resource: crates/tower-server/src/storage.rs
    title: EventLog (append, watch head, cursors, prune)
  - resource: crates/tower-server/src/tasks.rs
    title: report, notify_assignee, dispatch_for, sweep_leases
  - resource: crates/tower-server/src/sweeper.rs
    title: 10 s tick (deadlines, leases, schedules, dispatch)
  - resource: crates/tower-server/src/schedules.rs
    title: cron firing, exactly-once occurrences
---

# Workflows

Date: 2026-11-04. Status: **accepted** (operator answers in
[§11](#11-operator-decisions-2026-11-04)). Amends
[§5](../concepts/design/05-core-objects.md), [§6](../concepts/design/06-data-model.md),
[§7](../concepts/design/07-server-api.md), [§9](../concepts/design/09-server-modules.md),
[§10](../concepts/design/10-client-cli.md), [§18](../concepts/design/18-phase-mapping.md),
and item 8 of the [outcomes-and-brief decision](outcomes-and-brief.md). Plan:
[phase 7](../plans/phase-7-workflows.md).

## Context

The operator wants multi-step work that tower drives by itself. Two
examples set the scope:

1. **Issue to done.** The operator starts a run with an issue id. A triage
   agent decides "build" or "design". The matching worker agent gets a job
   that carries the triage result. A reviewer gets a job. If the reviewer
   asks for changes, the worker runs again with the review. If the reviewer
   approves, the run ends.
2. **Daily digest.** A schedule fires. A Slack-gathering agent and a
   Linear-gathering agent run in parallel. When both finish, a third agent
   gets both results and writes them to the knowledge base.

What tower had before this decision (2026-11-04, code at migration 0006):

- **Jobs** with assign / lease / heartbeat, reserved delivery to a named
  agent when it is free, one job per agent, lifecycle prompts
  ([§5.2](../concepts/design/05-core-objects.md)). A job's owner closes it
  with `tower_task_status {state, result}`. `tasks.result` is free-form
  JSON and optional.
- **Schedules**: cron plus timezone, with exactly-once firing (CAS on
  `next_run_at`, unique `(schedule_id, occurrence_at)`).
- **Event log**: `EventLog::append` inserts into `events` under a write
  mutex and advances a `tokio::sync::watch` head (`storage.rs`). The only
  in-process follower was the web UI. Nothing in the server *reacted* to
  events: state changes were inline calls (`tasks::report` calls
  `hooks::closed` and `dispatch_for`) or ran on the sweeper's 10 s tick.
- **The state write and its event are separate statements.**
  `tasks::report` runs its `UPDATE` on the pool, then appends the event on
  the event log's own connection. A crash between the two keeps the state
  and loses the event.
- `tasks.context_id` (A2A context grouping) and the `artifacts` table
  existed and were unused.
- Phase 2c's decision (item 8) rejected carrying results into later jobs
  because context would grow without bound, "revisit only with a bounded
  design". A workflow hand-off is that bounded design: it is scoped to one
  run, limited to declared nodes, and capped in size.

## Decision

### 1. Objects

- **Workflow**: a named graph definition, stored in versions. Applying a
  changed definition adds a version. A run pins the version it started
  with, so an edit never changes a run in flight (the same rule as an open
  schedule occurrence keeping its copy, [§5.2.3](../concepts/design/05-core-objects.md)).
- **Run**: one execution of a version, with an `input` object and the
  trigger that started it. States: `running | completed | failed |
  canceled`.
- **Step**: one execution of one node in a run. A node that a loop visits
  again gets a new step with `visit = n + 1`. The step rows are the
  execution history. They are durable rows, not events, because events are
  pruned after 14 days ([§17.6](../concepts/design/17-open-questions.md)).
  Step states: `pending | ready | running | completed | failed |
  canceled` (`pending` is a `join = "all"` node collecting its inputs).

### 2. Node kinds

- **`job`** (the only kind in phase 7): tower creates a job reserved for
  an agent. The job's description is the node's prompt plus the hand-off
  ([§4](#4-hand-off-between-steps)), so it shows in `tower task show` and
  reaches the agent in the delegation notice. The step completes when the
  owner closes the job. This one kind covers both "an agent runs a prompt"
  and "a tower job runs". A bare prompt with no job has no lease, no result
  channel, and no completion signal except `idle` detection. That breaks
  the rule that completion is owner-reported and never inferred
  ([§9.4](../concepts/design/09-server-modules.md)). So every agent step is
  a job. The "node that queues a job for the right agent" in example 1 is a
  `job` node whose agent is chosen from an earlier outcome:
  `agent = { from = "triage", map = { build = "builder", design = "designer" } }`.
  The node can set the job's `title`, `pre_run`/`post_run`, `priority`,
  `lease_s`, and `max_attempts`. Untargeted jobs are never auto-assigned
  ([§5.2.2](../concepts/design/05-core-objects.md)), so the definition must
  name an agent or a mapping to one.
- **`exec`** (follow-up, not in phase 7): tower would run a command (argv,
  never a shell string) on the coordinator, with a timeout. Template values
  would go into argv elements only. Exit 0 would complete the step and
  parse stdout as JSON into `data` (other stdout becoming `summary`); a
  non-zero exit would fail the step with the stderr tail as `error`. A tool
  step is not an agent process, so the herdr-only rule for agents would
  still hold. An `exec` node runs commands as the server's user, which is
  one reason defining workflows is operator-only from the start.

Also not in phase 7: a human `gate` node (a `question` to `me` whose
answer picks the edge), and an `a2a` node (delegation to a foreign A2A
agent, phase 6 backlog).

### 3. Edges, branches, joins, loops

- An edge goes `from` one node `to` another, with an optional `on`. `on` is
  one of the source node's declared `outcomes`, or `completed` (the
  default: any successful close), or `failed`.
- **Branching is by outcome.** A node that branches declares
  `outcomes = ["build", "design"]`. Its job prompt tells the agent to report
  `result.outcome` as one of them. `tasks::report` rejects a `completed`
  report whose outcome is missing or not in the list (400 `invalid`,
  `detail.reason = "outcome_invalid"`). The agent is still in its session
  and can report again. One closed set of words is easier for a model to
  get right than a free expression, and the server checks it at the
  source. An expression language (for example CEL over `data`) can come
  later if outcomes are not enough.
- **Fan-out**: several edges leave one node, or several nodes have no
  incoming edge. Nodes with no incoming edge start at run start.
- **Join**: `join = "any"` (default) starts a new step for each edge that
  fires into the node. `join = "all"` collects fired edges in a `pending`
  step and runs once every incoming edge has fired. A branch that is not
  taken never fires, so instead of passing "skipped" markers down every
  path, a `pending` join also runs when the run has nothing else `ready`
  or `running`: it runs with the edges that did fire. After it starts, the
  next fire into it opens a new `pending` step. One rule covers skipped
  branches and loops; the cost is that a join waiting on a skipped branch
  starts only after the rest of the run goes quiet.
- **Loops** are edges back to an earlier node. Each node has
  `max_visits` (default 1; a loop target sets it higher). An activation
  past the limit creates the step as `failed` with `max_visits`.
- **Failure**: a failed step follows its `on = "failed"` edges. With none,
  the run fails at once: its open steps are canceled and their jobs
  canceled (interrupting their owners, the existing cancel path). A job
  that is canceled, rejected, or lease-exhausted counts as a failed step.
- **End**: no explicit end node. A run completes when no step is `pending`,
  `ready`, or `running`. An optional top-level `result = "<node>"` copies
  that node's latest result to the run.

### 4. Hand-off between steps

- **Result contract for a workflow job**, enforced by `tasks::report` on
  `completed`: `result` is an object `{summary, outcome?, data?}` (a plain
  string becomes `{summary}`, as the CLI already does). `summary` is
  required, at most 2,000 characters; `data` is optional JSON of at most
  16 KB serialized; `outcome` is required exactly when the node declares
  `outcomes`. Violations answer 400 `invalid` with `detail.reason` =
  `summary_required | summary_too_long | outcome_invalid | data_too_large`.
  A `failed` report needs no summary; its result becomes the step's
  `error`. The agent writes the summary because it knows best what it did,
  and the report path already exists. A `post_run` prompt cannot do this:
  its output only reaches the pane.
- **Summaries are pushed, artifacts are pulled.** The job description
  carries the run input and, for each node in the step's `context` list
  (default: the node of the step that activated it), the latest completed
  step's outcome, summary, and artifact list. On a loop visit it also
  carries the node's own previous visits, so the worker sees what it did
  and what the reviewer said. Large output (the digest gatherers' notes)
  goes into an artifact attached to the job (`tower_artifact_put`,
  `tower artifact put`). The next description names the artifact ids, and
  the agent reads them with `tower_artifact_get` when it needs them. The
  caps bound what is pushed, whatever the size of the work.
- Artifacts are text, at most 1 MiB each, stored in SQLite with the job
  they belong to. Only the job's owner attaches them, while the job is
  open.
- Whether an agent's pane keeps its context between visits is the node's
  `pre_run` choice (`/clear` for a fresh start, nothing to keep it). The
  hand-off does not depend on pane context.

### 5. Triggers

| Kind | Starts a run when | Input |
|---|---|---|
| manual | `tower workflow run <name> --input k=v`, REST, or MCP; always allowed for an enabled workflow | given |
| `schedule` | a cron firing | the trigger's static `input` table |
| `event` | an event on the bus matches `type` + `match` | the trigger's `input`, templated from the event |
| `webhook` | follow-up: an external POST with a verified signature | — |

- **Schedule triggers reuse `schedules`.** Applying a workflow syncs one
  schedule row per schedule trigger (`schedules.workflow_id`,
  `schedules.workflow_input`); an unchanged cadence keeps its row and
  `next_run_at`. Firing starts a run instead of creating a job, through the
  same CAS, coalescing, and pause logic. `tower schedule list` shows these
  rows; pause, resume, and run work on them; edit and remove answer
  `conflict` (the definition owns them: change it and apply). Removing or
  disabling the workflow removes or pauses its schedules.
- **Overlap**: a schedule or event trigger skips while a run it started
  is still `running` (the schedule rule; recorded as `schedule.skipped`
  for schedules). Manual starts are not limited.
- **Event matching**: `type` is an exact event type; each `match` key is
  a dotted path into the event payload (`subject_id` and `subject_type`
  match the envelope), equal to the given value, or containing it when the
  payload value is an array. Events about the workflow's own runs and jobs
  are ignored, so a run cannot trigger itself.

### 6. Driving it: the engine follows the event log

- **Consumer**: one tokio task in `tower serve` reads the `events` table
  after a durable cursor (`event_consumers.cursor`, name `workflows`). The
  existing `watch` head wakes it, so there is no polling delay. It handles
  task events (a step's job reached a terminal state) and every type an
  enabled event trigger names, then saves the cursor. On first boot the
  cursor starts at the head, so old events never fire triggers.
- **Serialization**: every write to run and step rows holds one async
  mutex (`AppState::workflow_lock`). Routes, the consumer, and
  reconciliation all take it, so two branches that finish at the same
  moment reach a join one after the other. Every transition is also a CAS
  on the step's state, so handling an event twice changes nothing.
  Delivery is at-least-once.
- **Reconciliation covers the lost-event gap.** On boot and on each
  sweeper tick the engine finishes `running` job steps whose job is
  already terminal, starts `ready` steps whose job was never created, and
  re-checks every running run for its end. A step's job id is chosen when
  the step row is written and the job insert ignores a duplicate id, so a
  crash between the two never creates a second job. A later change can
  close the gap at the source (append the event in the state change's
  transaction); reconciliation makes the engine correct without it.
- **Retention**: the consumer registers its position with
  `EventLog::track_cursor`, the mechanism open SSE streams use, so pruning
  never passes it while the server runs.

Why this and not the alternatives:

- **Inline calls** (the `hooks::closed` pattern): each place that closes a
  job (report, lease exhaustion, cancel, occurrence expiry) would need a
  call, and event triggers need a general subscription anyway.
- **`tokio::sync::broadcast`**: it drops messages for slow receivers and
  loses everything on restart. Recovery would need the log anyway.
- **An external broker** (NATS, Redis): a second service and a second
  source of truth, against the single-binary design and
  [agentd's lesson](architecture-recommendations.md).

Events can be simple because tower is one binary: the transport is the
table plus the watch channel that already exist. The work is in durability
(cursor, reconciliation) and idempotency (CAS), not in moving bytes.

### 7. A2A: not now

A2A adds nothing inside one deployment today:

- A2A is for opaque parties that share no state
  ([reference](../references/a2a.md): "don't over-rotate … A2A is the edge
  protocol for parties that don't share a database"). The engine, the queue
  and every agent share one database and one process.
- The harnesses speak MCP and read prompts. None of them speak A2A, so A2A
  inside tower would need an adapter on both sides of every hand-off.
- Agents never see the graph. They get a job and report a result, and the
  engine routes. That separation makes an agent-to-agent protocol
  unnecessary.

The design keeps A2A cheap to add later. A run's id is the `context_id` of
its jobs (A2A's contextId groups related tasks). Results and artifacts stay
Part- and Artifact-shaped. Job states are already A2A states. A2A becomes
worth doing for one of three things: a node that delegates to a foreign
agent (outbound client), a workflow published as an Agent Card skill so
outside agents can start it, or runs that span two tower deployments. Each
is a phase-6 adapter, not a change to the engine.

### 8. Data model

Migration 0007:

```sql
CREATE TABLE workflows (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,
  description     TEXT,
  enabled         INTEGER NOT NULL DEFAULT 1,
  current_version INTEGER NOT NULL,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);

CREATE TABLE workflow_versions (
  workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  version     INTEGER NOT NULL,
  source      TEXT NOT NULL,            -- TOML as applied
  definition  TEXT NOT NULL,            -- validated graph, JSON
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (workflow_id, version)
);

CREATE TABLE workflow_runs (
  id            TEXT PRIMARY KEY,       -- = tasks.context_id of its jobs
  workflow_id   TEXT NOT NULL,          -- no FK: history survives removal
  workflow_name TEXT NOT NULL,
  version       INTEGER NOT NULL,
  state         TEXT NOT NULL,          -- running | completed | failed | canceled
  input         TEXT NOT NULL,          -- JSON object
  trigger       TEXT NOT NULL,          -- JSON {kind, by?, schedule_id?, event_seq?, trigger?}
  result        TEXT,
  error         TEXT,
  created_at    INTEGER NOT NULL,
  ended_at      INTEGER
);

CREATE TABLE workflow_steps (
  id           TEXT PRIMARY KEY,
  run_id       TEXT NOT NULL REFERENCES workflow_runs(id),
  node_id      TEXT NOT NULL,
  visit        INTEGER NOT NULL,
  state        TEXT NOT NULL,           -- pending | ready | running | completed | failed | canceled
  activated_by TEXT NOT NULL DEFAULT '[]',  -- JSON array of step ids whose edges fired
  task_id      TEXT UNIQUE,             -- chosen before the job exists (no FK)
  agent_id     TEXT,
  outcome      TEXT,
  result       TEXT,                    -- {summary, data}, frozen at close
  error        TEXT,
  created_at   INTEGER NOT NULL,
  started_at   INTEGER,
  ended_at     INTEGER,
  UNIQUE (run_id, node_id, visit)
);

CREATE TABLE event_consumers (
  name   TEXT PRIMARY KEY,              -- 'workflows'
  cursor INTEGER NOT NULL
);

-- schedules.workflow_id, schedules.workflow_input (NULL = fires a job)
-- artifacts: rebuilt (never written before) with content in SQLite:
--   id, task_id, name, media_type, content TEXT, size, created_by, created_at
-- workflow jobs: tasks.origin = 'workflow', tasks.context_id = run id
```

`workflow_steps.result` repeats the job's `tasks.result` on purpose: the
snapshot is fixed when the step closes, and a future `exec` step reads the
same way. The old `artifacts.content_path` design (files under
`artifacts/`) is dropped with `Paths::artifacts_dir`: artifacts are small
text, and one database file stays the whole backup.

New event kinds, subject `workflow:<id>` or `workflow_run:<id>`:
`workflow.applied | removed | enabled | disabled`,
`workflow.run.started | completed | failed | canceled`,
`workflow.step.started | completed | failed | canceled`.
`artifact.created` has subject `task:<id>`, so it lands in the job trail.

### 9. Definition format

TOML, like `config.toml` ([§4](../concepts/design/04-paths-and-configuration.md)).
`tower workflow apply <file>` validates the file and stores a new version
when it changed. Validation checks: node ids are unique slugs, edges name
real nodes, `on` values are declared outcomes (or `completed`/`failed`),
a mapped agent's `from` node declares every mapped outcome, a node with a
loop edge into it allows more than one visit, agents named by the
definition exist, templates refer only to known names. Templates are a
small `{{ path }}` substitution, not a template engine: `{{ input.<key> }}`,
`{{ run.id }}`, `{{ nodes.<id>.outcome }}`, `{{ nodes.<id>.summary }}`,
`{{ nodes.<id>.data.<path> }}`, and in event-trigger inputs
`{{ event.<path> }}`. An unknown root or undeclared input is an apply
error; a node that has not run yet renders as empty text.

Example 1:

```toml
name = "issue-to-done"
result = "review"

[inputs]
issue = { required = true, description = "Issue id, e.g. ENG-123" }

[[nodes]]
id = "triage"
agent = "triager"
outcomes = ["build", "design"]
prompt = "Read issue {{ input.issue }}. Is it software build work or design work?"

[[nodes]]
id = "work"
agent = { from = "triage", map = { build = "builder", design = "designer" } }
context = ["triage", "review"]
max_visits = 3
prompt = "Do the work for issue {{ input.issue }}."

[[nodes]]
id = "review"
agent = "reviewer"
outcomes = ["approved", "changes"]
context = ["work"]
prompt = "Review the work for issue {{ input.issue }}."

[[edges]]
from = "triage"
to = "work"

[[edges]]
from = "work"
to = "review"

[[edges]]
from = "review"
to = "work"
on = "changes"
```

`approved` has no edge, so that path ends there and the run completes. A
third `changes` fails `work` on `max_visits`, and the run fails.

Example 2:

```toml
name = "daily-digest"

[[triggers]]
kind = "schedule"
cron = "0 8 * * MON-FRI"
timezone = "America/Vancouver"

[[nodes]]
id = "slack"
agent = "slack-scout"
prompt = "Collect yesterday's decisions and open threads. Attach the full notes as an artifact."

[[nodes]]
id = "linear"
agent = "linear-scout"
prompt = "Collect yesterday's project status changes. Attach the full notes as an artifact."

[[nodes]]
id = "aggregate"
agent = "librarian"
join = "all"
context = ["slack", "linear"]
prompt = "Merge both reports into docs/knowledgebase/ and add a log entry."

[[edges]]
from = "slack"
to = "aggregate"

[[edges]]
from = "linear"
to = "aggregate"
```

An event trigger, for shape:

```toml
[[triggers]]
kind = "event"
type = "agent.state"
match = { to = "blocked" }
input = { agent = "{{ event.subject_id }}" }
```

`kind = "job"` on a node is the default and may be omitted.

### 10. Surfaces

- **CLI**: `tower workflow apply <file> | list | show <name> | rm <name>
  | enable <name> | disable <name>`, `tower workflow run <name>
  [--input k=v]... [--wait]`, `tower run list [--workflow <name>]
  [--state <s>] | show <id> | cancel <id>`, `tower artifact put <task>
  <name> [--file <path>] | get <id>`. `run show` prints the steps in
  order, with visit, agent, state, outcome, and summary.
- **REST**: `POST /v1/workflows` (apply), `GET /v1/workflows`,
  `GET /v1/workflows/{name}`, `DELETE /v1/workflows/{name}`,
  `POST /v1/workflows/{name}/enable|disable|runs`, `GET /v1/runs`,
  `GET /v1/runs/{id}`, `POST /v1/runs/{id}/cancel`,
  `POST /v1/tasks/{id}/artifacts`, `GET /v1/artifacts/{id}`. A run's live
  events stream from `GET /v1/events?subject=workflow_run:<id>`; no
  separate stream route. `GET /v1/tasks/{id}` lists the job's artifacts.
- **MCP**: `tower_workflow_list`, `tower_workflow_show`,
  `tower_workflow_run`, `tower_run_list`, `tower_run_show`,
  `tower_artifact_put`, `tower_artifact_get` for every caller;
  `tower_workflow_apply`, `tower_workflow_remove`, `tower_run_cancel` for
  the operator only. Enable/disable is CLI and REST only.
- **Cancel**: canceling a run cancels its open jobs (and interrupts their
  owners) and marks open steps `canceled`.
- **Removal**: removing a workflow with a running run answers `conflict`;
  cancel the run first. Finished runs keep their rows.

## Rejected alternatives

- **Separate "agent prompt" and "job" node kinds**: see
  [§2](#2-node-kinds). A prompt with no job has no completion signal tower
  trusts.
- **Agents drive the graph** (the worker queues the review job itself and
  waits): every agent would need to know the graph, and "waiting" would be a
  model holding a session open. The engine routes; agents stay unaware of
  the graph.
- **Run state as an event-sourced projection**: events are pruned, and a
  projection makes a joined, looping graph hard to query. Step rows are the
  record; events announce changes.
- **Dead-path elimination for joins** (passing "skipped" down every
  untaken branch): with loops, an edge can be skipped on one visit and
  fire on the next, so skip markers need generations. Resolving a pending
  join when the run goes quiet gives the same answer without them.
- **A general expression language for branches in phase 7**: outcomes are
  enough for both examples, and the server can check them at report time.
- **A template engine** (minijinja): `{{ path }}` lookups are all the
  definitions need; a dependency and a sandbox are not.
- **Workflow files watched in the config directory**: a second way to
  change server state, outside the API, with no validation response. The
  operator keeps files in git and runs `apply`.
- **A2A between tower's own agents**: [§7](#7-a2a-not-now).

## 11. Operator decisions (2026-11-04)

1. **One `job` node kind** for both "agent runs a prompt" and "a tower
   job". Accepted.
2. **Agents may start runs, not define them.** Over MCP an agent may list
   and show workflows, start runs, and list and show runs. Applying,
   removing, enabling, disabling, and canceling are operator-only. A
   starting point; revisit when an orchestrator role exists
   ([§17.8](../concepts/design/17-open-questions.md)).
3. **Jobs only in the first iteration.** `exec` nodes are a documented
   follow-up ([§2](#2-node-kinds)).
4. **Caps**: summary 2,000 characters, `data` 16 KB, artifact 1 MiB, with
   a per-node `context` list. A starting point, recorded here so a change
   is a decision change.
5. **Already-running agents first.** A workflow job for a stopped agent
   waits like any reserved job. The next iteration extends workflows into
   [on-demand agents](../concepts/on-demand-agents.md): a workflow job for
   a defined-but-stopped agent starts it. Decide that together with the
   on-demand question.

## Consequences

- No dependency on phase 2c: the result contract applies to workflow jobs
  only, checked by `tasks::report`. Phase 2c's mandatory result for every
  job stays its own change.
- Amends item 8 of the [outcomes-and-brief decision](outcomes-and-brief.md):
  results reach later jobs inside one run, from declared nodes, within the
  caps. Jobs outside workflows are unchanged.
- Migration 0007 takes the next number; phase 2c's planned migrations move
  to 0008 and 0009.
- `tasks::report` gains the contract check; `tasks` gains a workflow-job
  insert (origin `workflow`, `context_id` = run id, id chosen by the
  step). `schedules` gains workflow firing and the managed-row guard.
- A new server module `workflows` ([§9](../concepts/design/09-server-modules.md)),
  started from `serve.rs` next to the pump and the sweeper.
- Follow-ups: `exec` nodes, on-demand agents for workflow jobs, webhook
  triggers, gate nodes, TUI and web views of runs, branch expressions, the
  operator agent's (`agent/`) workflow tool.
