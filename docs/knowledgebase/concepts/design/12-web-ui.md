---
type: Concept
title: Design §12 — Web UI (read-only monitoring)
description: Agent-cloud visualization, supporting widgets, Topcoat technology choice, and drill-down backlog.
tags:
  - design
  - design-s12
  - web-ui
  - topcoat
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §12 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 12. Web UI (read-only monitoring)

The web UI is a **health-at-a-glance visualization**, not a management console.
Glance first: the whole system's health readable in seconds without reading
text. Drill-down is deliberately minimal and optional.

## 12.1 The agent cloud (primary view)

Agents rendered as **points in a 2D cloud** (force-directed layout; agents
 drift toward their machine cluster, away from crowded neighbors, settle
 under a light repulsion simulation):

| Visual channel | Encodes |
|---|---|
| Point **color** (hue) | agent state: working=blue, blocked=amber, idle=gray, done=green, dead=red, launching=teal, unknown=violet |
| Point **size** | activity volume — event/message/output rate over a rolling window (bigger = busier) |
| Point **halo/pulse** | attention needed: blocked or expired items glow/pulse |
| Point **brightness** | health/quality score (recency of heartbeats/stale detection = dim) |
| Cluster position | machine grouping (local vs node machines, phase 5) |
| Edge lines (optional) | message volume between agents in the last N minutes (thicker = more traffic); toggleable |

A **floating side panel** appears when a point is selected: agent name, kind,
machine, state, current task + lease countdown, message rate, recent event
sparkline, last output snippet. That is the extent of drill-down in v1 —
full detail lives in the TUI, by design.

## 12.2 Supporting widgets

- Pool bar: queued / working / blocked counts, live
- Machine strip: one chip per machine with status dot (local + nodes)
- Event ribbon: last ~10 events, fading ticker
- No output tails in v1 cloud view (kept on the agent panel snippet only);
  the old per-agent tail page is backlog (§12.4)

## 12.3 Technology

- **Topcoat** (tokio-rs/topcoat, pinned `=0.9.0`): full-stack Rust,
  server-rendered with client-side reactivity, no WASM/JS bundle, keeps
  the whole server + UI in Rust and the single-binary story intact. The
  Topcoat router is bridged into the axum app under `/ui` (same port, same
  auth layer); the browser runtime script is vendored into `tower-web` and
  served from memory with an in-code asset catalog — no `topcoat` CLI, no
  asset directory
- Rendering (spike S4.A): cloud points are SVG `<g>` nodes; the layout is
  computed server-side and deterministically per render (machine cluster
  centers, golden-angle spiral, repulsion passes), so every browser and
  every reconnect sees the same positions; a per-point CSS drift animation
  keeps the cloud alive without client code. State changes animate through
  CSS transitions on morphed elements
- Data: initial render server-side from the DB; live updates through a
  Topcoat `live!` region on a connected page: the server follows the event
  log (the same `seq` cursor `/v1/events` serves, [D§7](07-server-api.md)) and pushes re-rendered
  HTML over Topcoat's WebSocket, which the runtime morphs in place. The
  browser never parses events; after a server restart the runtime
  reconnects and the fresh render rebuilds the cloud from current state.
  Metrics (§12.1 channels) are rolled up incrementally as events arrive,
  cached per agent, and defined in the phase-4 plan (S4.B) and as tooltips
- Risk accepted: Topcoat is explicitly early-stage ("expect breaking
  changes"; 0.7→0.9 broke APIs three times in three weeks) — the version is
  pinned and Topcoat types never leave `tower-web` ([D§2](02-stack.md)); an upgrade is
  a one-crate change plus re-vendoring the runtime script (a test fails
  while the vendored copy differs from the pinned crate's)
- Read-only enforced: the UI registers no Topcoat procedures and calls no
  mutating service code; the browser holds only the scoped UI token
  ([D§13](13-security.md)), which opens `/ui` and nothing else

## 12.4 Backlog (drill-down, later if ever)

- Full agent detail page with output tail + message history
- Job queue board and task trails
- Historical charts (event rate, throughput) from the event log
- Config: points vs table view toggle
