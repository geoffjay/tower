---
type: Decision
title: Agent availability — deliver only to a free agent; runs claim their agents
description: Every assignment delivers only when the agent is free (pane idle/done, no open job, no post-run prompt, not claimed by another run) and reserves the job otherwise; --when-available goes. A workflow run claims every agent its definition names at start, all or nothing, and waits FIFO when one is claimed. Session save/resume is a follow-up.
tags:
  - decision
  - job-queue
  - dispatch
  - workflows
status: accepted
generated:
  by: omp/claude-opus-5-5
  at: "2026-11-10T18:30:00Z"
---

# Agent availability — deliver only to a free agent; runs claim their agents

Date: 2026-11-10. Accepted with the operator's answers (claim all agents
at run start; sessions are a follow-up). Amends the
[job-queue decision](job-queue.md), the
[scheduled-jobs decision](scheduled-jobs.md) (reserved delivery stops
being opt-in), the [workflows decision](workflows.md) (runs gain
`waiting`), and design
[§5.2.1, §5.2.2, §5.6](../concepts/design/05-core-objects.md),
[§6](../concepts/design/06-data-model.md),
[§7](../concepts/design/07-server-api.md),
[§9.4, §9.7](../concepts/design/09-server-modules.md),
[§10](../concepts/design/10-client-cli.md).

## Context

The operator assigned a job to an agent that was busy with a prompt, and
tower typed the job into the working pane. From the event log
(2026-11-09, UTC):

| Time | Event |
|---|---|
| 20:53:50 | operator prompt to `s1-rsb-comms`; pane `idle → working` |
| 21:07:17 | `task.assigned` by `me`; the delegation is typed at once |
| 21:07:51 | the turn ends early (`working → done`) |
| 21:09:31 | the agent releases the job |
| 21:18:12 | assigned again after a `/compact` |

Verified behavior before this decision:

- The one-job rule (D§5.2.2) counts only owned jobs and running post-run
  prompts. The assignment `UPDATE` never reads the pane state, so a
  prompt, or a turn that runs on after the agent reported its job, does
  not count as busy (`tasks::assign`).
- A job without a pre-run prompt is typed at once (`notify_assignee` →
  `deliver_prompt` → `driver.prompt`). Only the pre-run runner waits for
  the pane to settle (`hooks::run_prompt`).
- Reserved jobs already wait: the dispatcher delivers only to an
  `idle`/`done` row with no open job, on every move to `idle`/`done`, on a
  job close, and on the ~10 s sweep (`dispatch_for`). Reserving is opt-in
  (`--when-available`). The Herdr app's agent picker, the TUI `assign`
  verb, and `tower task assign` without the flag take the direct path.
- A workflow run uses an agent only while one of its steps owns a job.
  Between steps the agent is free, so a manual job, a schedule, or another
  run's step can take it. Nothing is interrupted, but the run waits, and
  an agent with one worktree (a builder) can have its branch switched
  under the run. Two runs of a review loop that share an author interleave
  their builds and fixes on that worktree.

## Decision

### 1. Deliver only to a free agent

An agent is **free** when all of these hold:

- its row is `idle` or `done` (not `working`, `blocked`, `launching`,
  `unknown`, or `dead`);
- it owns no open job (`assigned`, `working`, `input-required`);
- none of its closed jobs still runs a post-run prompt (`hook = 'post'`);
- no workflow run other than the job's own claims it (§2).

The assignment `UPDATE` checks every condition in the same statement, so
the check stays atomic. Every assignment — `POST /v1/tasks/{id}/assign`,
`POST /v1/tasks` with `assign`, `tower_task_assign`, `tower_task_create`,
the TUI, the Herdr app — delivers when the agent is free and otherwise
**reserves** the job for it (`target_agent_id`). The dispatcher delivers
it when the agent becomes free. The response says why it waits:
`{"task": …, "waiting": {"reason", "agent_state", "busy_with"?,
"held_by"?}}`; `waiting` is `null` when the job was delivered.

- Availability is never a conflict any more: a busy, claimed, or dead
  target gets a reservation. A dead target waits for its resume, as a
  reservation always could.
- Explicit assignment re-targets a job reserved for someone else and
  clears `not_before` (an explicit operator action still wins over
  `--at`), then joins the agent's own queue in the dispatcher's order
  (priority, then age).
- `when_available` is removed from the API, MCP, CLI (`--when-available`),
  TUI (`reserve` verb), and Herdr app (the "when available" toggle): every
  assignment means it. Old clients that still send it are ignored (the
  bodies accept unknown fields).
- `tower prompt` stays immediate. It is the operator's direct line, and
  typing while the agent works is how a harness is steered.
- Residual window: the row state lags the pane by the pump's latency
  (about a second). A prompt typed just before an assignment can still
  race it. The per-agent prompt lock keeps the two texts from
  interleaving.

### 2. A run claims its agents at start

A run **claims** every agent its definition can name — each node's
`agent`, and every value of an `agent = { from, map }` table — before its
first step, all at once (`run_claims`, one row per agent, the agent as
primary key: one run per agent). Agents that do not exist are skipped;
their steps fail as before.

