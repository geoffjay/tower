---
type: Concept
title: Design §3 — Process and deployment model
description: tower serve is the only stateful process; clients and node relays are stateless.
tags:
  - design
  - design-s3
  - deployment
  - architecture
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §3 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 3. Process and deployment model

```
                 ┌──────────────────────── tower serve ────────────────────────┐
                 │ axum router, single port                                       │
                 │  /v1/* control+query   /v1/events SSE   /mcp   /a2a   /ui     │
                 │ ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐ │
                 │ │inventory│ │sessions │ │messaging │ │ tasks  │ │ machine  │ │
                 │ │ module  │ │ module  │ │ module   │ │ module │ │ hub      │ │
                 │ └─────────┘ └─────────┘ └──────────┘ └────────┘ └──────────┘ │
                 │ ┌──────────────────┐  ┌──────────────────────────────────┐     │
                 │ │ herdr driver(s)  │  │ SQLite (WAL): tower.db        │     │
                 │ │ (local machine)  │  │ + monotonic event log           │     │
                 │ └──────────────────┘  └──────────────────────────────────┘     │
                 └─────────────────────────────────────────────────────────────────┘
                    ▲            ▲              ▲                ▲
          tower CLI│     tower tui│    browser (UI)    tower node ── herdr ── agents
                (HTTP)          (HTTP)       (HTTP+SSE)     on remote machines
```

- `tower serve` runs as a systemd user unit on the primary machine
  (`tower.service` ships with the project). It is the only stateful process.
- Agents run inside herdr on the same machine (local herdr driver) or on remote
  machines (node agents relay through the hub).
- Clients are stateless: CLI, TUI, browsers, MCP clients, A2A clients.
- `tower node` is a stateless relay: it holds no database, executes driver
  calls against its local herdr, and forwards events to the coordinator.
