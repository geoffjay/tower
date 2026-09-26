# Phase 4 — Web UI (agent cloud, read-only)

Goal: a highly graphical, at-a-glance health view — agents as points in a
cloud, metrics encoded visually, floating detail panel on selection.
Drill-down stays minimal by design (D§12). Everything in Rust via Topcoat.

Exit criteria (DESIGN.md §18): cloud shows all agents as colored points with
live state changes for an hour soak: zero polling errors, blocked agents
visibly pulse, selecting a point opens the side panel with live detail.

Depends on: phase 2. Independent of phase 3.

Reference: D§12 (agent cloud, widgets, Topcoat choice, risks).

---

## Milestone 0 — Spikes (D§17.3, D§17.5, D§17.7)

- **S4.A** **Topcoat validation** (was htmx-vs-React; tech is now chosen,
  this spike validates it). Build a throwaway Topcoat page with ~80
  reactive points (mock data) and answer:
  - Do Topcoat reactive expressions handle per-point updates at
    state-change frequency (worst case: one update per agent per poll tick)?
  - SVG nodes vs absolutely-positioned DOM divs — rendering cost at
    50–100 points with color/size/halo transitions
  - Where does the force-directed layout run: server-side (recomputed on
    roster change, positions shipped in render) vs client-side in compiled
    reactivity (continuous animation)? Recommend the simpler one that still
    looks alive
  - Pin the topcoat version; note any breaking-change risk for the roadmap
  Output: findings appended here + DESIGN.md §17.3 closed. If Topcoat fails
  the 80-point test, fallback decision recorded (plain axum + hand-rolled
  SSE + vanilla JS canvas, still no build pipeline) and §12.3 amended.
- **S4.B** **Metric semantics** (D§17.7): define exact formulas from the
  event log for: activity volume (point size), health/quality (brightness),
  message volume (edge thickness), blocked pulse. Window sizes (e.g. 5-min
  rolling). Output: short spec table appended here; becomes tooltips in
  the UI and stays out of the hot path (computed on event append, cached
  per agent, not per render).

## Milestone 1 — Server-side groundwork (D§12.3)

- **T1.1** Topcoat app scaffold in `tower-web` crate, mounted under `/ui`
  (`/` redirect); server-rendered initial cloud from the DB roster.
  Isolation rule: no other crate imports topcoat types (framework churn
  stays local). Verify: roster fixture renders N points server-side.
- **T1.2** Metrics queries (per S4.B): event-log rollups per agent —
  activity rate, health score, edge volumes — computed incrementally and
  cached; exposed only via the UI's render data (no new public API in v1;
  if a route is needed, GET-only under `/v1/ui/*` per D§12 amendment rules).
  Verify: fixture events produce expected cached values; staleness handled.

## Milestone 2 — The cloud (D§12.1)

- **T2.1** Cloud view: points with the four channels — state color, size
  from activity, halo/pulse on blocked-or-attention, brightness from health;
  machine clustering (local only in this phase, but positions derive from
  machine groups so phase 5 slots in). Layout per S4.A decision.
  Verify: mixed-state fixture (10 agents, all states represented) renders
  correctly; transitions animate on event arrival.
- **T2.2** Live updates: SSE `/v1/events` consumer with cursor resume;
  state color flips working↔blocked↔idle in < 2s of the event; pool bar,
  machine strip, event ribbon widgets fed from the same stream.
  Verify: scripted event bursts drive all widgets; reconnect-after-server-
  restart resumes without duplicate points.

## Milestone 3 — Floating detail panel (D§12.1)

- **T3.1** Point selection → floating side panel: name, kind, machine,
  state, current task + lease countdown, message rate, recent-event
  sparkline, last output snippet. Closes on deselect/Esc. Updates live
  while open. Verify: panel binds to a fixture agent and reflects scripted
  state changes; lease countdown ticks from `lease_expires_at`.
- **T3.2** Read-only guard pass: UI client code exposes GET-only access;
  no mutation routes referenced anywhere in `tower-web` (grep test);
  token handling per S4.C outcome.

## Milestone 4 — Phase exit verification

- **T4.1** Hour soak: live agents (real herdr + pi) doing real work; cloud
  must show state transitions with zero polling errors, no memory growth
  (per-point DOM/SVG stable; metrics cache bounded), SSE reconnects
  survive a server restart. Record in Verification log.
- **T4.2** Glance test: show a 15-agent mixed-state cloud to the operator
  for 10 seconds, hide it, ask: which agents need you? Correct answer must
  be readable from color+halo alone (amber pulsing). Record result.
- **T4.3** Update DESIGN.md §17 (S4 items closed), plans/README status,
  CHANGELOG entry.

## Backlog (v2 seeds, D§12.4)

- Full agent detail page (output tail + message history)
- Task pool board view
- Historical charts from the event log
- Points ↔ table view toggle
- Edges: message-volume lines between agents (defined in S4.B, v2 wiring)

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|

## Spike findings

### S4.A — Topcoat validation (filled during execution)

### S4.B — metric semantics (filled during execution)