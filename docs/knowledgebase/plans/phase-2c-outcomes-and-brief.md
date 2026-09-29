---
type: Plan
title: Phase 2c — Run outcomes, standing brief, job hold
description: Mandatory durable job results, open questions that hold the job and stay answerable in the inbox, a standing brief stored on the agent and restated with every job delivery, and a held flag that parks a queued job until resumed.
tags:
  - plan
  - phase-2c
  - job-queue
  - messaging
  - context
status: draft
generated:
  by: omp/glm-5.3
  at: "2026-09-29T15:40:24Z"
---

# Phase 2c — Run outcomes, standing brief, job hold

Goal: what an agent produces survives the run, what the agent is for
travels with every job, and the operator can park a queued job without
cancelling it. A terminal report carries a result that persists on the job
record. An open question holds the job open and stays in the inbox until
answered. The standing brief is stored on the agent row and restated with
each delivery. A held job stays queued until resumed. Decisions: [run
outcomes and standing brief](../decisions/outcomes-and-brief.md) and [job
hold](../decisions/job-hold.md) — both proposed, to be accepted with this
plan.

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): an
agent completes a job with a reported result that persists on the record; a
question asked at completion holds the job open, stays in the inbox, and is
answerable after expiry; each job delivery restates the stored brief; a
  held reserved job is not dispatched while the target goes idle, is skipped
  instead of replaced by the next schedule firing, and is delivered on
  resume; a server restart loses neither the result nor the brief.

Depends on: phase 2 (messaging, job queue, sweeper, MCP). Touches the TUI
and web inbox surfaces from phases 3–4 (both complete).

---

## Milestone 1 — Open questions hold and stay visible
([D§5.2.1](../concepts/design/05-core-objects.md), [D§5.3](../concepts/design/05-core-objects.md), [D§9.3](../concepts/design/09-server-modules.md))

- **T1.1 Question deadlines and late answers.** Split the deadline default:
  questions 24 h (`DEFAULT_QUESTION_DEADLINE_S`), approvals keep 300 s.
  `deadline_s` overrides still apply. `respond` accepts a question in
  `pending` **or** `expired` status and delivers the answer late; an expired
  approval still returns `conflict` (it was denied on screen). Verify:
  clock-injected tests — question default 24 h, approval 5 min; respond on
  expired question → `answered` + prompt delivered; respond on expired
  approval → 409; override still honored.
- **T1.2 The inbox shows what still needs you.** Default inbox contract
  everywhere: pending items plus expired, unanswered questions; expired
  approvals stay out (denied is decided). CLI `tower inbox` gains
  `--status <s>` and `--all`; TUI inbox lists expired questions with a
  marker and keeps `y`/`n`/`r` on them; the web `needs_you` badge counts by
  the same contract. Verify: route test for the default filter; TUI smoke;
  badge count matches the CLI view.
- **T1.3 Report gate and mandatory result.** `report` to a terminal state
  requires a `result` (400, `result_required`) and is rejected with 409
  `question_open` while the caller has a pending question (from-agent,
  kind `question`, status `pending`). Non-terminal status reports and
  `release` pass unchanged. The gate lifts on answer or expiry. Verify:
  terminal report with open question → 409; after `respond` → succeeds;
  after deadline sweep → succeeds; terminal report without result → 400;
  `working`/`input-required` reports and `release` unaffected; the open job
  still blocks a second assignment (existing one-job-per-agent CAS).

## Milestone 2 — Standing brief
([D§5.2.1](../concepts/design/05-core-objects.md), [D§6](../concepts/design/06-data-model.md), [D§7](../concepts/design/07-server-api.md), [D§10](../concepts/design/10-client-cli.md))

- **T2.1 Migration 0005.** Add `agents.brief TEXT` (nullable); drop the
  dead `agents.config` column. Update the `Agent` struct and every
  constructor. Verify: migration applies on a phase-2b database; brief
  round-trips; the migration checksum test pins the new file.
- **T2.2 Surfaces.** Spawn stores `--prompt` into `brief` (and still sends
  it once, unchanged). `PATCH /v1/agents/{id}` `{brief}` sets or clears,
  operator-only. CLI `tower brief <name>` shows; `tower brief <name>
  '<text>'` sets; `--clear` removes; `--send` also delivers it as a prompt
  now. Agent JSON (list, detail, `tower_ps`) includes `brief`. Verify:
  route tests (set, clear, non-operator refused); CLI `--json` shape; MCP
  `tower_ps` carries the brief; `--send` delivers one prompt.
- **T2.3 Delivery restates the brief.** `notify_assignee` prepends
  "You are <name>. <brief>" when set, omits it when `NULL`. The expired
  question prompt also says: report the result and close the job.
  Verify: delegation text contains the brief when set, omits it when
  `NULL`; existing delegation tests updated.

