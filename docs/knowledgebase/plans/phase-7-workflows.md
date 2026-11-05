---
type: Plan
title: Phase 7 — Workflows
description: Versioned workflow graphs of job nodes, outcome-routed edges with fan-out, join and bounded loops, a bounded hand-off of summaries and artifacts between steps, manual, schedule and event triggers, and an engine that follows the event log from a durable cursor — over REST, CLI, and MCP.
tags:
  - plan
  - phase-7
  - workflows
  - job-queue
  - events
status: stable
generated:
  by: omp/claude-opus-5-5
  at: "2026-11-04T12:00:00Z"
---

# Phase 7 — Workflows

Goal: the operator applies a workflow definition, starts a run (or a
schedule or an event starts it), and tower drives the run to its end by
itself: each step is a job for a named agent, each agent reports a
bounded summary, the engine picks the next steps from the reported
outcome and hands them the summaries. Decision:
[workflows](../decisions/workflows.md).

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)):
a run of the issue-to-done shape branches on the triage outcome, loops
worker → reviewer until approval, and stops at `max_visits`; a run of the
digest shape fans out to two agents and joins; each step's job carries the
earlier steps' summaries and artifact ids; a workflow job's `completed`
report without a valid summary or outcome is refused; a schedule trigger
starts runs exactly once per occurrence and skips while its run is open;
an event trigger starts a run and never triggers itself; a lost
`task.completed` event and a server restart mid-run both end in the same
run state; a live agent runs a two-step workflow end to end.

Depends on: phase 2b (reserved delivery, schedules), phase 2d (lifecycle
prompts on jobs). Independent of phase 2c: the result contract applies to
workflow jobs only. Migration 0007; phase 2c's migrations move to 0008
and 0009.

---

## Milestone 1 — Data and definitions ([D§5.6](../concepts/design/05-core-objects.md), [D§6](../concepts/design/06-data-model.md), [D§7](../concepts/design/07-server-api.md))

- **T1.1 Migration 0007 + core.** Tables `workflows`,
  `workflow_versions`, `workflow_runs`, `workflow_steps`,
  `event_consumers`; `schedules.workflow_id`, `schedules.workflow_input`;
  `artifacts` rebuilt with `content TEXT` (never written before), and
  `Paths::artifacts_dir` removed. tower-core: `WorkflowId`, `RunId`,
  `StepId`; `Workflow`, `WorkflowRun`, `WorkflowStep`, `Artifact`
  (content-less row); `RunState`, `StepState`; event kinds `workflow.*`
  and `artifact.created`; schema registry. Verify: applies on a 0006
  database; serde round-trip; the checksum test pins the new file.
- **T1.2 Definition parser.** TOML → validated `Definition` (nodes,
  edges, inputs, triggers, `result`); the rules in the decision's §9;
  `{{ path }}` templates with apply-time name checks; render with a run
  context. Verify: unit tests — each rule refuses with a message naming the
  node or edge; both decision examples parse; template render covers
  input, node outcome/summary/data path, missing node → empty, event path.
- **T1.3 Apply, list, show, remove, enable, disable.** `POST
  /v1/workflows {source}` stores version 1, or a new version when the
  definition changed (`changed: false` otherwise); agents named by the
  definition must exist. `DELETE` refuses while a run is `running`.
  Disabled workflows refuse manual starts. Events `workflow.applied |
  removed | enabled | disabled`. Verify: route tests — re-apply unchanged
  keeps the version; changed bumps it; bad TOML → 400 with the parser
  message; unknown agent → 400; remove with an open run → 409.

## Milestone 2 — Engine ([D§9.7](../concepts/design/09-server-modules.md), decision §3–§6)

- **T2.1 Runs and job steps.** `start_run` (manual) checks declared
  inputs, writes the run, and starts nodes with no incoming edge. A job
  step resolves its agent (static, or mapped from an earlier node's
  outcome), renders the prompt, appends the hand-off block (run input,
  context steps' outcome + summary + artifacts, own earlier visits, the
  result contract), and inserts a job (origin `workflow`, `context_id` =
  run id, id chosen in the step row, insert ignores a duplicate) reserved
  for the agent. Verify: the job description carries the hand-off; a
  mapped agent follows the outcome; a missing mapping fails the step.
