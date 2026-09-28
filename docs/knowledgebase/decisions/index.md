# Decisions

Decisions for the tower project.

* [Architecture recommendations](architecture-recommendations.md) - synthesis: single Rust server over herdr, SQLite, SSE, CLI/TUI/web/MCP, A2A edge; rejected alternatives.
* [IPC: inter-agent and human-agent communication](ipc.md) - one HTTP port, SSE streaming, MCP for agents, A2A shapes, unified messages table.
* [Cross-server communication](cross-server.md) - single coordinator with dial-home node agents; A2A between independent deployments.
* [Scheduled jobs](scheduled-jobs.md) - reserved delivery when an agent is free, `not_before`, cron schedules with skip / replace / coalesce / pause policies.
* [Operator skills](operator-skills.md) - five repo-local skills drive the CLI so an agent can operate tower; a stack is a naming convention; MCP gains stop, cancel, inbox, schedule show.
* [Job queue dispatch](job-queue.md) - assignment-only dispatch, no self-serve claims; lease+heartbeat liveness; orchestrator role + jev/laya router deferred.
* [Web UI](web-ui.md) - Topcoat bridged into the axum app under `/ui`, live regions over Topcoat's WebSocket, server-side layout, derived read-only UI token.
