---
type: Plan
title: Phase 2b — Scheduled and recurring jobs
description: Reserved delivery (dispatcher, one job per agent), one-off not_before jobs, recurring cron schedules with documented policies, and running tower as a user service.
tags:
  - plan
  - phase-2b
  - job-queue
  - scheduling
status: draft
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