## Milestone 3 — Contract docs, design amendments, exit verification

- **T3.1 Contract and UX docs.** `docs/agent-loop.md`: terminal reports
  require a result; ask before completing — an open question holds the job;
  after expiry, close with your best answer. `tower-agent-add` skill: brief
  template gains the report-and-ask lines. Review `tower-queue-job` for the
  same. `docs/getting-started.md`: §6 questions (24 h, expired questions
  stay visible and answerable), §7 task show carries the result, §4 spawn
  table (the prompt is stored as the standing brief), command summary
  (`tower brief`). Verify: examples match real output; doc read-through.
- **T3.2 Design amendments.** Apply the deltas listed in the
  [outcomes decision](../decisions/outcomes-and-brief.md) and the
  [hold decision](../decisions/job-hold.md) to
  [D§5.2.1](../concepts/design/05-core-objects.md),
  [D§5.3](../concepts/design/05-core-objects.md),
  [D§6](../concepts/design/06-data-model.md),
  [D§7](../concepts/design/07-server-api.md),
  [D§9.3](../concepts/design/09-server-modules.md),
  [D§10](../concepts/design/10-client-cli.md), and the scheduled-jobs
  policy table. Flip both decisions from `proposed` to `accepted` once
  the operator approves this plan.
- **T3.3 Live exit verification.** Real herdr + one agent (omp or pi):
  complete a job with a result → `tower task show` carries it; ask a
  question at completion → report 409, inbox item, job held open, second
  assignment refused; answer after expiry → late answer delivered, job
  completes; next job's delegation restates the brief; restart the server
  → brief and history intact. Record in the log; update the plans overview
  status; add the CHANGELOG entry.

## Milestone 4 — Job hold
([job-hold decision](../decisions/job-hold.md), [D§5.2.1](../concepts/design/05-core-objects.md), [D§5.2.2](../concepts/design/05-core-objects.md))

- **T4.1 Migration 0006 + core.** `tasks.held_at INTEGER` (NULL = not
  held); `task.held` / `task.resumed` event kinds; Task JSON carries
  `held_at`. Verify: applies on a 0005 database; serde round-trip;
  checksum test pins the new file.
- **T4.2 Hold semantics.** `hold` sets the flag on a queued job
  (reservation kept), `resume` clears it and attempts delivery at once;
  both operator-only. Dispatch (`dispatch_for`, `dispatch_all`) skips
  `held_at IS NOT NULL`; `assign` returns `conflict` naming the hold;
  `cancel` still works; re-holding is idempotent. Verify:
  clock-injected: held reserved job is not delivered while the target
  cycles idle; resume delivers on the spot; manual assign → `held`
  conflict; cancel on held works; double hold/resume are no-ops.
- **T4.3 Schedules compose.** `apply_occurrence` treats a held queued
  occurrence as `previous_held` → `schedule.skipped` (the held job is
  not replaced); unheld behavior is unchanged. Verify:
  clock-injected — held occurrence skipped with reason `previous_held`;
  unheld occurrence still replaced (`occurrence_expired`); schedule with
  no open occurrence still fires.
- **T4.4 Surfaces.** `POST /v1/tasks/{id}/hold` / `resume` routes;
  `tower task hold/resume <id>`; `tower task assign <id> <agent>
  --when-available --at <time>` (the route passes `not_before`
  through, mirroring create); MCP `tower_task_hold` /
  `tower_task_resume` (agent callers refused); TUI jobs view held
  marker + action keys; `task list`/`show` show held. Verify: route +
  MCP tests (agent refused, unknown id); CLI smoke incl. `--json`;
  TUI shows the marker; `assign --at` flows into `not_before`.
- **T4.5 Live hold verification.** Real herdr + one agent: reserve a
  job for it, hold it, let the agent go idle → not delivered; resume →
  delivered at once; a schedule firing on a held occurrence skips.
  Record in the log below.

## Backlog (seeds)

- Supervisor/controller agent for fleet self-governance (compaction,
  context reset, question triage, hold policy) — [design §17.9](../concepts/design/17-open-questions.md);
  likely orchestrator-role, not a harness fork.
- Harness-hook injection (claude `SessionStart`, pi extensions) — only if
  restatement inside a job is ever needed.
- Transcript artifacts — the `artifacts` table is unused; full run output
  would outlive the 14-day event retention.
- A question-withdraw tool, so an agent can cancel its own question before
  the deadline instead of waiting for the sweep.
- Result carry-over into later delegations — rejected for unbounded context
  growth; revisit only with a bounded design.

## Verification log

| Date | Check | Result |
|---|---|---|
