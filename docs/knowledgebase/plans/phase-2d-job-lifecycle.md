---
type: Plan
title: Phase 2d — Job lifecycle prompts and editing
description: Pre-run and post-run prompts on jobs and schedules, sequenced around each delivery and close; PATCH editing for queued jobs and schedules over REST, CLI, MCP, and a TUI edit panel that saves each field on submit.
tags:
  - plan
  - phase-2d
  - job-queue
  - scheduling
  - tui
status: stable
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
---

# Phase 2d — Job lifecycle prompts and editing

Goal: a job creator can attach a pre-run prompt (`/clear`, `/compact`) and
a post-run prompt (`/reflect`, `/learn`) to a job or a schedule, tower
types them into the harness around every run without ever interleaving
them with job prompts, and any open job or schedule can be edited from
the TUI. Decision: [job lifecycle prompts](../decisions/job-lifecycle-hooks.md).

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): a
job's pre-run prompt reaches the agent before its job prompt and the
delegation waits until the agent settles; a closed job's post-run prompt
runs before the next reserved job is delivered; a restart mid-prompt
clears the mark and the lease recovers the job; a queued job's prompt,
schedule, and pre/post-run prompts are edited from the TUI and each edit
is saved as it is submitted; a delivered job refuses all but post-run.

Depends on: phase 2b (reserved delivery, schedules), phase 3 (TUI jobs
view). Independent of phase 2c; its migrations renumber after 0006.

---

## Milestone 1 — Data and editing ([D§5.2.3](../concepts/design/05-core-objects.md), [D§6](../concepts/design/06-data-model.md), [D§7](../concepts/design/07-server-api.md))

- **T1.1 Migration 0006 + core.** `tasks.pre_run`, `tasks.post_run`,
  `tasks.hook`; `schedules.pre_run`, `schedules.post_run`. `Task`/`Schedule`
  fields, `HookPhase`, event kinds `task.updated`, `task.hook`,
  `schedule.updated`; schema registry. Verify: applies on a 0005 database;
  serde round-trip; the checksum test pins the new file.
- **T1.2 Create paths.** `POST /v1/tasks` and `POST /v1/schedules` take
  `pre_run`/`post_run` (blank → none, trimmed); schedule firings copy them.
  Verify: route tests; an occurrence carries its schedule's pair.
- **T1.3 `PATCH /v1/tasks/{id}`.** Queued → every field; delivered →
  `post_run` only; closed → 409; `not_before` needs a reservation and
  re-runs dispatch; CAS against the checked state; `task.updated`.
  Verify: one test per rule, incl. a job assigned between check and write.
- **T1.4 `PATCH /v1/schedules/{id}`.** Title, description, lifecycle
  prompts, cadence (cron/daily/tz → `next_run_at` from now);
  `schedule.updated`; later firings only. Verify: cadence change moves
  `next_run_at`; an open occurrence keeps its copy; invalid cron/tz refused.

## Milestone 2 — Lifecycle sequencing ([D§5.2.3](../concepts/design/05-core-objects.md), [D§9.4](../concepts/design/09-server-modules.md))

- **T2.1 Pre-run.** The assignment `UPDATE` sets `hook = 'pre'`; a
  background runner waits for `idle`/`done`, sends the prompt, waits for
  it to settle, clears the mark, renews the lease, sends the delegation.
  Lease expiry pauses while `hook = 'pre'`. Verify (FakeHarness with a
  polling `wait_until`): prompt order pre-run → delegation; no delegation
  before the agent settles; the paused lease is not swept; a release or
  cancel during pre-run sends no delegation.
- **T2.2 Post-run.** A terminal owner report sets `hook = 'post'`; the
  agent is busy (assignment CAS) until the runner has sent the prompt and
  the agent settled; then the dispatcher runs. Verify: a reserved job
  waits through post-run and is delivered after; manual assign → 409
  naming the post-run; cancel / release / lease expiry never run it.
- **T2.3 Failure, timeout, restart.** `[jobs] hook_timeout_s`; a send
  error or timeout records `task.hook` failed and continues; boot clears
  marks (`interrupted`) before reconcile. Verify: a never-settling agent
  still gets its delegation after the timeout; recovery clears both phases.

## Milestone 3 — Surfaces ([D§7](../concepts/design/07-server-api.md), [D§10](../concepts/design/10-client-cli.md), [D§11](../concepts/design/11-tui.md))

