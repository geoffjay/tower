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

### Notes

- Phase 1 complete; pi provider auth on this machine is pending (S1.B) —
  does not affect driver/spawn/monitoring paths.