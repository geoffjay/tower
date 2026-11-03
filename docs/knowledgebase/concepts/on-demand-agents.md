---
type: Concept
title: On-demand agents (undecided)
description: Open question, not a decision - should tower keep named agents running and idle until a job arrives, or define them without launching and start one when a job is due? Current behavior, the trade-offs, middle-ground shapes and the questions to settle first.
tags:
  - agents
  - jobs
  - dispatch
  - lifecycle
  - open-question
status: draft
generated:
  by: omp/claude-opus-5-5
  at: "2026-11-03T00:00:00Z"
sources:
  - resource: crates/tower-server/src/tasks.rs
    title: dispatch_for / dispatch_all (reserved delivery)
  - resource: crates/tower-server/src/sessions.rs
    title: spawn, resume/continue, stop
---

# On-demand agents (undecided)

**Status: open.** This records a question being thought about, not a
direction. Nothing here is design ([D§](design/)) or a decision
(`decisions/`). If it settles, the outcome goes into a decision doc and
the affected design sections first, per `AGENTS.md`.

The question: an agent today must be running and `idle` before a job can
reach it. Would it be better to **define** a named agent without launching
it, and have tower **start it when a job for it is due**?

Both sides look right in different cases. Always-on is simple and
immediate. On-demand looks better for most of the fleet, which spends most
of its time waiting.

## What tower does today

- **Definition and launch are one step.** `tower spawn` inserts the agent
  row and starts the harness in the same call (`sessions.rs`, `spawn`).
  There is no way to create a named agent that is not running.
- **Only running agents get jobs.** The dispatcher delivers a reserved job
  only when its target's row is `idle` or `done` and it owns no open job
  (`tasks.rs`, `dispatch_for`; [D§5.2.2](design/05-core-objects.md)). It
  runs when the target becomes `idle`/`done`, when its job closes, and on
  the ~10 s sweep.
- **Untargeted jobs are never auto-assigned.** They wait for
  `tower task assign`. Picking an agent for them is the orchestrator/router
  role, still open question 8 in
  [D§17](design/17-open-questions.md).
