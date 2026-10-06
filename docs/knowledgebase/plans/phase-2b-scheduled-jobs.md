---
type: Plan
title: Phase 2b — Scheduled and recurring jobs
description: Reserved delivery (dispatcher, one job per agent), one-off not_before jobs, recurring cron schedules with documented policies, and running tower as a user service.
tags:
  - plan
  - phase-2b
  - job-queue
  - scheduling
status: stable
generated:
  by: human:geoff
  at: "2026-10-05T23:46:56Z"
---

# Phase 2b — Scheduled and recurring jobs

Goal: the operator can reserve a job for a busy agent and have it delivered
when the agent is free, hold a job until a time, and define recurring
schedules ("daily 09:00 → backend, whenever it's available") that fire
exactly once per occurrence with documented overlap / catch-up / expiry /
removal behavior. Decision: [scheduled jobs](../decisions/scheduled-jobs.md).

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): a job reserved for a busy agent is delivered
only when it goes idle; a schedule fires exactly once per occurrence across
a restart, skips while the previous run is being worked, coalesces missed
firings, and a live agent runs a scheduled job end to end.

Depends on: phase 2 (job queue, sweeper, pump transitions).

---

## Milestone 1 — Reserved delivery ([D§5.2.2](../concepts/design/05-core-objects.md))

- **T1.1** Migration 0003: `tasks.target_agent_id`, `not_before`,
  `schedule_id`, `occurrence_at` (+ unique `(schedule_id, occurrence_at)`),
  `schedules` table ([D§6](../concepts/design/06-data-model.md)). Core types + `schedule.*` event kinds.
  Verify: round-trip serde; migration applies on a phase-2 database.
- **T1.2** One job per agent: the assignment `UPDATE` also requires no open
  job owned by the agent. Verify: assigning a busy agent → `conflict`;
  concurrent assigns of two different jobs to one agent → exactly one wins.
- **T1.3** Dispatcher: reserved queued jobs (`not_before` passed) are
  assigned to their target when it is available; triggers = the target's
  row entering `idle`/`done`, its job closing, each sweep. Verify:
  clock-injected — busy target waits, idle target gets it, priority order,
  `not_before` honored, dead/blocked target waits.
- **T1.4** API/CLI: `assign --when-available`, `create --assign X
  --when-available | --at T`; queue views show the reservation. Verify:
  route tests; CLI smoke.

## Milestone 2 — Recurring schedules

- **T2.1** Scheduler (in the sweep): cron + IANA tz via croner/chrono-tz;
  CAS-advanced `next_run_at`; policies per the decision table (skip,
  replace undelivered, coalesce, pause on target removal, run-now,
  pause/resume). Verify: clock-injected tests for every policy row, DST
  gap/overlap, exactly-once across a simulated restart and concurrent
  sweeps.
- **T2.2** Routes, CLI (`tower schedule …`), MCP tools (operator-only),
  schema registry. Verify: route + MCP tests; CLI smoke.

## Milestone 3 — Always-on server

- **T3.1** `tower service install|uninstall|status`: launchd user agent
  (macOS) / systemd user unit (Linux) running `tower serve` with the
  caller's `TOWER_HOME`. Verify: generated unit lints; install → server
  reachable → uninstall leaves nothing behind.

## Milestone 4 — Exit verification

- **T4.1** Live, real herdr + omp: reservation waits while the agent is
  busy and is delivered on idle; a schedule (every-minute cron) fires,
  skips while its run is worked, and after a server restart fires exactly
  once; the agent completes a scheduled job. Record in the log below.

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-09-27 | T1.2 one job per agent: busy agent → `conflict` naming the open job; 16 concurrent assigns of different jobs to one agent → exactly 1 wins | pass (`tests/dispatch.rs`) |
| 2026-09-27 | T1.3 dispatcher, clock-injected: busy target waits and gets it when its job closes (lease starts at delivery, `by: dispatch`); working → idle transition delivers; `not_before` boundary; priority order; dead target waits; release drops the releaser's own reservation; explicit assign overrides; removal returns reservations to the queue | pass (12 tests) |
| 2026-09-27 | T1.3 found a deadlock: `stop --remove` held the agent lock while releasing jobs, dispatch then prompted the same agent; the lock map is global per name, so 4 tests hung. Also `spawn --prompt` held the lock across `prompt()` (would hang) | fixed: locks released before any call that can re-enter; `detach_agent` clears reservations first |
| 2026-09-27 | T1.4 live CLI (:8277 via `config.toml`): busy assign refused with hint; `--when-available` → `→a1` in list; finishing A delivered B at once (`by dispatch`); `--at` job held until 10:18:00, delivered by the sweep at 10:18:05 | pass |
| 2026-09-27 | T2.1 scheduler, clock-injected: fires once per occurrence + delivers to an idle target (`by: schedule:<id>`); 16 racing sweeps → 1 firing; coalesce (`missed: 3`, most recent occurrence); skip while running; undelivered → `occurrence_expired` + replaced; untargeted → general queue; target removed → paused + target cleared; pause/resume without catch-up; run-now keeps cadence and obeys skip; invalid cron/tz/daily/target rejected; MCP agent callers refused | pass (`tests/schedules.rs`, 12 tests) |
| 2026-09-27 | DST (America/Los_Angeles 2026), croner behavior probed then pinned: daily 02:30 on 03-08 runs at 03:00 PDT; daily 01:30 on 11-01 runs once (first, PDT) | pass — matches the decision table |
| 2026-09-27 | T3.1 `tower service` (real binary, test label, :8277 home): plist lints; install → loaded, running, healthz ok; `kill -9` → relaunched (new pid); uninstall removes plist + job, port closed; second uninstall a no-op. systemd path covered by unit tests only | pass |
| 2026-09-27 | **T4.1 live** herdr + omp `b1`, every-minute schedule reserved for b1 (job: start → `sleep 75` → complete via the CLI): 10:30 fired → b1 completed it; 10:31 **skipped** (`previous_running`); 10:32 fired → completed; 10:33 skipped. Server down 10:33:49-10:36:39 → on restart **one** firing for 10:36 with `missed: 2`, delivered to b1 (trail: reserved → `assigned … by schedule:<id>` → working). CLI run (skipped while running), resume, rm (in-flight job left alone) | pass |
| 2026-09-27 | Found along the way: CLI couldn't find a default install's token on macOS (read `~/.local/share` instead of `ProjectDirs`) and ignored `bind_tcp` | fixed `16e3033` |
