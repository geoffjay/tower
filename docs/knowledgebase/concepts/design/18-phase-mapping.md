---
type: Concept
title: Design §18 — Phase mapping
description: Per-phase scope and exit criteria that the execution plans derive from.
tags:
  - design
  - design-s18
  - phases
  - roadmap
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §18 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 18. Phase mapping (→ execution plans)

| Phase | Scope | Exit criteria |
|---|---|---|
| 1. MVP core | core types, server shell (axum + SQLite + event log), HerdrDriver via CLI, agents spawn/prompt/read/wait, CLI verbs `ps/spawn/prompt/read/stream`, SSE `/v1/events` | one machine: spawn claude+pi via herdr, prompt both, stream output to terminal, states visible in `ps`; restart server, agents rebind |
| 2. Messaging + job queue | messages table + kinds, inbox, questions/approvals + sweeper, MCP endpoint, `blocked`→inbox flow, job queue (assign/start/heartbeat/release/status + lease sweeper + priorities/tags) | agent blocks on a question; it appears in inbox; answered via CLI or MCP; agent resumes; expiry path tested. A queued job is assigned to an agent; agent declares start, heartbeats, completes; a second assignment conflicts (race tested); a killed owner's job requeues within one lease window and another agent completes it after reassignment |
| 2b. Scheduled jobs | reserved delivery (dispatcher, one job per agent), `not_before`, recurring schedules (cron + tz, exactly-once, overlap/catch-up/expiry/removal policies), schedule API/CLI/MCP, `tower service` | a job reserved for a busy agent is delivered only when it goes idle; a schedule fires exactly once per occurrence across a restart, skips while the previous run is being worked, coalesces missed firings, and a live agent runs a scheduled job end to end |
| 3. TUI | Fleet/Agent/Inbox/Tasks/Events views, herdr attach action, queue banner | daily monitoring driven entirely from TUI |
| 4. Web UI (agent cloud) | Topcoat UI, agent-cloud view (color/size/halo/brightness channels), floating detail panel, queue bar, machine strip, event ribbon; SSE-fed | cloud shows all agents as colored points with live state changes for an hour soak: zero polling errors, blocked agents visibly pulse, selecting a point opens the side panel with live detail |
| 5. Multi-machine | node agent, machine hub, `/nodes` WS, machine registry, remote spawn, cross-machine assignment | agent runs on second machine, appears in local ps/TUI; node disconnect handles gracefully; a job queued on the coordinator is assigned to an agent on the remote machine; node loss mid-job requeues the lease |
| 6. A2A edge | agent card, `/a2a` send/stream, task mapping, foreign delegation in | external A2A client delegates a task to a named agent and streams it to completion |

Each phase gets a detailed execution plan (task breakdown, spike resolutions,
test plan) derived from this document's relevant sections.
