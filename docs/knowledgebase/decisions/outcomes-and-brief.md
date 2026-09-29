---
type: Decision
title: Run outcomes and standing brief — durable results, open questions that hold, purpose restated per job
description: A terminal job report requires a result; an open question holds the job open and stays answerable in the inbox after expiry; the agent's standing brief is stored on the agent row and restated with each job delivery. Harness hooks and a supervisor harness are deferred with revisit triggers.
tags:
  - decision
  - job-queue
  - messaging
  - context
status: proposed
generated:
  by: omp/glm-5.3
  at: "2026-09-29T15:40:24Z"
---

# Run outcomes and standing brief

Date: 2026-09-29. From dogfooding on a real project. Amends
[§5.2.1](../concepts/design/05-core-objects.md), [§5.3](../concepts/design/05-core-objects.md),
[§6](../concepts/design/06-data-model.md), [§7](../concepts/design/07-server-api.md),
[§9.3](../concepts/design/09-server-modules.md), [§10](../concepts/design/10-client-cli.md),
[§18](../concepts/design/18-phase-mapping.md). Plan:
[phase 2c](../plans/phase-2c-outcomes-and-brief.md).

## Context

Two problems surfaced while running real jobs.

### Problem 1 — end-of-run output is lost

- A job's `result` column is written only when the owner reports it
  (`tasks::report`, `crates/tower-server/src/tasks.rs`). In practice, agents
  end their run with prose in the pane instead. That prose lives only in
  `agent.output` events, and the sweeper prunes events after 14 days.
- A question that nobody answers expires after 5 minutes
  (`DEFAULT_DEADLINE_S`, `crates/tower-server/src/messaging.rs:36`). Every
  inbox surface filters `status=pending` (CLI, TUI, web badge). The question
  then disappears from view, although the row remains.
- The next job arrives as a prompt in the same pane. Nothing re-surfaces the
  prior run's result or its open questions.

### Problem 2 — the purpose is start-only

- The brief is the one-shot `--prompt` at spawn (`sessions::spawn`). It is
  not stored anywhere: `agents.config` is always `'{}'` and no code reads it.
- The delegation notice (`notify_assignee`) carries the job id, title,
  description, and the work-loop text. It does not carry the agent's purpose.
- The conversation accumulates across jobs in one pane. Harness compaction
  can drop the brief. After compaction, the agent no longer knows its role.

## Decision

### A — Outcomes (problem 1)

The inbox and the job record both have a role. This answers both questions
from dogfooding: use the inbox for questions, and use the job record for the
summary.

1. **The job record is the summary of record.** A terminal report
   (`completed`, `failed`) must carry a `result`. No schema change:
   `tasks.result` exists. The work loop already has the tool; the contract
   makes it mandatory.
2. **The inbox stays the question channel.** Questions remain message rows,
   linked to the job by `task_id` and shown in the task trail. They are not
   copied into `result`. One fact, one place.
3. **An open question holds the job open.** A terminal report is rejected
   (409, `question_open`) while the caller has a pending question. The job
   stays in a non-terminal state, and the one-job-per-agent rule keeps the
   next job away. An answer, or the question's expiry, lifts the gate. This
   gives the "block the agent while it waits on the human" protection
   mechanically, with no new state machine.
4. **Questions no longer vanish.** The default deadline for questions rises
   to 24 hours; approvals keep 5 minutes (an unattended permission is denied,
   not lingered on). The inbox shows every item that still needs the
   operator: pending items plus expired, unanswered questions. A response to
   an expired question is accepted and delivered late. An expired approval
   stays denied; a late response to it returns `conflict`.

### B — Standing brief (problem 2)

5. **The brief becomes a column on the agent** (`agents.brief`, migration
   0005). Spawn stores the `--prompt` into it. The dead `config` column is
   dropped. `tower brief <name>` shows it; `tower brief <name> '<text>'` sets
   it; `--send` also prompts the agent with it now; `--clear` removes it.
   Mutation is operator-only (`PATCH /v1/agents/{id}`). Reads flow through
   the normal agent JSON, so MCP's `tower_ps` carries it too.
6. **Each delivery restates the brief.** The delegation notice prepends
   "You are <name>. <brief>" when a brief is set, and omits it when not.
   Plain prompt text on the existing delivery path; no harness configuration.
7. **Restart-safe by data, not hooks.** The brief lives in SQLite, so a
   server restart, an adoption, or a fresh session cannot lose it. The next
   job delivery restates it. No per-project or user-wide harness config is
   needed for this purpose.
8. **Prior results are not injected into later jobs.** The next delegation
   carries the job and the brief only. Carrying results forward would grow
   the context without bound. The operator reads results from the record;
   an agent that needs earlier work re-reads the repository.

### Not adopted now

- **Harness hooks** (claude `SessionStart`, pi extensions): per-harness,
  per-machine configuration that duplicates the prompt delivery tower
  already owns. Revisit only if restatement is ever needed *inside* a job,
  which per-delivery text covers today.
- **A supervisor/controller harness** (pi wrapped as a custom agent type):
  the concept is sound, but it needs no new harness today. Tower's MCP and
  CLI surfaces already let an agent read jobs, questions, and results, and
  drive assignment. This decision adds the durable data such a supervisor
  would manage. Revisit when fleet self-governance is wanted (auto-assign,
  compaction control); it likely fits the orchestrator role from the
  [job-queue decision](job-queue.md) rather than a harness fork. Recorded as
  [design §17.9](../concepts/design/17-open-questions.md).

## Consequences

- Migration 0005: add `agents.brief TEXT`; drop `agents.config`.
- Delegation notice, sweeper expiry prompt, `docs/agent-loop.md`, and the
  `tower-agent-add` skill text change.
- Design amendments at execution: [§5.2.1](../concepts/design/05-core-objects.md)
  (report gate, delegation text), [§5.3](../concepts/design/05-core-objects.md)
  (deadlines, late answers), [§6](../concepts/design/06-data-model.md) (brief
  column), [§7](../concepts/design/07-server-api.md) (PATCH route, inbox
  query), [§9.3](../concepts/design/09-server-modules.md) (expiry semantics),
  [§10](../concepts/design/10-client-cli.md) (`tower brief`).
- No new event kinds: `message.status` and `task.status` already exist.
- New error codes: `question_open` (409) for a gated report;
  `result_required` (400) for a terminal report without a result.