- **T2.2 Result contract.** `tasks::report(completed)` on a workflow job
  checks `{summary ≤ 2000, outcome ∈ outcomes, data ≤ 16 KB}`; a string
  result is `{summary}`. 400 `invalid` with `detail.reason`. Verify: one
  test per reason; a valid report closes the job; non-workflow jobs are
  unchanged.
- **T2.3 Consumer and routing.** Durable cursor (`event_consumers`),
  woken by the watch head, registered with `track_cursor`; task terminal
  events finish their step (completed → outcome + result; failed /
  canceled / rejected → error). Routing: `on` outcome / `completed` /
  `failed`; join any; join all with pending collection and quiet-run
  resolution; `max_visits`; unhandled failure fails the run and cancels
  open steps; run end with `result` copy. Events `workflow.run.*`,
  `workflow.step.*`. Verify (clock-injected, consumer drained directly):
  branch by outcome; review loop runs worker visit 2 with the review
  summary; third `changes` fails on `max_visits`; fan-out + join all runs
  the join once with both summaries; a skipped branch into a join resolves
  when the run goes quiet; `on = "failed"` routes; unhandled failure fails
  the run and cancels the sibling job.
- **T2.4 Reconciliation and cancel.** Boot and every sweep: finish
  running steps whose job is terminal, start ready steps without a job,
  end quiet runs. `cancel_run` cancels open jobs and marks steps
  `canceled`. Verify: delete the `task.completed` event row before the
  consumer runs → the sweep still advances the run; a ready step with no
  job gets exactly one job across two reconciles; cancel stops the run and
  its jobs; a canceled step job alone (operator `task cancel`) fails the
  step.

## Milestone 3 — Triggers (decision §5)

- **T3.1 Schedule triggers.** Apply syncs managed schedules
  (`workflow_id`, `workflow_input`; unchanged cadence keeps `next_run_at`;
  dropped triggers delete their rows). Firing starts a run (trigger
  `{kind: schedule, schedule_id}`), or skips with `previous_running` while
  its last run is open. Schedule edit/remove on a managed row → 409;
  pause/resume/run work. Disable pauses, enable resumes, remove deletes.
  Verify: clock-injected — fires once per occurrence; skip while open;
  re-apply with same cron keeps `next_run_at`; new cron replaces it;
  `schedule rm` → 409.
- **T3.2 Event triggers.** Each consumer batch matches events against
  enabled workflows' event triggers (`type`, `match` paths, array
  contains), renders `input` from the event, skips while the trigger's
  previous run is open, ignores events about the workflow's own runs and
  jobs. Verify: a matching event starts one run; a non-matching one
  doesn't; the workflow's own `task.completed` doesn't; overlap skips.

## Milestone 4 — Artifacts (decision §4)

- **T4.1 Artifacts.** `POST /v1/tasks/{id}/artifacts {as, name, content,
  media_type?}` (owner, open job, ≤ 1 MiB text) → `artifact.created`
  (subject task); `GET /v1/artifacts/{id}` → row + content; `GET
  /v1/tasks/{id}` lists `artifacts`. The hand-off lists context steps'
  artifacts. Verify: non-owner → 409; closed job → 409; too large → 400;
  the next step's description names the artifact id.

## Milestone 5 — Surfaces and docs ([D§7](../concepts/design/07-server-api.md), [D§10](../concepts/design/10-client-cli.md))

- **T5.1 CLI.** `tower workflow apply|list|show|rm|enable|disable|run
  [--input k=v] [--wait]`, `tower run list|show|cancel`, `tower artifact
  put|get` (owner from `$TOWER_AGENT`). Table and `--json` output.
  Verify: CLI smoke against a temp server.
- **T5.2 MCP.** `tower_workflow_list|show|run|apply|remove`,
  `tower_run_list|show|cancel`, `tower_artifact_put|get`; operator-only
  rules from the decision §11. Verify: MCP tests — an agent starts a run
  and reads it; an agent `apply`/`cancel` → unauthorized; an agent puts an
  artifact on its job.
- **T5.3 Docs.** `docs/getting-started.md` (workflows walkthrough,
  command summary), `docs/agent-loop.md` (workflow jobs: summary,
  outcome, artifacts; contract test markers), `docs/mcp.md` tool list,
  design amendments (D§4, §5, §6, §7, §9, §10, §17, §18), knowledge-base
  index and log. Verify: doc read-through against real CLI output.

## Milestone 6 — Exit verification

