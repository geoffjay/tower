---
okf_version: "0.2"
---

# tower knowledge base

This is the working knowledge base for the tower project, conforming to the
[Open Knowledge Format (OKF) v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md).

It consolidates working knowledge about the project: what tower is, how it is
structured, decisions and their rationale, recurring patterns, and plans.
It is authored by people and agents and meant to be read by both.

## For agents (policy)

This section is the single source of truth for how agents should use this knowledge
base. Tooling injects it into context automatically, so it does not depend on
`CLAUDE.md`/`AGENTS.md` being picked up.

**Consult before acting.** Before working on a task, scan the entries below and
read any concept/decision/pattern doc relevant to what you are about to change.
Prefer the recorded decision or pattern over re-deriving one. This index is the
map; read the specific doc on demand rather than guessing.

**Update after acting.** Update the knowledge base when a change would make an
existing entry wrong or leave a new fact unrecorded. In particular:

* A new architectural decision or a change to startup/threading →
  add or update a [decision](decisions/index.md) and relevant concept docs.
* A new recurring convention → add a [pattern](patterns/index.md).
* A new concept or architectural understanding → add a [concept](concepts/index.md).
* A forward-looking plan or roadmap item → add a [plan](plans/index.md).
* An external source or spec referenced by the KB → add a [reference](references/index.md).

Concept docs require YAML frontmatter with a `type` field; `index.md` and
`log.md` are reserved. When you add a doc, add a one-line pointer to the matching
category index below and a line to [`log.md`](log.md). If you deliberately decide
*not* to record a change, that is fine — the policy is judgement, not a mandate
to touch the KB on every edit.

Wired agents: Claude Code via a SessionStart hook (`.claude/hooks/kb-inject.py`) plus PostToolUse/Stop reminders (`.claude/hooks/kb-reminder.py`), opencode via the `instructions` config in `.opencode/opencode.jsonc`, and oh-my-pi via the `.omp/extensions/kb-hooks.ts` extension.

## Concepts

* [Design](concepts/design/) - tower design document (draft v0.1), one concept per section; cited as `D§N`.

## Decisions

* [Architecture recommendations](decisions/architecture-recommendations.md) - synthesis: single Rust server over herdr, SQLite, SSE, CLI/TUI/web/MCP, A2A edge; rejected alternatives.
* [IPC: inter-agent and human-agent communication](decisions/ipc.md) - one HTTP port, SSE streaming, MCP for agents, A2A shapes, unified messages table.
* [Cross-server communication](decisions/cross-server.md) - single coordinator with dial-home node agents; A2A between independent deployments.
* [Scheduled jobs](decisions/scheduled-jobs.md) - reserved delivery when an agent is free, `not_before`, cron schedules with skip / replace / coalesce / pause policies.
* [Operator skills](decisions/operator-skills.md) - five repo-local skills drive the CLI so an agent can operate tower; a stack is a naming convention.
* [Job queue dispatch](decisions/job-queue.md) - assignment-only dispatch, no self-serve claims; lease+heartbeat liveness; orchestrator role + jev/laya router deferred.
* [Web UI](decisions/web-ui.md) - Topcoat inside the server under `/ui`, live regions over its WebSocket, server-side layout, derived read-only UI token.

## Patterns

* _(empty — add pattern docs here)_

## Plans

* [Execution plans overview](plans/overview.md) - plan structure, dependency graph, cross-cutting rules, status table.
* [Phase 1 — MVP core](plans/phase-1-mvp-core.md) - complete; herdr driver, server, CLI, restart rebind.
* [Phase 2 — Messaging + job queue](plans/phase-2-messaging-task-pool.md) - complete; messages, blocked inbox, assignment-only job queue, MCP.
* [Phase 2b — Scheduled jobs](plans/phase-2b-scheduled-jobs.md) - complete; reserved delivery, one job per agent, recurring cron schedules, `tower service`.
* [Phase 3 — TUI](plans/phase-3-tui.md) - daily monitoring from the TUI.
* [Phase 4 — Web UI](plans/phase-4-web-ui.md) - complete; read-only agent cloud via Topcoat at `/ui`, scoped UI token, event retention.
* [Phase 5 — Multi-machine](plans/phase-5-multi-machine.md) - remote nodes, single coordinator, cross-machine assignment.
* [Phase 6 — A2A edge](plans/phase-6-a2a-edge.md) - Agent Card + A2A delegation.

## References

* [A2A protocol research](references/a2a.md) - Agent2Agent concepts and interaction modes; adopted internally, exposed at the edge.
* [agentd research](references/agentd.md) - prior project's microservice sprawl and the single-server simplification path.
* [herdr research](references/herdr.md) - herdr 0.8.2 socket API, agent launch, detection manifests; tower's execution substrate.
* [openrig research](references/openrig.md) - daemon/adapter architecture, herdr provider split, concepts to borrow, cautions.
* [OKF spec](references/okf-spec.md) - pointer to the Open Knowledge Format v0.2 specification.
