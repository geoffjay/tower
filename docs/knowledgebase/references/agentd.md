---
type: Reference
title: agentd research
description: agentd's microservice architecture, what worked, what hurt, and the single-server simplification path for tower.
resource: https://github.com/geoffjay/agentd
tags:
  - reference
  - research
  - agentd
  - prior-art
status: stable
sources:
  - resource: git:340c189:research/agentd.md
    title: agentd research (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# agentd research

## What it is

agentd is the user's own project (`github.com/geoffjay/agentd`, source not on this
machine — architecture reconstructed from deployment configs and operational skills
in `/home/cap/Projects/abu/.claude/skills/agent-*/SKILL.md`, `.agentd/` config dirs
in abu and nemo, and abu's ADRs).

- Rust Cargo workspace, 15+ crates, shared `common` crate
- SeaORM + SQLite persistence; Prometheus + Grafana observability
- Deployed as a suite of independent HTTP microservices
- Agent-facing surface is the `agent` CLI, which routes to services over HTTP

## Services (the pain point)

| Service | Dev port | Prod port | Function |
|---|---|---|---|
| Ask | 17001 | 7001 | Registered checks, question/answer flows |
| Hook | — | 7002 | git/shell PreToolUse hooks |
| Monitor | — | 7003 | Monitoring daemon |
| Notify | — | 7004 | Alert bus (priorities, lifetimes, actionable) |
| Wrap | 17005 | 7005 | tmux session management for agents |
| Orchestrator | 17006 | 7006 | Agent lifecycle, workflows, approvals, policies |
| Memory | 17008 | 7008 | Vector-backed semantic knowledge store |
| Communicate | 17010 | 7010 | Room-based messaging (SQLite + WebSocket) |
| Index | — | 7012 | Referenced in nemo configs only |

Every agent YAML carries 8+ `AGENTD_*_SERVICE_URL` env vars so spawned agents can
find every service. A dedicated `agent status` command exists solely to check all
services before doing anything. Partial startup causes confusing partial failures.

## What works (keep these ideas)

- **Declarative YAML** in `.agentd/` (agents, workflows), applied with `agent apply` /
  `agent teardown`
- **Room-based messaging** with humans as first-class room members (`--kind human`)
- **Actionable notifications** requiring a response; UUID'd questions via the ask service
- **Approval gates**: tool policies (`AllowAll/DenyAll/AllowList/DenyList/RequireApproval`,
  5-minute approval timeouts)
- **Isolation options**: git worktrees, Docker with CPU/memory limits, per-agent env
- **Workflows**: orchestrator polls sources (github_issues, github_pull_requests, cron,
  webhook, manual) at 60s intervals, dispatching templated prompts
- **Harness abstraction**: `claude-code`, `crush`, `opencode` runtimes with providers
  (anthropic, openai, ollama)
- **tmux as the execution substrate** (wrap service)

## What hurts

1. **Nine microservices** with separate ports, health endpoints, startup ordering, and
   dev/prod port splits. This is the primary complaint.
2. **Service sprawl with overlapping responsibilities**: notify vs. ask vs. communicate
   are three separate message channels; wrap and orchestrator both manage tmux sessions.
3. **Configuration burden on agents**: every spawned agent needs the full service-map
   env vars, or it can't reach anything.
4. **CLI routing complexity**: the CLI must discover and route to each service.
5. **Ad-hoc coordination protocol**: agents coordinate via `[LOCK]`/`[UNLOCK]` message
   conventions in communicate rooms, enforced by PreToolUse hook scripts
   (e.g. `check-coordination.py`). Convention, not type safety.

## Simplification path for tower

Collapse everything into **one server binary**:

- One process: orchestrator + wrap + notify + ask + communicate + memory become
  internal modules, not services
- One SQLite database: rooms, notifications, and questions unify into a single
  `messages` table with different types; memory stays a table in the same DB
- One socket/port: HTTP + streaming multiplexed on a single endpoint
- One env var (or zero): the client and spawned agents learn the server address once
  (e.g. `TOWER_URL` or a well-known socket path), not eight
- The `agent` CLI becomes the thin **client** in the client/server model

This single-server shape is already proven locally by Hermes Agent
(`~/.hermes/`, `hermes-gateway.service`): one gateway process hosting dispatcher,
kanban (SQLite), Telegram chat, and dashboard — see
`/home/cap/Projects/factory/FACTORY_NOTES.md`.

## Reference material on this machine

- `/home/cap/Projects/abu/.claude/skills/agent-{apply,ask,communicate,memory,notify,ops,orchestrator,status,wrap}/SKILL.md`
- `/home/cap/Projects/abu/.agentd/` and `/home/cap/Projects/nemo/.agentd/`
- `/home/cap/Projects/abu/docs/planning/adr-002-workspace-structure.md` (15+ crates)
- `/home/cap/Projects/abu/docs/planning/adr-004-database-and-orm.md` (SeaORM/SQLite)
- `/home/cap/Projects/abu/docs/planning/adr-005-observability.md` (Prometheus/Grafana)
