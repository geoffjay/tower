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

- **Topcoat** (tokio-rs/topcoat, v0.9): full-stack Rust, server-rendered
  with client-side reactivity, no WASM/JS bundle, keeps the whole server +
  UI in Rust and the single-binary story intact
- Rendering: cloud points as absolutely-positioned DOM/SVG nodes updated
  via Topcoat reactive expressions; layout simulation computed server-side
  or client-side in Rust-compiled reactivity (Spike S4.B decides: SVG vs
  DOM points, and where the force layout runs)
- Data: initial render server-side from the DB; live updates by consuming
  the same SSE `/v1/events` stream every client uses (cursor resume on
  reconnect, [D§7](07-server-api.md))
- Risk accepted: Topcoat is explicitly early-stage ("expect breaking
  changes") — pin the version; isolate UI code in `tower-web` so framework
  churn is one crate's problem ([D§2](02-stack.md)). Spike S4.B verifies canvas-scale
  reactivity (~50–100 points) before committing
- Read-only enforced as before: UI data routes are GET-only; no mutation
  routes in the UI bundle; token per [D§13](13-security.md)/[§17.5](17-open-questions.md)

## 12.4 Backlog (drill-down, later if ever)

- Full agent detail page with output tail + message history
- Task pool board and task trails
- Historical charts (event rate, throughput) from the event log
- Config: points vs table view toggle
