---
type: Decision
title: Job hold — park a queued job without canceling it
description: "A held flag on queued jobs: dispatch skips them, manual assign refuses them, resume delivers them; a held schedule occurrence makes the next firing skip instead of replace; assign gains a time. Queue-wide pause and hold on working jobs are rejected."
tags:
  - decision
  - job-queue
  - dispatch
  - scheduling
status: proposed
generated:
  by: omp/glm-5.3
  at: "2026-09-29T15:40:24Z"
---

# Job hold — park a queued job without canceling it

Date: 2026-09-29. From the same dogfooding batch as the
[outcomes-and-brief decision](outcomes-and-brief.md). Amends the
[job-queue decision](job-queue.md) and the
[scheduled-jobs decision](scheduled-jobs.md), and design
[§5.2.1](../concepts/design/05-core-objects.md),
[§5.2.2](../concepts/design/05-core-objects.md),
[§6](../concepts/design/06-data-model.md),
[§7](../concepts/design/07-server-api.md),
[§10](../concepts/design/10-client-cli.md),
[§18](../concepts/design/18-phase-mapping.md). Plan:
[phase 2c](../plans/phase-2c-outcomes-and-brief.md), milestone 4.

## Context

Verified current behavior — no hold exists:

- A queued job without a reservation already waits. Tower only hands out
  work the operator assigns.
- A reserved job (`--when-available`, `--at`, or a schedule target) is
  delivered by the dispatcher as soon as the target is free
  (`dispatch_for` / `dispatch_all`, `crates/tower-server/src/tasks.rs`).
  Nothing can take it back. The workarounds damage the job: `cancel` is
  terminal; `release` is owner-flavored and drops the reservation; `--at`
  can only be set at create — the assign route has no `not_before` field
  and passes `None` (`AssignBody`, `assign_route`).
- Schedules have `pause` / `resume`; jobs have nothing.

## Decision

1. **`held` is a flag, not a state.** New column `tasks.held_at` (NULL =
   not held; the value records when). The job stays `queued`. This follows
   the [scheduled-jobs](scheduled-jobs.md) precedent: a column is enough;
   a new state would ripple through the A2A mapping, the views, and every
   state check.
2. **`tower task hold <id>`** (also `POST /v1/tasks/{id}/hold`) sets the
   flag. It works on any queued job, reserved or not. The reservation
   (`target_agent_id`, `not_before`) is kept: hold parks delivery, it does
   not drop intent. Holding emits a new `task.held` event.
3. **`tower task resume <id>`** (also `POST /v1/tasks/{id}/resume`) clears
   the flag, emits `task.resumed`, and tries delivery at once — the same
   trigger a fresh reservation gets.
4. **Nothing moves a held job.** The two dispatch queries skip
   `held_at IS NOT NULL`, and a manual `assign` to a held job returns
   `conflict` naming the hold ("resume it first"). This is stronger than
   `not_before`, which a manual assign may ignore: `not_before` is a
   delivery time; hold is an order not to dispatch. The error costs one
   command and prevents a mistaken dispatch. `cancel` still works on a
   held job — the operator can always kill.
5. **A held schedule occurrence changes replace to skip.** The next
   firing skips it (`schedule.skipped`, reason `previous_held`) instead
   of canceling the held occurrence, per the
   [scheduled-jobs](scheduled-jobs.md) policy table. A hold set by the
   operator is a newer order than the schedule's default replace policy.
   An operator who wants the new occurrence cancels the held one.
6. **Assign gains a time.** `not_before` joins the assign route and the
   CLI (`tower task assign <id> <agent> --when-available --at <time>`),
   mirroring create. The server already accepts it in `reserve`; only the
   route and the CLI hide it. A timed hold (`not_before`) and an open
   hold compose: resume after the time delivers at once; resume before it
   still waits for the time.
7. **Operator-only.** Hold and resume are dispatch policy, like assign.
   The MCP tools `tower_task_hold` / `tower_task_resume` refuse
   agent-identified callers.

## Not adopted now

- **Queue-wide pause.** Per-job hold covers the real cases. A global
  switch is a different mechanism, with dispatch bypass and restart
  semantics of its own. A stopped `tower serve` already stops all
  hand-offs. Backlog seed.
- **Hold on an assigned or working job.** The agent already has the job.
  `release` (back to the queue) followed by `hold` covers the intent.
  Pausing an agent mid-work means an interrupt plus a frozen lease —
  different semantics, not needed yet.
- **Hold at create.** An unassigned queued job already waits; nothing
  dispatches it. The flag matters only once an orchestrator exists.

## Consequences

- Migration 0006: `tasks.held_at INTEGER` (0005 is the brief column,
  phase 2c milestone 2).
- New event kinds `task.held` / `task.resumed`; new error code `held`
  (409) on assign.
- `dispatch_for` / `dispatch_all` gain a held guard; `apply_occurrence`
  gains the `previous_held` skip branch.
- CLI: `task hold` / `task resume`, `assign --at`; MCP:
  `tower_task_hold`, `tower_task_resume` (operator-only); the TUI jobs
  view shows a held marker and follows its action-key pattern; queue
  views show held.
- Design amendments at execution: [§5.2.1](../concepts/design/05-core-objects.md)
  (held guard, assign refusal), [§5.2.2](../concepts/design/05-core-objects.md)
  (held reservation), [§6](../concepts/design/06-data-model.md) (column),
  [§7](../concepts/design/07-server-api.md) (routes, tools),
  [§10](../concepts/design/10-client-cli.md) (CLI),
  [§18](../concepts/design/18-phase-mapping.md). The scheduled-jobs
  policy table gains the `previous_held` row.