- **A stopped agent keeps its identity.** `tower stop <name>` (without
  `--remove`) marks the row `dead` and clears its pane, but the row keeps
  its id, jobs and schedules. Reserved jobs for it wait
  ([scheduled jobs decision](../decisions/scheduled-jobs.md): "dead /
  blocked / busy -> waits"). Nothing restarts it:
  `tower spawn <name> --continue` / `--resume` does, by hand
  ([D§9.2](design/09-server-modules.md)), and the waiting job is delivered
  once it reaches `idle`.
- **`desired_state` is unused.** The agents table has
  `desired_state TEXT ... -- running | stopped`
  ([D§6](design/06-data-model.md)) and the TUI shows "stopped", but no
  server code writes `stopped` and the dispatcher ignores the field.

So most of the parts for on-demand already exist: a `dead` row that keeps
its jobs, `resume`/`continue` that relaunch it in place, and reserved jobs
that wait. The missing part is the dispatcher starting the agent itself.

## The fleet this would apply to (2026-11-03)

- 8 agents, all `omp`, all `idle` at the time of writing.
- 5 schedules, each targeted at one agent, each firing at
  `0 9,11,13,15,17 * * MON-FRI`: 5 runs per agent per weekday. None sets
  `pre_run` or `post_run`, so an agent's context carries from one run to
  the next.
- 284 completed jobs: median 2.2 min, p90 24 min from creation to last
  update. This includes queue wait, so actual run time is shorter.
- 40 processes matching `omp` used 5.4 GB RSS together. That includes
  sessions outside the fleet, so the cost per idle agent was not measured.

On these numbers, a typical agent works for minutes, five times a day, and
sits idle for the rest of the day.

## The two models

| | Always-on (today) | On-demand |
|---|---|---|
| Job start | Immediate | Harness launch plus time to reach `idle` (seconds; not measured) |
| Idle cost | A harness process and herdr pane per agent; no model tokens while idle | None |
| Context between jobs | Carries over unless `pre_run` is `/clear` | A choice: fresh start, or `--continue` the last session |
| Failure modes | Agent dies: jobs wait, operator restarts it | Launch can also fail: needs retry, backoff and a limit, or a job waits on an agent that never comes up |
| Watching | The pane is always there to look at | The pane appears for the job and goes away after |
| Direct prompts | `tower prompt` reaches it any time | A stopped agent must start first, be refused, or queue the prompt |
| Simplicity | No new states or loops | New lifecycle loop in the server |

## Middle-ground shapes

These are not exclusive.

1. **Per-agent policy.** Each agent is `always` or `on-demand`. Busy,
   interactive agents stay up; scheduled specialists start when needed.
2. **Idle timeout as the one setting.** Every agent starts on demand and
   stops after `idle_stop_s` with nothing reserved. A timeout of "never" is
   always-on; 0 is strict on-demand. One setting covers both models, and
   a long timeout keeps an agent warm between closely spaced jobs.
3. **Pre-warm for schedules.** Start a schedule's target shortly before
   the firing (the scheduler knows `next_run_at`), so the job sees no
   launch delay and the agent still stops afterwards.
4. **Stop only.** Keep launch manual, but let tower stop idle agents after
   a timeout. Smallest change, least benefit: someone still has to start
   them.

## What on-demand would need

If on-demand were chosen, roughly:

- **Define without launching.** A verb or flag that writes the row with
  the launch spec (kind, model, args, workdir, worktree) and
  `desired_state = 'stopped'`, `state = 'dead'`.
- **The dispatcher launches.** For a due reserved job whose target is
  stopped: launch once (a per-agent "starting" mark stops the sweep from
  launching twice), wait for `idle`, then the normal assignment `UPDATE`
  runs. Leases start at delivery, so launch time does not use up the lease.
- **Launch failure counting.** Charge failed launches to the job's
  `max_attempts` or a separate limit; on exhaustion fail the job and raise
  an inbox item.
- **Stop after.** After the job and its `post_run` finish, with nothing
  else reserved and the agent not `blocked`, stop it and keep the row.
- **Remote machines.** Launch goes through the agent's machine (node hub,
  [D§9.5](design/09-server-modules.md)), so an unreachable node is one
  more launch failure.

## Questions to settle first

1. **Memory between runs.** Should a scheduled agent remember its previous
   run (an issue allocator knowing what it allocated two hours ago), or
   start clean? Today it remembers, because nothing clears it. On-demand
   forces an explicit choice: fresh launch, `--continue`, or `--resume`.
2. **Which agents are which.** Is the distinction "scheduled specialist vs
   interactive collaborator"? Does any agent need to stay up because a
   person talks to it directly?
3. **Launch latency tolerance.** Is a launch delay before each job fine for
   scheduled work? It probably is for jobs that run on a 2-hour cadence; is
   it for jobs someone queues and watches?
4. **Blocked agents.** An on-demand agent waiting on an inbox answer must
   not be stopped by an idle timeout. Its job's lease already pauses
   (`blocked` moves the job to `input-required`, `tasks.rs`
   `sync_agent_state`); does `blocked` also hold the agent up indefinitely?
5. **Direct prompts to a stopped agent.** Start it, refuse, or hold the
   prompt as a message until it is next up?
6. **Worktrees.** A persistent worktree per agent (today), or a fresh one
   per job when the agent starts per job?
7. **Visibility.** Panes appearing and disappearing in herdr and in the
   Herdr app's Agents panel: is that useful (activity at a glance) or
   noise?
8. **Untargeted jobs.** On-demand only covers jobs reserved for a named
   agent. Does this change the shape of the future orchestrator (it could
   pick a *definition* and have tower start it), and should the two be
   decided together?

## Related

- [D§5.2.1-5.2.3](design/05-core-objects.md): job queue, reservations,
  lifecycle prompts.
- [D§9.1-9.2](design/09-server-modules.md): inventory reconcile, sessions
  (spawn, resume, continue).
- [Scheduled jobs decision](../decisions/scheduled-jobs.md): what a
  reserved job does when its target is dead, busy or removed.
- [Job queue decision](../decisions/job-queue.md): the reserved
  orchestrator role.
