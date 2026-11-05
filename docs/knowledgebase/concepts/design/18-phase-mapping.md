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
  at: "2026-10-31T12:00:00Z"
---

# 18. Phase mapping (→ execution plans)

| Phase | Scope | Exit criteria |
|---|---|---|
| 1. MVP core | core types, server shell (axum + SQLite + event log), HerdrDriver via CLI, agents spawn/prompt/read/wait, CLI verbs `ps/spawn/prompt/read/stream`, SSE `/v1/events` | one machine: spawn claude+pi via herdr, prompt both, stream output to terminal, states visible in `ps`; restart server, agents rebind |
| 2. Messaging + job queue | messages table + kinds, inbox, questions/approvals + sweeper, MCP endpoint, `blocked`→inbox flow, job queue (assign/start/heartbeat/release/status + lease sweeper + priorities/tags) | agent blocks on a question; it appears in inbox; answered via CLI or MCP; agent resumes; expiry path tested. A queued job is assigned to an agent; agent declares start, heartbeats, completes; a second assignment conflicts (race tested); a killed owner's job requeues within one lease window and another agent completes it after reassignment |
| 2b. Scheduled jobs | reserved delivery (dispatcher, one job per agent), `not_before`, recurring schedules (cron + tz, exactly-once, overlap/catch-up/expiry/removal policies), schedule API/CLI/MCP, `tower service` | a job reserved for a busy agent is delivered only when it goes idle; a schedule fires exactly once per occurrence across a restart, skips while the previous run is being worked, coalesces missed firings, and a live agent runs a scheduled job end to end |
| 2c. Run outcomes + standing brief + job hold | mandatory terminal-report results, open questions that hold the job open and stay answerable in the inbox, `agents.brief` stored and restated per delivery, report gate, inbox contract (expired questions visible), `tasks.held_at` hold with `previous_held` schedule skip | an agent completes a job with a reported result that persists on the record; a question asked at completion holds the job open, stays in the inbox, and is answerable after expiry; each job delivery restates the stored brief; a held reserved job is not dispatched while the target goes idle, is skipped instead of replaced by the next schedule firing, and is delivered on resume; a server restart loses neither the result nor the brief |
| 2d. Job lifecycle + editing | `pre_run`/`post_run` prompts on jobs and schedules, delivery sequencing (pre-run before the delegation, post-run after the owner's close, agent busy until it settles, lease paused during pre-run), `hook` mark + boot recovery, `PATCH` for jobs and schedules, CLI/MCP edit verbs, TUI edit panel | a job's pre-run prompt reaches the agent before its job prompt and the delegation waits until the agent settles; a closed job's post-run prompt runs before the next reserved job is delivered; a restart mid-prompt clears the mark and the lease recovers the job; a queued job's prompt, schedule, and pre/post-run prompts are edited from the TUI and each edit is saved as it is submitted; a delivered job refuses all but post-run |
| 3. TUI | Fleet/Agent/Inbox/Tasks/Events views, herdr attach action, queue banner | daily monitoring driven entirely from TUI |
| 4. Web UI (agent cloud) | Topcoat UI, agent-cloud view (color/size/halo/brightness channels), floating detail panel, queue bar, machine strip, event ribbon; SSE-fed | cloud shows all agents as colored points with live state changes for an hour soak: zero polling errors, blocked agents visibly pulse, selecting a point opens the side panel with live detail |
| 5. Multi-machine | node agent, machine hub, `/nodes` WS, machine registry, remote spawn, cross-machine assignment | agent runs on second machine, appears in local ps/TUI; node disconnect handles gracefully; a job queued on the coordinator is assigned to an agent on the remote machine; node loss mid-job requeues the lease |
| 6. A2A edge | agent card, `/a2a` send/stream, task mapping, foreign delegation in | external A2A client delegates a task to a named agent and streams it to completion |
| 7. Workflows | versioned workflow definitions (TOML, `apply`), runs and steps, job nodes with outcome routing, join any/all, bounded loops, summary + artifact hand-off with caps, workflow-job result contract, event-log consumer with durable cursor + reconciliation, schedule and event triggers, artifacts, workflow/run/artifact CLI + MCP | a run branches on the triage outcome, loops worker → reviewer until approval and stops at `max_visits`; a run fans out to two agents and joins; each step's job carries the earlier summaries and artifact ids; an invalid workflow-job report is refused; a schedule trigger starts runs exactly once and skips while its run is open; an event trigger starts a run and never triggers itself; a lost event and a restart mid-run end in the same run state; a live agent runs a two-step workflow end to end |

Each phase gets a detailed execution plan (task breakdown, spike resolutions,
test plan) derived from this document's relevant sections.