- **All or nothing.** When another run claims any of them, or an earlier
  waiting run wants one of them, the run claims none and starts
  `waiting`. Waiting runs are admitted oldest first, so a later run never
  overtakes an earlier one for a shared agent. Atomic acquisition in one
  order can't deadlock.
- **Holding.** While run R claims agent X, the dispatcher gives X only
  jobs of R (`origin = 'workflow'`, `context_id = R`). Every other job
  reserved for X — manual, scheduled, another run's — waits until R ends.
  The claim does not need X to be idle: a job X owned before the claim
  finishes first, and R's step waits for it like any reserved job.
- **Release.** Ending a run (completed, failed, canceled) deletes its
  claims, admits the waiting runs that can now claim (oldest first), and
  only then delivers each free agent's next reserved job. A waiting run
  therefore gets a released agent before standalone work reserved for
  it. Removing an agent deletes its claim.
- **Waiting runs.** `RunState` gains `waiting` (before `running`). A
  waiting run has no steps. `tower run show` lists what it waits for
  (`waiting_for: [{agent, run_id}]`); `tower run cancel` ends it. A
  schedule treats a waiting run like a running one (skip with
  `previous_running`), and so do the "a run is open" checks (`workflow
  rm`, event-trigger dedupe, the list's open-run count).
- **Events.** `workflow.run.waiting` is emitted when a run starts waiting
  (payload of `workflow.run.started`, plus `waiting_for`).
  `workflow.run.started` is emitted when the run claims its agents and
  its first steps start — at creation, or when it leaves `waiting`.

### 3. Sessions are a follow-up

Tower can already relaunch a dead row into a named session
([D§8.4](../concepts/design/08-harness-layer.md): omp/claude
`--resume <id>`, pi/opencode `--session <id>`), and herdr reports the
pane's current session. Recording the session per (run, agent) and per
job, and resuming it on the next visit instead of `/clear`, would give
review loops their context back. It does not save worktree state, so it
complements §2 rather than replacing it. With §1 and §2 nothing is
interrupted any more, so sessions get their own decision after a
harness-by-harness check.

## Rejected

- **Conflict instead of reserving** (the old manual path): the operator
  asked for "queue it for later", and the reservation machinery already
  exists.
- **Hold from first use** (a run holds an agent from its first step on
  it): more parallel runs, but two runs that take agents in opposite
  orders deadlock, and breaking the cycle hands an agent out again.
- **Priority only** (a running run's jobs go first, no hold): other work
  still runs between steps.
- **Queueing `tower prompt` while busy**: it removes steering.

## Consequences

- Two runs that share any agent run one after another. A run holds an
  agent from its start even when that agent's step comes hours later
  (`kb-daily` holds `kb-curator` from 06:00). Standalone jobs for a
  claimed agent wait for the run.
- The tasks module reads `run_claims` and `context_id`, but still knows
  nothing of graphs (D§9.7).

## Plan

1. Design amendments (this commit): D§5.2.1, D§5.2.2, D§5.6, D§6
   (migration 0008: `run_claims`), D§7, D§9.4, D§9.7, D§10.
2. Server: availability in the assignment `UPDATE`, operator `assign`
   reserves when not free and reports `waiting`, `when_available` removed,
   dispatcher honors claims; run claims, `waiting` runs, admission,
   release; open-run checks include `waiting`; events registered.
3. Clients: CLI (`task assign/create` output, wizard delivery question,
   `run list/show/cancel`), TUI, Herdr app (jobs form and picker,
   workflows run state), the omp/pi `tower-tools` extension, skills,
   `docs/getting-started.md`.

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-11-10 | Integration tests: a busy agent's assign reserves with `busy_with`; an agent working on a prompt gets nothing typed until it goes idle, then the job; 16 concurrent assigns → 1 delivered, 15 reserved; dead target reserves; post-run mark reserves; routes answer `waiting` (`tests/dispatch.rs`, `lifecycle.rs`, `tasks.rs`) | pass |
| 2026-11-10 | Integration tests: a run's claimed agents refuse manual jobs (`held_by`) but take the run's steps; a sharing run waits with `waiting_for`, a later one queues behind it, an unrelated run starts at once; ending the first admits only the second; a released agent goes to the waiting run before the standalone job reserved for it; a waiting run cancels (`tests/workflows.rs`) | pass |
| 2026-11-10 | Live, throwaway `tower serve` + omp agent: assign during a `sleep 30` prompt → `reserved for avail-probe: avail-probe is working`; delivered by `dispatch` in the same second the turn ended (`working → done`), after the agent answered the prompt | pass |
| 2026-11-10 | Live CLI: `workflow run` prints `started` / `waiting`; `run show` lists `waiting for avail-probe (run …)`; manual assign → `claimed by run … (claim-a) until it ends`. The first try handed the released agent to the manual job before the waiting run; fixed (admit, then dispatch) and re-run: the waiting run took it, the manual job stayed held by it | pass after fix |
