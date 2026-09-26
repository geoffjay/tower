---
type: Concept
title: Design §2 — Stack
description: Rust single-binary stack choices, subcommand binary layout, and crate workspace.
tags:
  - design
  - design-s2
  - stack
  - rust
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §2 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 2. Stack

Rust, single binary. Rationale: agentd lineage, static deployable binary, a2a-rs
wire types, mature async stack.

| Concern | Choice |
|---|---|
| Runtime | tokio |
| HTTP | axum (REST control + SSE + MCP + A2A all on one router) |
| Database | SQLite via sqlx, WAL mode, busy_timeout=5s |
| IDs | ULID (sortable, time-ordered) |
| Time | unix epoch milliseconds (INTEGER columns) |
| JSON | serde / serde_json |
| CLI parsing | clap |
| TUI | ratatui + crossterm |
| MCP | rmcp (or hand-rolled streamable HTTP) |
| A2A | a2a-rs types, custom axum routes |
| Logs | tracing + tracing-subscriber (JSON) |
| Static web assets | rust-embed for icons/fonts; UI itself is Topcoat server-rendered ([§12.3](12-web-ui.md)) |

## Binary layout

One binary, `tower`, dispatched by subcommand (herdr's shape):

```
tower serve          # the server (foreground; systemd unit provided)
tower node           # remote-machine agent (dials coordinator)
tower tui            # interactive TUI (client)
tower <verbs>...     # CLI client verbs (see §13)
```

Client and server share one crate workspace:

```
tower/
  crates/
    tower-core      # types: Agent, Task, Message, Event, states, errors
    tower-server    # axum app, modules, herdr driver, node hub
    tower-client    # CLI verbs (thin over core + reqwest)
    tower-tui       # ratatui client
    tower-web       # Topcoat UI crate (agent cloud + widgets, §12)
```
