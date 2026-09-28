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

## 12.5 Settings, themes, command palette

- **Pages**: one file per page in `tower-web/src/pages/` (`cloud.rs` =
  `/ui`, `settings.rs` = `/ui/settings`), re-exported from `pages/mod.rs`
  and registered on the Topcoat router. Shared chrome (`<head>`, theme
  boot script, palette) lives in `shell.rs`
- **Command palette**: `Cmd+K` (macOS) / `Ctrl+K` (Linux, Windows) or the
  `⌘K` header button opens a `<dialog>` with a search input; results
  (subsequence match on title + hint) list beneath it; arrows move,
  `Enter`/click navigates, `Esc` or a second `Cmd+K` closes. The command
  list is `shell::COMMANDS` — a new page registers itself there. Plain
  JS rendered once per page outside the live regions: no signals, no
  server round trip, and the button uses a delegated listener so the
  cloud's live re-renders never unwire it
- **Themes**: `theme::THEMES` is the registry — `tower-dark` (the
  original colors, default), `tokyo-night-storm`, `tokyo-night-light`
  (chrome colors from the Tokyo Night palette site). A theme sets only
  chrome variables (`--bg --fg --dim --hi --card --line --accent
  --on-accent`); the §12.1 state colors are identical in every theme, so
  the glance channels (amber = needs you) never change meaning. Only the
  default carries `:root`; the others match `[data-theme=…]` on `<html>`
- **Persistence: browser `localStorage`** (`tower.theme`, JSON string),
  not a cookie or the DB. The theme is a per-browser preference, and
  writing a cookie or a row would need a mutating endpoint under `/ui` —
  exactly what the read-only guard (§12.3, plan T3.2) forbids. A
  synchronous boot script in `<head>` applies the stored theme before
  first paint (no flash of the default), and falls back to the default
  on junk. The browser scripts take their id list from the registry
  (`theme::ids_json`), so a new theme is one registry entry + one CSS
  block. A later server-side settings store can seed or replace this
  without changing the settings page's contract
