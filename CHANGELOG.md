# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace scaffold (6 crates: core, driver, server, client, tui, web)
- CI workflow (fmt, clippy -D warnings, test, build)
- `tower serve` / `tower node` CLI dispatch (stubs until milestones land)
- Core types: agents/tasks/messages/events with A2A-shaped states
- SQLite schema (machines, agents, tasks, messages, artifacts, events)
- Event log with monotonic cursor + SSE streaming (`/v1/events`)
- Server shell: axum dual-transport (unix socket + TCP), bearer auth,
  `/healthz`, `/v1/schema` contract registry
- Harness trait + herdr CLI driver (spawn/prompt/read/wait/stop) + event pump
- Inventory reconcile with adoption (discover/adopt split), sessions
  module, agent control routes (`/v1/agents/*`)
- FakeHarness scripted driver for integration tests
- CLI verbs: ps, spawn, prompt, read, stream, interrupt, stop, doctor, schema
- E2E smoke script (`scripts/e2e.sh`): spawn → prompt → read → restart →
  rebind → seat survival
- `tower tui` (phase 3): fleet with queue banner and state/machine
  filters, agent detail (live ANSI screen, message history, prompt,
  interrupt, jump to the herdr pane), inbox with inline approve/deny/answer,
  jobs (live lease countdowns, queue with reservations, recent, schedules,
  trail), events (filters, follow), machines; command palette; follows
  `/v1/events` with cursor resume across server restarts
- `GET /v1/messages?agent=<name|id>`: an agent's message history
- Web UI (phase 4) at `/ui`: the agent cloud — agents as SVG points
  (state = color, activity = size, needs-you = amber pulse, recent fault =
  red ring, health = brightness), clustered by machine; queue bar, machine
  strip, event ribbon; floating panel with job, live lease countdown,
  message rate, sparkline, and output snippet. Topcoat 0.9 (pinned) inside
  the same server and port; live over Topcoat's WebSocket, reconnects by
  itself after a server restart
- `tower ui`: prints a login link; the browser gets a derived, read-only UI
  token that opens only `/ui` (`GET /v1/ui/token` for scripts)
- Event retention: the sweeper deletes events older than
  `event_retention_days` (default 14, `0` keeps all) hourly, never past an
  open `/v1/events` stream

### Fixed

- `EventKind::as_str` leaked a string per call (SSE filtering per event)
- Unset agent `pane_id`/`workdir`/`worktree` were returned as `""`
- SSE streams (`tower stream`) were cut after 65s by the request timeout
- `POST /v1/agents/{id}/prompt` now records the `prompt` message (history)

### Notes

- Phase 1 complete; pi provider auth on this machine is pending (S1.B) —
  does not affect driver/spawn/monitoring paths.