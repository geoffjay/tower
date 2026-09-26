---
type: Decision
title: tower — recommendations
description: "Architecture synthesis from the research: single Rust server over herdr, SQLite, SSE, CLI/TUI/web/MCP clients, A2A edge, and rejected alternatives."
tags:
  - decision
  - architecture
  - synthesis
status: stable
sources:
  - resource: git:340c189:RECOMMENDATIONS.md
    title: tower — recommendations (original, removed from repo root after ingest)
  - resource: ../references/agentd.md
  - resource: ../references/herdr.md
  - resource: ../references/openrig.md
  - resource: ../references/a2a.md
  - resource: ipc.md
  - resource: cross-server.md
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# tower — recommendations

Synthesized from the research ([agentd](../references/agentd.md), [herdr](../references/herdr.md), [openrig](../references/openrig.md), [A2A](../references/a2a.md), [IPC](ipc.md), [cross-server](cross-server.md)). Input to the [tower design](../concepts/design/), which refines these recommendations where they differ.


Proposed architecture, synthesized from the research (agentd's failure mode,
herdr's substrate, openrig's layering, A2A's protocol shapes).

## One sentence

A single Rust server that supervises agents (Claude Code, pi/ohmypi) through
herdr's socket API, persists messages/tasks/events in SQLite, streams
everything over SSE, and is consumed by: a CLI client, a read-only web UI,
a TUI that layers onto herdr, and an MCP endpoint for agents — with A2A at the
edge for foreign agents and remote deployments.

## Architecture

```
                    tower server (single binary, Rust)
   ┌───────────────────────────────────────────────────────────────┐
   │ HTTP: control API · SSE /events · SSE /agents/:id/stream        │
   │      MCP endpoint · A2A endpoint (+ agent card)                 │
   │ modules: inventory · sessions · messaging · tasks · policy     │
   │ SQLite (WAL): agents · messages · tasks · events · machines      │
   │ herdr driver (socket API): prompt · send-keys · read · wait     │
   └───────────────┬───────────────────────────────────────────────┘
                   │
        ┌──────────┼──────────────┬───────────────┐
     herdr      CLI client     web UI         node agents
   (PTYs,     (humans and    (read-only    (remote machines,
   detection)  agents)         monitoring)   SSH-dialed home)
```

### 1. Server: one process, one port, one database

The lesson from agentd: don't build services, build **modules**.

- HTTP + JSON-RPC namespaced routes on a single port (or Unix socket + TCP both)
- SQLite (WAL mode) for everything: agent inventory, messages (unified table:
  prompt/question/approval/notice/delegation/broadcast — not agentd's three
  overlapping channels), tasks, event log, machine registry
- All commands append to a **monotonic event log**; SSE streams are cursors
  over it (reconnect = replay from cursor; A2A's resubscription semantics)
- Server should start with zero configuration and be trivially
  systemd-socket-activated

### 2. Agent runtime: herdr as the execution engine

Do not own PTYs. Drive agents through herdr's socket API:

- `herdr agent start <name> --kind claude|pi --pane <id>` to provision
  (both target harnesses have first-class detection manifests)
- `agent prompt --wait` for input, `agent read` for output (feeds SSE streams),
  `agent wait --until` and detection-state subscriptions for lifecycle
- **Blocked state = human attention**: surface in web UI, TUI, and as
  question/approval messages
- Escapable: every agent is still in a tmux pane (`tmux attach`) — the openrig
  principle that raw terminals are the debugging hatch
- **Adoption first**: like `rig discover`/`rig adopt`, inventory existing herdr
  agents before demanding greenfield

If direct herdr API use proves limiting, fall back to spawning agents in own
tmux panes — but keep detection rules herdr-shaped.

### 3. Harness adapters

- `claude-code`: `--permission-mode acceptEdits` default (openrig's choice);
  explicit opt-in for `--dangerously-skip-permissions`
- `pi` (ohmypi): RPC-runner-in-pane model (openrig's Pi adapter is prior art;
  note herdr's `pi` kind may provide most of it via pane detection)
- Adapter trait minimal: `start`, `prompt`, `interrupt`, `read`, `state`.
  New harnesses = mostly a detection manifest, not code

### 4. Client: one CLI for humans and agents

- The `agent` CLI reimagined: `tower ps`, `tower prompt <agent> 'msg'`,
  `tower ask`, `tower approve <id>`, `tower stream <agent>`,
  `tower spawn --kind claude`
- Same API surface exposed over MCP so agents self-organize (openrig pattern)
- Interactive mode = the TUI

### 5. TUI: layer on herdr, don't compete

- A herdr client (via its socket API) that renders **coordination state**
  (agents, tasks, messages, blocked alerts) alongside herdr's own panes —
  the exact split openrig chose: "the TUI shows the team's coordination state;
  herdr and cmux show the actual agent terminals"
- Think `rig tui`-style topology/status views + message/queue panes, driving
  the same server API
- Terminal presentation stays in herdr; tower never re-implements PTYs

### 6. Web UI: monitoring only

- Subscribes to SSE (`/events` + per-agent streams); renders agent states,
  output tails, message/task history, machine inventory
- **No mutation routes** (or auth-gated and off by default) — config and control
  happen through CLI/TUI, per the goal
- Static SPA served by the server; no build pipeline on the server side if
  avoidable

### 7. Streaming: SSE everywhere

- One-directional, HTTP-native, auto-reconnect, A2A-aligned
- Event types A2A-shaped: `task-status-update` (state transitions),
  `artifact-update` (chunked output), plus `message`, `notice`
- WebSockets rejected: no bidirectional need (input is a POST), simpler infra

### 8. Cross-server: coordinator + node agents + A2A edge

- One coordinator owns the DB; remote machines run node agents that dial home
  (SSH-tunneled or outbound websocket) and run local herdr — factory notes'
  option 1, avoiding network-FS SQLite
- Node down → its agents marked unreachable; coordinator down → herdr keeps
  agents alive locally (agents survive coordination loss by design)
- **A2A at the edge**: Agent Card at `/.well-known/agent-card.json`, tasks/
  messages/SSE for foreign agents delegating in; inter-agent messages use
  A2A's Message/Part/Artifact shapes internally so the edge is a thin adapter

### 9. Durability & lifecycle

- Every message/task/event is a row before delivery (agentd, herdr kanban, and
  openrig's queue all converge on this)
- Snapshot/restore of topology (openrig's `rig down --snapshot` / `rig up
  <name>` is the model); agent sessions themselves survive via herdr
- Approval gates with timeouts (agentd's RequireApproval) as message kinds
- Worktree isolation for coding agents (agentd + factory conventions both do
  this)

## Stack recommendation

- **Rust** (agentd lineage, a2a-rs types, single static binary; axum or hono-
  equivalent = axum/actix, tokio, sqlx+SQLite, serde JSON-RPC)
- Alternative if speed-to-first-demo matters: TypeScript monorepo (openrig-
  style, Hono + better-sqlite3 + MCP SDK), accepting a Node runtime
- Web UI: server-rendered + htmx **or** tiny React/Vite SPA; read-only
- TUI: ratatui over the herdr socket + tower HTTP API

## Phasing

1. **MVP**: server + SQLite + herdr driver (spawn/prompt/read/wait) + CLI +
   SSE events. One machine, claude + pi.
2. **Messaging**: unified messages table, questions/approvals, blocked-state
   alerts, MCP endpoint.
3. **TUI**: herdr-overlay with topology/status/messages views.
4. **Web UI**: read-only dashboard over SSE.
5. **Multi-machine**: node agents + machine registry.
6. **Edge**: A2A endpoint + Agent Card, external delegation.

## Explicitly rejected (with reasons)

- Microservices (agentd's failure)
- WebSocket streams (no bidirectional need; SSE is simpler and A2A-aligned)
- Owning PTYs/terminal emulation (herdr exists; 22 harnesses solved)
- Shared SQLite over network FS (factory notes' known-bad option)
- Config/control via web UI (goal says monitoring only)
- Building an agent runtime from scratch (harness adapters only)
