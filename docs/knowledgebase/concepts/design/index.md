# tower design

The tower design document (draft v0.1), one concept per section. Code, plans, and commits cite sections as `D§N` (e.g. `D§5.2.1` → §5 below, subsection 5.2.1).

* [§1 Purpose and scope](01-purpose-and-scope.md) - What tower is, target harnesses, goals, non-goals, and the naming note.
* [§2 Stack](02-stack.md) - Rust single-binary stack choices, subcommand binary layout, and crate workspace.
* [§3 Process and deployment model](03-process-and-deployment.md) - tower serve is the only stateful process; clients and node relays are stateless.
* [§4 Paths and configuration](04-paths-and-configuration.md) - XDG paths, config.toml keys and defaults, and tower doctor checks.
* [§5 Core objects and state machines](05-core-objects.md) - Agent, Task (job queue with assign/lease/heartbeat), Message, Event, and Machine objects and their state machines.
* [§6 Data model (DDL)](06-data-model.md) - SQLite schema for machines, agents, tasks, messages, artifacts, and events; single-writer discipline.
* [§7 Server API](07-server-api.md) - REST control/query routes, SSE streams, MCP tools, the A2A edge, and auth conventions.
* [§8 Harness layer](08-harness-layer.md) - Harness adapter trait, HerdrDriver and TmuxDriver, Claude Code and pi specifics, harness discovery and adoption.
* [§9 Server modules](09-server-modules.md) - Responsibilities of the inventory, sessions, messaging, tasks, machine hub, and event bus modules.
* [§10 Client CLI](10-client-cli.md) - tower CLI verbs, all thin over /v1, with --json everywhere.
* [§11 TUI](11-tui.md) - ratatui client views and keybindings; tower shows coordination state, herdr shows terminals.
* [§12 Web UI (read-only monitoring)](12-web-ui.md) - Agent-cloud visualization, supporting widgets, Topcoat technology choice, and drill-down backlog.
* [§13 Security](13-security.md) - Socket, TCP, node, and A2A authentication; secret handling; file permissions.
* [§14 Reliability and failure modes](14-reliability.md) - Behavior under server, herdr, node, coordinator, SSE, lease, assign-race, and DB-write failures.
* [§15 Observability](15-observability.md) - Logs, event log as audit surface, /healthz contents, optional metrics.
* [§16 Testing](16-testing.md) - Unit, integration (FakeHarness), E2E smoke, and schema contract testing strategy.
* [§17 Open questions](17-open-questions.md) - Unresolved design questions and the phase that resolves each.
* [§18 Phase mapping](18-phase-mapping.md) - Per-phase scope and exit criteria that the execution plans derive from.
