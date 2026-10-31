---
type: Decision
title: Job lifecycle prompts and editable jobs
description: A job or schedule may carry a pre-run prompt (typed into the harness before the job prompt, on every delivery) and a post-run prompt (typed after the owner closes the job); the agent counts as busy until each one settles. Queued jobs and schedules become editable over PATCH, the CLI, MCP, and a TUI edit panel that saves each field as it is submitted.
tags:
  - decision
  - job-queue
  - scheduling
  - lifecycle
  - tui
status: accepted
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
---

# Job lifecycle prompts and editable jobs

Date: 2026-10-31. Builds on the [job-queue](job-queue.md) and
[scheduled-jobs](scheduled-jobs.md) decisions. Amends
[§4](../concepts/design/04-paths-and-configuration.md),
[§5.2.1, §5.2.2, new §5.2.3, §5.4](../concepts/design/05-core-objects.md),
[§6](../concepts/design/06-data-model.md),
[§7](../concepts/design/07-server-api.md),
[§9.4](../concepts/design/09-server-modules.md),
[§10](../concepts/design/10-client-cli.md),
[§11](../concepts/design/11-tui.md),
[§18](../concepts/design/18-phase-mapping.md). Plan:
[phase 2d](../plans/phase-2d-job-lifecycle.md).

## Context

- A job reaches its agent as one prompt: the delegation notice (title,
  description, work-loop text). Nothing runs around it. The operator wants
  harness commands around each run: `/clear` or `/compact` before it (a
  fresh or a compacted context), a self-improvement skill such as
  `/reflect` or `/learn` after it.
- Typing them by hand defeats reserved delivery and schedules — nobody is
  at the keyboard when the 09:00 job is delivered.
- A slash command is a command only as its own input, typed verbatim while
  the harness is idle. Inside the delegation text it is just words, and
  `/clear` would wipe the job prompt it arrived with.
- Jobs and schedules are database rows, and nothing changes them after
  creation except dispatch (assign, cancel, pause). Editing a schedule
  meant remove + create; a job could not be edited at all, and the TUI
  had no way to change either.

## Decision

### Lifecycle prompts

1. **Two optional texts per job: `pre_run` and `post_run`.** A schedule
   carries the same pair and copies it into every job it fires, like its
   title and description. They are set at creation (REST, CLI
   `--pre-run` / `--post-run`, MCP) and changed by an edit.
2. **Typed verbatim.** Each one goes to the harness as its own prompt with
   the text unchanged: `/compact` reaches the harness as `/compact`. Tower
   does not interpret the text, so any prompt works on any harness.
3. **Pre-run runs on every delivery.** When a job with a pre-run prompt is
   assigned (manually, by the dispatcher, or from a schedule), tower waits
   until the agent is idle, sends the pre-run prompt, waits until the agent
   settles again, and only then sends the delegation notice. The job prompt
   never overtakes the pre-run prompt. A requeued job runs its pre-run
   prompt again when it is delivered again.
4. **The lease pauses during pre-run.** The job stays `assigned` while its
   pre-run prompt runs. Lease expiry pauses (as it does in
   `input-required`) and the full lease starts when the delegation goes
   out — `/compact` can take longer than a 60 s lease.
5. **Post-run runs after the owner closes the job.** When the owner reports
   `completed` or `failed`, tower waits until the agent settles, sends the
   post-run prompt, and waits until that settles. Cancel, lease expiry, and
   release do not run it: the run did not end with the agent's report, and
   an expired lease usually means the agent is gone.
6. **The agent stays busy until post-run settles.** The one-job rule
   extends: an agent is busy while it owns an open job *or* one of its
   closed jobs runs its post-run prompt. The assignment `UPDATE` checks both
   in one statement, so a manual assign answers `conflict` and the
   dispatcher holds the next reserved job until the post-run prompt
   settles. Post-run work never interleaves with the next job.
7. **A lifecycle prompt never blocks the job.** If tower cannot send it, or
   the agent does not settle within `[jobs] hook_timeout_s` (default 600),
   the failure is recorded and the sequence continues: the delegation still
   goes out (pre-run), the agent is released (post-run).
8. **Visible.** While a lifecycle prompt runs, the job row carries
   `hook = pre | post`. `task.hook` events record `running`, `done`,
   `failed`, and `interrupted`. Each prompt is a `prompt` message row linked
   to the job, so it shows in the job detail and the agent's history.
9. **A restart abandons running lifecycle prompts.** Boot clears `hook` and
   records `task.hook` `interrupted`. A job that waited on its pre-run
   prompt falls back to its lease (expiry requeues it, and the next
   delivery runs the pre-run prompt again); a post-run prompt is dropped.

### Editing

10. **`PATCH /v1/tasks/{id}`** changes `title`, `description`, `pre_run`,
    `post_run`, and `not_before`; `null` or an empty string clears an
    optional field. A queued job accepts every field. A delivered job
    (`assigned`, `working`, `input-required`) accepts only `post_run` —
    the rest already reached the agent. A closed job accepts nothing
    (`conflict`). `not_before` needs a reservation, as at creation; a new
    time re-runs delivery at once.
11. **`PATCH /v1/schedules/{id}`** changes `title`, `description`,
    `pre_run`, `post_run`, and the cadence (`cron` or `daily`, and
    `timezone`). A new cadence recomputes `next_run_at` from now, with no
    catch-up. An edit applies to the jobs the schedule fires afterwards; an
    open occurrence keeps what it was fired with (edit the job to change
    it). This replaces "editing is remove + create" in the
    [scheduled-jobs](scheduled-jobs.md) decision.
12. **Operator-only, atomic, recorded.** MCP refuses agent callers. Each
    edit is a CAS on the state it was checked against, so a job assigned
    while it is being edited answers `conflict`. `task.updated` /
    `schedule.updated` events name the changed fields.
13. **Every surface.** CLI `tower task edit` / `tower schedule edit`; MCP
    `tower_task_edit` / `tower_schedule_edit`.
14. **TUI edit panel.** `e` on a job or a schedule opens a panel with its
    title, prompt, schedule (a job's `at` time, a schedule's cron), pre-run,
    and post-run. `enter` edits the selected field in the input line; `E`
    opens it in `$EDITOR` (a multi-line value always opens there). A field
    is saved when it is submitted: no save step and no unsaved state.

## Rejected alternatives

- **Commands inside the delegation text** — not commands there (see
  context), and `/clear` would erase the job it came with.
- **Harness hooks** (claude `SessionStart` / `Stop`, pi extensions) —
  per-harness, per-machine configuration. Tower owns delivery and already
  knows when a run starts and ends (same reasoning as the
  [outcomes-and-brief](outcomes-and-brief.md) decision).
- **Fire post-run on every close** — cancel, expiry, and release are not
  the end of the agent's own run.
- **New task states for the lifecycle phases** — a column is enough; new
  states ripple through the A2A mapping and every state check (the
  scheduled-jobs precedent).
- **An in-memory busy flag** — not atomic with the assignment CAS, and
  invisible to clients. The `hook` column is both.
- **A save step in the TUI** — unsaved state that can be lost; one PATCH
  per field is immediate and atomic.

## Not adopted now

- **Agent-level lifecycle prompts** (every job an agent gets): the operator
  decides per job and per schedule; revisit if the same pair ends up on
  every job.
- **Resuming interrupted lifecycle prompts after a restart**: the lease
  already recovers a job that waited on its pre-run prompt, and its next
  delivery runs the pre-run prompt again. A second recovery path adds
  nothing but cases to test.