- **T3.1 CLI + MCP.** `--pre-run`/`--post-run` on `task create` and
  `schedule create`; `tower task edit`, `tower schedule edit` (`''`
  clears); `task show`/`schedule show` print the prompts and the hook in
  flight; trail lines for the new events. MCP `pre_run`/`post_run` on the
  create tools, `tower_task_edit` / `tower_schedule_edit` (operator-only).
  Verify: MCP agent callers refused; CLI smoke incl. `--json`.
- **T3.2 TUI edit panel.** `e` on a job or schedule; `j/k` field, `enter`
  inline edit (prefilled), `E` `$EDITOR` (multi-line values always);
  every submit is one `PATCH`; read-only fields on delivered jobs; closed
  jobs refused. Detail panels show the lifecycle prompts; the lease cell
  reads `pre-run` while paused. Verify: view tests for the panel and keys;
  server-backed TUI test edits a queued job end to end.

## Milestone 4 — Exit verification

- **T4.1 Live.** Real server + herdr + one agent: a job with pre-run and
  post-run prompts runs end to end (order visible in the trail and the
  agent's history); a reserved job waits through the post-run prompt; a
  job edited from the TUI (prompt, `at`, pre/post-run) delivers the edited
  text. Record in the log below.

## Backlog (seeds)

- Agent-level default lifecycle prompts (see the decision's "not adopted").
- Editing the remaining fields (priority, tags, lease, schedule target)
  from the TUI panel — the PATCH shape extends without a new mechanism.

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-10-31 | T1.1 migration 0006 applies; checksum pinned (`storage::tests`); `Task`/`Schedule` round-trip with `pre_run`/`hook`; event names pinned to serde | pass |
| 2026-10-31 | T1.2–T1.4 edit rules over REST: queued job takes every field (`null`/`""` clear, absent unchanged, `task.updated` names fields); empty patch / blank title / `not_before` without a reservation → 400; delivered job → 409 except `post_run`; closed → 409; a new `not_before` re-runs delivery; schedule cadence edit recomputes `next_run_at`, an open occurrence keeps its copy, the next firing carries the edit; bad cron/tz/both → 400 | pass (`tests/lifecycle.rs`) |
| 2026-10-31 | T2.1–T2.3 sequencing with FakeHarness (`wait_until` now polls): pre-run typed first and trimmed, no delegation and no lease sweep until the agent settles, full lease from the delegation; cancel during pre-run → no delegation; never-settling prompt (`hook_timeout_s = 1`) → `failed` recorded, delegation still sent; post-run holds the reserved next job (manual assign → 409 naming the post-run) and it is dispatched once the prompt settles; cancel / release / lease expiry never run post-run; `hooks::recover` clears both marks (`interrupted`), the lease then requeues the undelivered job | pass (11 tests) |
| 2026-10-31 | T3.1 CLI smoke (isolated `TOWER_HOME`, :8299): `task create --pre-run ' /clear ' --post-run` (stored trimmed), `task edit` (`''` clears; `--at` without reservation refused; nothing to change refused), `task show` (prompts, multi-line indented, trail `edited: …`), `schedule create/edit/show` (`--daily 23:15` → next moved; bad cron refused); MCP edit tools refuse agent callers | pass |
| 2026-10-31 | T3.2 TUI: view tests (panel snapshot, per-field saves, locked fields on a delivered job, closed job refused, multi-line → `$EDITOR`, `on_edited`, schedule cron/`HH:MM`, required fields kept); server-backed test edits a queued job's pre-run from the jobs view and the panel shows it off the bus. Live in tmux: prompt edited inline, post-run via `E` with `VISUAL` set to a script writing two lines → saved, panel shows `(+1 lines)`. Found: `tower task show` printed continuation lines unindented | pass; fixed (`print_text`) |
| 2026-10-31 | **T4.1 live**, herdr 0.9.5 + Claude Code 2.1.316 (`smoke2`, haiku; folder trust approved through `tower approve`): pre-run `/clear` typed into the real harness, delegation 5 s later (no working state seen → taken-up window → settled); lease paused meanwhile, full lease after. The pane could not reach its model API (gateway env missing in a `/tmp` workdir), so the job lease ran out → requeued; redelivery ran `/clear` again (every delivery). The owner's report was issued with the agent's own CLI verbs (`task start/status --as smoke2`): post-run prompt sent at once, settled 5 s later, and only then the reserved next job was dispatched. Harness screen order: `/clear` → job prompt → post-run → next job. Server killed 0.5 s into a pre-run: boot logged `interrupted lifecycle prompts cleared`, trail `pre-run prompt interrupted: server restarted`, lease ticking again | pass |
