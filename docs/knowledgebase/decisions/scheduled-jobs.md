---
type: Decision
title: Scheduled and recurring jobs — reserved delivery, cron schedules
description: Jobs can be reserved for an agent and delivered when it is available; one-off jobs can wait until a time; recurring schedules materialize ordinary jobs on a cron cadence with documented overlap, catch-up, expiry and removal policies.
tags:
  - decision
  - job-queue
  - scheduling
  - dispatch
status: accepted
generated:
  by: human:geoff
  at: "2026-10-05T23:46:56Z"
---

# Scheduled and recurring jobs

Date: 2026-09-27. Builds on the [job-queue decision](job-queue.md). Amends
[§5.2.1](../concepts/design/05-core-objects.md), [§6](../concepts/design/06-data-model.md), [§7](../concepts/design/07-server-api.md), [§9.4](../concepts/design/09-server-modules.md),
[§10](../concepts/design/10-client-cli.md), [§18](../concepts/design/18-phase-mapping.md). Plan: [phase 2b](../plans/phase-2b-scheduled-jobs.md).

## Context

The operator needs jobs dispatched on a schedule — "every day, have
`backend` run the dependency audit, whenever it's free". The phase-2 queue
can't express that: `assign` starts the lease and prompts the agent at
once, and nothing refuses an agent that is already busy. A job assigned at
09:00 to an agent mid-task burns its lease, requeues, and ends in
`lease_exhausted` although nothing failed.

## Decision

### Reserved (deferred) delivery

1. A queued job may be **reserved** for an agent (`tasks.target_agent_id`).
   It stays `queued`; a **dispatcher** assigns it through the normal
   assignment CAS the moment the target is **available**. The lease starts
   at delivery, not at reservation.
2. **Available** = the agent row is `idle` or `done` (not `working`,
   `blocked`, `launching`, `dead`, `unknown`) **and** it owns no open job.
3. Dispatch triggers: the agent's row entering `idle`/`done`; the agent's
   open job closing (completed, failed, released, canceled, lease swept);
   and the ~10s sweep as a backstop (missed edges, restarts). When several
   jobs are reserved for one agent, the highest priority, then oldest, goes
   first.
4. This stays **assignment-only**: the reservation *is* the dispatch
   decision, made in advance by the operator (or a schedule the operator
   created). Agents still never take work.
5. **One job per agent at a time.** Any assignment — manual or dispatched —
   is refused with `conflict` when the agent already owns an open job. The
   check is inside the assignment `UPDATE` (atomic with the owner CAS), so
   two dispatch paths can't hand one agent two jobs.
6. Manual reservation: `tower task assign <id> <agent> --when-available`
   and `tower task create … --assign <agent> --when-available`. Re-running
   it re-targets; an immediate `assign` of a reserved job to anyone is
   still allowed (explicit operator action wins).

### One-off scheduled jobs

7. `tasks.not_before` holds a job back from the **dispatcher** until that
   time. `tower task create … --assign <agent> --at <time>` reserves the job
   for the agent with `not_before = <time>`; `--at` requires a target
   (nothing else would dispatch it). An explicit immediate `assign` ignores
   `not_before`.

### Recurring schedules

8. A **schedule** is a job template (title, description, tags, priority,
   `lease_s`, `max_attempts`, optional target agent) plus a **cron
   expression and an IANA timezone**. Each firing materializes an ordinary
   job (`origin = 'schedule'`, `schedule_id`, `occurrence_at`) — reserved
   for the target when there is one, otherwise a plain queued job for the
   operator. Everything after that is the existing queue: leases, retries,
   trail, inbox.
9. **Exactly-once firing**: the firing step advances `next_run_at` with a
   CAS on its old value, and `(schedule_id, occurrence_at)` is unique — a
   restart, a double tick or two servers racing can't create a duplicate.
10. The timezone is stored on the schedule (default: the machine's zone at
    creation), so a later machine tz change doesn't move the schedule.
    `--daily HH:MM` is sugar for `M H * * *`.

### Policies (all recorded as events)

| Situation | Policy | Event |
|---|---|---|
| Previous occurrence **still being worked** (assigned / working / input-required) when the next fires | **skip** the new occurrence | `schedule.skipped` `{reason: "previous_running"}` |
| Previous occurrence **never delivered** (still queued) when the next fires | the stale one is **canceled** (`result: {"error":"occurrence_expired"}`) and the new one is created — at most one pending occurrence per schedule | `task.status` canceled, `schedule.fired` |
| tower was down / machine asleep across firings | **coalesce**: fire **once**, for the most recent missed time; `next_run_at` = next time after now | `schedule.fired` `{missed: N}` |
| Target agent **removed** (`stop --remove`) | schedule **pauses** and its target is cleared (resuming it sends jobs to the general queue — recreate it to pick a new agent); its reserved jobs lose the target and fall back to the general queue for the operator | `schedule.paused` `{reason: "target_removed"}` |
| Target agent **dead / blocked / busy** | the reserved job **waits**; queue views show it as waiting for the target | — |
| Schedule **paused**, later resumed | no firings while paused; resume computes `next_run_at` from now (no catch-up for the paused span) | `schedule.paused` / `schedule.resumed` |
| **run-now** | one extra occurrence at now, same overlap/expiry rules; `next_run_at` unchanged | `schedule.fired` `{manual: true}` |
| **DST** | croner's rules: a fixed-time job whose local time falls in a spring-forward gap runs at the first valid time after it; in a fall-back repeated hour it runs once. Interval patterns follow elapsed time | — |

Schedules are **operator-only** (REST/CLI; MCP refuses agent-identified
callers): a recurring job is a dispatch policy. Editing is remove + create
in this phase.

### Always-on server

11. Firing happens only while `tower serve` runs; coalescing covers
    downtime. `tower service install|uninstall|status` installs the server
    as a user service (launchd agent on macOS, systemd user unit on Linux)
    so daily jobs don't depend on an open terminal.

## Rejected alternatives

- **Assign at firing time** — the lease burns while the agent is busy (the
  problem above).
- **A `reserved` task state** — a column is enough; a new state would ripple
  through the A2A mapping, views and every state check for no gain.
- **Agents polling for their reserved jobs** — reintroduces self-serve.
- **System cron calling `tower task create`** — no coalescing, overlap or
  exactly-once, and it depends on the cron environment having the token
  and PATH. tower already has a clock-injected sweeper.
- **Firing every missed occurrence after downtime** — a week offline would
  flood the agent with seven identical daily jobs.
