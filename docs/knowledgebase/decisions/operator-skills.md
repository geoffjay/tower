---
type: Decision
title: Operator skills — agents operate tower through the CLI
description: Five repo-local agent skills (tower-queue-job, tower-agent-add, tower-agent-remove, tower-status, tower-deploy) let an AI agent operate tower for the user. They drive the CLI with --json. A stack is a naming convention, not a tower object. MCP gains stop, cancel, inbox, and schedule show so that it covers the same operations.
tags:
  - decision
  - skills
  - operator
  - mcp
status: accepted
---

# Operator skills

Date: 2026-09-27. Amends [§7](../concepts/design/07-server-api.md) (MCP
tools). Skills: `.agents/skills/tower-*/SKILL.md`, linked from
`.claude/skills/`.

## Context

The user wants an AI agent to set up and run tower work from a plain
description: a stack purpose, its agents, recurring jobs, and one-off jobs.
The CLI and MCP surfaces can do each step. The agent needs the procedure,
the conventions, and the checks that catch known failures.

## Decision

1. **Five skills.** `/tower-queue-job` (a job from an argument or an
   interview), `/tower-agent-add`, `/tower-agent-remove`, `/tower-status`
   (read-only), and `/tower-deploy` (interview, proposal, deploy, update,
   teardown).
2. **The skills drive the CLI** (`tower … --json`). Every harness with a
   shell can run them, and they need no MCP configuration. One procedure
   serves all harnesses.
3. **Repo-local.** The skills live in `.agents/skills/` with symlinks in
   `.claude/skills/`, like the `okf` and `ste` skills. They work when the
   agent runs in the tower repo. They refer to `docs/getting-started.md`
   and `docs/agent-loop.md` by path.
4. **A stack is a naming convention.** Agents are `<stack>-<role>`.
   Schedules are titled `<stack>: <title>`. Jobs and schedules carry the
   tag `stack:<stack>`. The skill reads the current stack from `tower ps`,
   `tower schedule list`, and `tower task list --tag`. Tower stores no
   stack record.
5. **Approval gates.** Each skill that changes state shows its plan and
   waits for the user. No skill answers inbox items for the user.
6. **MCP stays complete for these operations.** New tools: `tower_stop`,
   `tower_task_cancel`, `tower_inbox`, `tower_schedule_show`. Stop and
   cancel are operator-only. `tower_inbox` returns the caller's messages.

## Known failures the skills check

| Failure | Check in the skill |
|---|---|
| `tower` is not on `PATH` in agent panes | Briefs give the absolute path of the binary |
| Harness cannot reach its model (for example `401`) but shows `idle` | The brief asks for a `ready <name>` reply; the skill reads the pane |
| Startup dialog blocks a new agent | `blocked` state → show the inbox item to the user |
| Claude Code sandbox blocks `127.0.0.1` | `Operation not permitted` → ask the user to allow the command |
| Schedules fire only while the server runs | Deploy suggests `tower service install` |
| Removing an agent pauses its schedules | Teardown removes schedules before agents |

## Rejected alternatives

- **A stack file with `tower stack apply` / `down`.** It gives an
  idempotent diff and a reviewable record. It needs a new design section,
  a CLI verb, and tests. The user chose the skill-only form. The
  declarative fleet file stays a backlog item
  ([getting-started §4](../../getting-started.md)).
- **Skills installed by the binary (`tower skills install`).** They would
  work from any project. The user chose repo-local skills.
- **Skills that drive MCP.** Structured calls, but each agent needs MCP
  configured, and pi has no MCP.

## Consequences

- The convention is the only record of a stack. A renamed agent or a
  schedule with a wrong title drops out of its stack.
- A schedule cannot be edited. An update removes it and creates it again.
- The skills describe the current CLI. A CLI change must update them in
  the same commit.
