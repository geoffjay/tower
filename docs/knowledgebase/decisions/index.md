# Decisions

Decisions for the tower project.

* [Architecture recommendations](architecture-recommendations.md) - synthesis: single Rust server over herdr, SQLite, SSE, CLI/TUI/web/MCP, A2A edge; rejected alternatives.
* [IPC: inter-agent and human-agent communication](ipc.md) - one HTTP port, SSE streaming, MCP for agents, A2A shapes, unified messages table.
* [Cross-server communication](cross-server.md) - single coordinator with dial-home node agents; A2A between independent deployments.
* [Run outcomes and standing brief](outcomes-and-brief.md) - mandatory durable job results, open questions that hold the job and stay answerable in the inbox, a stored standing brief restated per job delivery; hooks and supervisor harness deferred.
* [Job hold](job-hold.md) - a held flag on queued jobs: dispatch skips them, assign refuses them, resume delivers; held schedule occurrences skip instead of replace; assign gains `--at`.
* [Scheduled jobs](scheduled-jobs.md) - reserved delivery when an agent is free, `not_before`, cron schedules with skip / replace / coalesce / pause policies.
* [Job lifecycle prompts and editable jobs](job-lifecycle-hooks.md) - pre-run / post-run prompts typed verbatim around each run, agent busy until they settle, lease paused during pre-run; jobs and schedules edited in place (PATCH, CLI, MCP, TUI panel saving per field).
* [Operator skills](operator-skills.md) - five repo-local skills drive the CLI so an agent can operate tower; a stack is a naming convention; MCP gains stop, cancel, inbox, schedule show.
* [Job queue dispatch](job-queue.md) - assignment-only dispatch, no self-serve claims; lease+heartbeat liveness; orchestrator role + jev/laya router deferred.
* [Web UI](web-ui.md) - Topcoat bridged into the axum app under `/ui`, live regions over Topcoat's WebSocket, server-side layout, derived read-only UI token.
* [Multi-domain operation](multi-domain-operation.md) - accepted: phased build — herdr plugin (switcher + overview board), tower-primed pi agent via launch recipe, pi-durable deferred; control-session workflow with one tower server driving multiple herdr sessions (driver session binding + multi-session inventory as prerequisites).
* [Workflows](workflows.md) - accepted, built in phase 7: versioned graphs of job nodes, outcome-routed edges with fan-out, join and guarded loops, bounded summaries and artifacts handed to the next step, manual / schedule / event triggers, an engine that follows the events table from a durable cursor; A2A not needed inside one deployment; exec nodes and on-demand agents are follow-ups.
* [Agent availability](agent-availability.md) - accepted: every assignment delivers only to a free agent (pane idle/done, no open job, no post-run prompt, no other run's claim) and reserves the job otherwise; `--when-available` removed; a workflow run claims every agent it names at start, all or nothing, and waits oldest first; session save/resume is a follow-up.