- **T6.1 Suite.** `cargo make lint && cargo make test` green.
- **T6.2 Live run.** Real herdr + a real harness, temp `TOWER_HOME` on its
  own port: apply a two-step workflow (classify → act, branching on the
  outcome), start a run, the first agent reports its outcome and summary
  through the work loop, the second agent's delegation shows the summary,
  the run completes; restart the server mid-run once. Record in the log.

## Backlog (follow-ups from the decision)

- `exec` nodes (argv command on the coordinator, stdout JSON → `data`).
- On-demand agents for workflow jobs: start a defined-but-stopped agent
  when its step's job is due ([concept](../concepts/on-demand-agents.md)).
- Webhook triggers with signature verification (Linear, GitHub).
- Human `gate` nodes (a question to `me` picks the edge).
- TUI and web views of workflows and runs; the Herdr app panel.
- Branch expressions over `data` (CEL or similar).
- The operator agent's (`agent/`) `tower_workflow` tool.
- Transactional event append (outbox) so reconciliation becomes a
  backstop only.

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-11-04 | **T1.2** `cargo test -p tower-server --lib workflows` | 9 parser/template unit tests pass: both decision examples parse and round-trip through JSON; every rule refuses with the node or edge named (unknown keys, `kind = "exec"`, duplicate ids, unknown edge ends and outcomes, reserved outcomes, unguarded loops, no start node, bad agent maps, unknown template names, event inputs limited to `event.*`, cron/tz/daily checks, required trigger inputs) |
| 2026-11-04 | **T1.3–T5.2** `cargo test -p tower-server --test workflows` | 18 integration tests pass (FakeHarness, clock-injected, consumer drained directly): issue run branches on `design`, loops worker → reviewer with the review summary in visit 2, completes with `result` from `review`; the third loop visit past `max_visits = 2` fails with no job and fails the run; each result-contract reason (`summary_required`, `outcome_invalid`, `summary_too_long`, `data_too_large`) refused while the job stays open, a non-owner gets `conflict`, plain jobs unchanged; fan-out + `join = "all"` runs once with both summaries; a join behind an untaken branch starts when the run goes quiet; a `failed` edge routes with the error in the hand-off; an unhandled failure fails the run and cancels the sibling step and job; operator `task cancel` fails the step; an unmapped outcome fails the step; `run cancel` cancels jobs once, a second cancel → 409; a lost completion event (cursor skipped past) is recovered by `reconcile`; a `ready` step gets exactly one job across two reconciles, and its job is recreated when missing; a schedule trigger fires once per occurrence, skips while its run is open (`schedule.skipped`), keeps its row on an unchanged apply, replaces it on a new cron, refuses `schedule rm`, pauses/resumes with disable/enable; an event trigger matches `{to = "blocked"}`, templates `event.subject_id` into the input, skips while open, ignores its own jobs' `task.completed` but fires on another workflow's; the consumer starts at the head (old events fire nothing); artifacts: non-owner 409, > 1 MiB 400, listed on `task show`, readable, named in the next step's job, refused on a closed job; REST apply versions (201 v1 / 200 unchanged / v2 on change), 400 on bad TOML and unknown agent, missing input 400, remove 409 while running, history survives removal; disabled workflow refuses starts 409; MCP: agent may run/show/put/get, may not apply/cancel |
| 2026-11-04 | **T6.2 live**, herdr 0.9.5 + omp (Haiku 4.5), temp `TOWER_HOME` on port 8397 | `tower workflow apply parity.toml` → `v1 (created)`, re-apply → `v1 (unchanged)`. `workflow run parity --input number=7`, then the server killed and restarted at once: the classifier's delegation showed the hand-off block (input, contract, outcomes); the agent reported `{"summary": …, "outcome": "odd", "data": {…}}` through `tower task status` on the restarted server; the consumer resumed from its cursor and started `announce` for `wfs-act` with the title `parity: announce odd` and the classifier's summary in its job; an artifact put with `tower artifact put … --as wfs-act` and read back with `tower artifact get`; run `completed` with `result` from `announce`. `workflow run parity --input number=4 --wait` followed the even branch (`classify → even → unexpected`) and exited 0. A first attempt on omp's default model hit a provider usage limit; `tower run cancel` canceled that run and its job. Event log: 3 `workflow.run.started`, 2 completed, 1 canceled; cursor = head (176) |
| 2026-11-04 | **T6.1** `cargo make lint && cargo make test` | fmt + clippy (`-D warnings`) clean; 282 tests pass, 0 fail |
