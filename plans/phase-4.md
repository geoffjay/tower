# Phase 4 — Web UI (read-only)

Goal: an embedded, read-only dashboard. Live states, pool, output tails,
messages/tasks history. Zero config/control surface — by design and by route
(D§12).

Exit criteria (DESIGN.md §18): dashboard shows live states, pool
(queued/working/blocked) and output tails with zero polling errors through an
hour soak.

Depends on: phase 2. Independent of phase 3.

---

## Milestone 0 — Decision spike (D§17.3, D§17.5)

- **S4.A** htmx + server-rendered partials vs minimal React/Vite SPA.
  Constraints: embedded in binary (rust-embed, D§2), read-only, SSE-fed, no
  Node toolchain at server runtime (build-time toolchain acceptable).
  Evaluate: SSE wiring effort, output-tail ANSI rendering (does htmx swap
  strategy handle a scrolling tail well?), dependency weight. Output:
  decision + rationale appended to this file; update DESIGN.md §17.3/§17.5.
- **S4.B** Read-token scoping (D§17.5): v1 single-operator — decide between
  (a) same token as everything (simplest), (b) read-only token minted at
  first run for the UI bundle only. Default recommendation: (b) if it costs
  < half a day, else (a) with a tracking note for v2. Decision recorded same
  way.

## Milestone 1 — Serving + data contract (D§12)

- **T1.1** `/ui` route serving embedded assets (rust-embed); redirect `/` →
  `/ui`; cache headers; no mutation routes in the UI bundle's API client —
  enforced by the client library only exposing GETs (verify by review +
  grep test that no POST is referenced in web assets).
- **T1.2** Read-only data endpoints the UI needs (reuse phase-2 routes,
  no new ones unless a gap appears; if a gap appears, add GET-only routes
  under `/v1/ui/*` and record why in DESIGN.md §7). Verify: gap analysis
  written into this file before building views.

## Milestone 2 — Dashboard + agent detail (D§12)

- **T2.1** Dashboard: agent cards (name, machine, kind, state color/glyph),
  blocked banner (inbox pending), pool summary (queued/working/blocked
  counts), machine status strip. All fed by SSE `/v1/events` with cursor
  resume; initial paint from GETs. Verify: deterministic render test with
  fixture state; SSE-reconnect behavior in a scripted test.
- **T2.2** Agent detail page: output tail (ANSI → HTML rendering decision from
  S4.A), current task with lease countdown, recent messages. Verify: ANSI
  passthrough visually checked against a recorded pane capture; countdown
  renders from fixture.

## Milestone 3 — Tasks + messages pages (D§12)

- **T3.1** Tasks page: pool board (queued/working/input-required columns
  conceptually — table is fine for v1), priority ordering, tags, claim trail
  detail view. Verify: fixture-driven render test.
- **T3.2** Messages/inbox page: history with kind/status badges; read-only
  (respond only via CLI/TUI — the page links to the command, not a form).
  Verify: history pagination over fixture messages.

## Milestone 4 — Phase exit verification

- **T4.1** Hour soak: dashboard open against live agents (real herdr + pi
  doing real work), zero polling errors, SSE reconnects on server restart,
  no unbounded DOM growth (output tails ring-buffered at ~500 lines
  client-side). Record in Verification log.
- **T4.2** Accessibility + keyboard pass: state glyphs have text labels,
  tables navigable, no console errors. Record checks.

## Backlog (v2 seeds)

- Historical charts (events/day, task throughput) from the event log
- Read-only node dashboards per machine
- WebSocket upgrade path if SSE ever proves limiting (not expected, D§7)

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|

## Spike findings

### S4.A — UI tech (filled during execution)

### S4.B — read token (filled during execution)