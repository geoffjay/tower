---
type: Plan
title: Phase 4 — Web UI (agent cloud)
description: Read-only, graphical agent-cloud health view built in Rust via Topcoat.
tags:
  - plan
  - phase-4
  - web-ui
  - topcoat
status: draft
sources:
  - resource: git:340c189:plans/phase-4.md
    title: Original plan (removed from repo; full text in git history)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T23:50:57Z"
---

# Phase 4 — Web UI (agent cloud, read-only)

Goal: a highly graphical, at-a-glance health view — agents as points in a
cloud, metrics encoded visually, floating detail panel on selection.
Drill-down stays minimal by design ([D§12](../concepts/design/12-web-ui.md)). Everything in Rust via Topcoat.

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): cloud shows all agents as colored points with
live state changes for an hour soak: zero polling errors, blocked agents
visibly pulse, selecting a point opens the side panel with live detail.

Depends on: phase 2. Independent of phase 3.

Reference: [D§12](../concepts/design/12-web-ui.md) (agent cloud, widgets, Topcoat choice, risks).

---

## Milestone 0 — Spikes ([D§17.3](../concepts/design/17-open-questions.md), [D§17.5](../concepts/design/17-open-questions.md), [D§17.7](../concepts/design/17-open-questions.md))

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
  Output: findings appended here + [design §17.3](../concepts/design/17-open-questions.md) closed. If Topcoat fails
  the 80-point test, fallback decision recorded (plain axum + hand-rolled
  SSE + vanilla JS canvas, still no build pipeline) and [§12.3](../concepts/design/12-web-ui.md) amended.
- **S4.B** **Metric semantics** ([D§17.7](../concepts/design/17-open-questions.md)): define exact formulas from the
  event log for: activity volume (point size), health/quality (brightness),
  message volume (edge thickness), blocked pulse. Window sizes (e.g. 5-min
  rolling). Output: short spec table appended here; becomes tooltips in
  the UI and stays out of the hot path (computed on event append, cached
  per agent, not per render).
- **S4.C** **UI token** ([D§17.5](../concepts/design/17-open-questions.md)): scoped read-only
  UI token vs the full bearer token; how a browser gets it without the
  full token ever reaching the browser. Output: decision appended here,
  [D§13](../concepts/design/13-security.md) amended.

## Milestone 1 — Server-side groundwork ([D§12.3](../concepts/design/12-web-ui.md))

- **T1.1** Topcoat app scaffold in `tower-web` crate, mounted under `/ui`
  (`/` redirect); server-rendered initial cloud from the DB roster.
  Isolation rule: no other crate imports topcoat types (framework churn
  stays local). Verify: roster fixture renders N points server-side.
- **T1.2** Metrics (per S4.B): event-log rollups per agent — activity,
  health, message rate, sparkline buckets, output snippet — computed
  incrementally and cached; exposed only via the UI's render data (no new
  public API in v1). Edge volumes are specified in S4.B and computed when
  the edge lines are wired (backlog) — no consumer before that.
  Verify: fixture events produce expected cached values; staleness handled.
- **T1.3** Event retention ([D§17.6](../concepts/design/17-open-questions.md), carried from phase 3): prune
  events older than `event_retention_days` (default 14) in the sweeper,
  never past a live SSE cursor's replay window; metrics rollups (T1.2)
  must not depend on pruned rows. Verify: clock-injected sweep deletes
  exactly the expired rows; a cursor older than the horizon resumes at the
  oldest kept event.

## Milestone 2 — The cloud ([D§12.1](../concepts/design/12-web-ui.md))

- **T2.1** Cloud view: points with the four channels — state color, size
  from activity, halo/pulse on blocked-or-attention, brightness from health;
  machine clustering (local only in this phase, but positions derive from
  machine groups so phase 5 slots in). Layout per S4.A decision.
  Verify: mixed-state fixture (10 agents, all states represented) renders
  correctly; transitions animate on event arrival.
- **T2.2** Live updates (S4.A data path): a connected `live!` region
  follows the event log by `seq` and re-renders on each batch; state color
  flips working↔blocked↔idle in < 2s of the event; queue bar, machine
  strip, event ribbon widgets fed from the same loop.
  Verify: scripted event bursts drive all widgets; reconnect-after-server-
  restart resumes without duplicate points.

## Milestone 3 — Floating detail panel ([D§12.1](../concepts/design/12-web-ui.md))

- **T3.1** Point selection → floating side panel: name, kind, machine,
  state, current task + lease countdown, message rate, recent-event
  sparkline, last output snippet. Closes on deselect/Esc. Updates live
  while open. Verify: panel binds to a fixture agent and reflects scripted
  state changes; lease countdown ticks from `lease_expires_at`.
- **T3.2** Read-only guard pass: the browser holds only the S4.C UI token;
  it opens `/ui` reads and nothing else. Verify: every mutating route in
  the schema registry refuses the UI token (cookie and bearer); `/v1`
  reads refuse it too; `tower-web` registers no procedures. (Replaces the
  planned grep test: the registry sweep checks behavior — a grep for route
  strings misses a mutation reached any other way. `tower-web` depends on
  neither `tower-server` nor `tower-client`, so it has no mutating code to
  call.)

## Milestone 4 — Phase exit verification

- **T4.1** Hour soak: live agents (real herdr + pi) doing real work; cloud
  must show state transitions with zero polling errors, no memory growth
  (per-point DOM/SVG stable; metrics cache bounded), SSE reconnects
  survive a server restart. Record in Verification log.
- **T4.2** Glance test: show a 15-agent mixed-state cloud to the operator
  for 10 seconds, hide it, ask: which agents need you? Correct answer must
  be readable from color+halo alone (amber pulsing). Record result.
- **T4.3** Update [design §17](../concepts/design/17-open-questions.md) (S4 items closed), [plans overview](overview.md) status,
  CHANGELOG entry.

## Backlog (v2 seeds, [D§12.4](../concepts/design/12-web-ui.md))

- Full agent detail page (output tail + message history)
- Job queue board view
- Historical charts from the event log
- Points ↔ table view toggle
- Edges: message-volume lines between agents (defined in S4.B, v2 wiring;
  compute the pair volumes in the metrics cache with it)
- The panel covers the right edge of the cloud; shift the view when a
  point under it is selected
- herdr 0.8.2 never reports omp as `working` (it stayed `idle` through a
  25 s shell command), so omp agents never change color in the cloud;
  same root as the phase-3 note on omp's ask dialog — needs herdr's omp
  detection manifest
- The web UI reconnect backoff doubles to 30 s; after a long outage the
  cloud can lag the server's return by up to that long
- `tower approve` on claude's workspace-trust dialog answered "no plain
  'Yes' option" (`Yes, I trust this folder`), yet the dialog cleared —
  check `dialog.rs` against that screen

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-09-27 | S4.A throwaway: 80 / 200 SVG points, all rewritten every 250 / 100 ms, Topcoat inside axum; reconnect across a server kill | pass — 60 fps, 0 long tasks, stable DOM; see findings |
| 2026-09-27 | T1.3 clock-injected sweep: rows with `ts < now − 14 d` deleted, `ts == horizon` kept, an old cursor resumes at the oldest kept event; an open stream's unread rows survive until it closes; `seq` never reused after pruning everything | pass (`tests/sweeper.rs`, 3 tests) |
| 2026-09-27 | T1.2 metrics: 5-min window drains with time, id + name keys merge, message rate, fault window + health factors + clamp, radius curve, removal + prune bound memory, ribbon keeps 10 non-output events | pass (`metrics.rs`, 7 tests) |
| 2026-09-27 | Layout: deterministic, 100 points on canvas with no overlap, agents nearest their own machine's center, viewBox framing | pass (`layout.rs`, 4 tests) |
| 2026-09-27 | T1.1 roster of 12 renders 12 points server-side; runtime script served from memory at the URL the page references; vendored script == pinned crate's | pass (`tests/cloud.rs`, `assets.rs`) |
| 2026-09-27 | T2.1 10-agent fixture, all 7 states: class per state, amber `needs` for blocked and for a pending question, red `fault` after `task.failed`, `2 need you · 1 in inbox`, queue bar counts a blocked owner's job as blocked, dead at 0.35 brightness, active agent larger | pass (`tests/cloud.rs`) |
| 2026-09-27 | T2.2 over the real WebSocket protocol: working → blocked → idle each in < 2 s with the ribbon entry; a 41-event burst + queue + machine change lands in one coalesced render < 2 s; server restart + state change while down → reconnect shows 8 points once, new state | pass (`tests/cloud.rs`) |
| 2026-09-27 | T3.1 panel for a selected fixture agent: job + `lease 1:30`, output snippet; clock +5 s → `lease 1:25`; blocked while open → "needs you"; a non-agent selection renders no panel | pass (`tests/cloud.rs`) |
| 2026-09-27 | T3.2 UI token: `/ui`, runtime asset, re-render `POST` → 200; other `POST /ui` → 401; `/v1/agents`, `/v1/events`, `/v1/ui/token` → 401 (cookie and bearer); all 20 mutating registry routes → 401; login link sets `HttpOnly; SameSite=Strict; Path=/ui`, bad link and the bearer token as link → 401 with a `tower ui` hint | pass (`tests/ui.rs`, 4 tests) |
| 2026-09-27 | Real UI in Chrome, 80-agent churn fixture (`N=80`): 60 fps, 0 long tasks, 5 class flips in 12 s applied in place on the same elements (transitions run), `<svg>` and `<g>` identity kept across swaps, 5.4 MB heap | pass |
| 2026-09-27 | Live server (isolated home, port 8277, herdr + omp/claude agents): `tower ui` → link → cookie → cloud with 4 agents, a claude agent blocked at startup pulses amber, `1 need you · 1 in inbox` | pass |
| 2026-09-27 | T4.1 real-agent soak, 18:01–20:01: page open 1 h 58 m against the isolated server; 3 omp agents did 31 real jobs through the work loop (assign → start → complete; 283 output chunks), a claude agent sat blocked (amber pulse) until its approval expired (→ `dead`). Page: one WebSocket the whole time, 0 closes, 0 console errors, 0 error frames, 1650 frames, DOM 90 nodes flat, heap 3.9–4.6 MB (first/last sample 4.04 / 4.07). Server RSS 21.9–23.7 MB over 66 one-minute samples | pass for stability; **partial** for state changes: only launching → idle / blocked, blocked → dead occurred — omp never reports `working` (backlog), pi has no provider auth, claude spawned from this environment fails its API config |
| 2026-09-27 | T4.1 state-churn soak, 53 min: the real UI in Chrome over a 40-agent fixture flipping working / idle / blocked (throwaway harness): 431 point class flips applied in place, one WebSocket, 0 closes, 0 errors, 0 error frames, 4510 frames, 40 points and 0 duplicates in every sample, DOM 302–311 nodes, heap 1.9–2.9 MB (first/last 2.81 / 2.41) | pass (stopped at 53 min, not 60) |
| 2026-09-27 | T4.1 restart: server killed with the page open, back after ~15 s; the runtime retried at 1 / 2 / 4 / 8 / 16 s and reconnected; 4 points, 0 duplicates, 0 errors, ribbon shows `server.started` | pass |
| 2026-09-27 | T4.2 glance test, first run (15-agent demo: indexer + mobile blocked, review waiting on an answer): the operator named mobile, one wrong agent, and missed two | **fail** — the amber halos were seen but names were not readable; fix `cd75257`: the badge names them (`3 need you: indexer, mobile, review`), their labels turn amber and bold. Re-run pending |

## Spike findings

### S4.A — Topcoat validation (2026-09-27)

Question: can Topcoat 0.9 carry an 80-point live cloud inside tower's axum
server and single binary; SVG or DOM points; where the layout runs.

Throwaway (`/tmp/s4a`, not committed): axum 0.8 app with a Topcoat router
bridged in (`Router::handle` behind an axum `any` route), a `live!` region
that re-emits the whole cloud on a `tokio::sync::watch` tick, and a mock
sim that rewrites random points (state, size, brightness, position).
Measured in headless Chromium with an in-page probe (rAF frame times,
`longtask` observer, DOM node count, JS heap).

| Run | fps | longest frame | long tasks | DOM nodes | heap |
|---|---|---|---|---|---|
| SVG, 80 points, all 80 rewritten every 250 ms, 10 s | 60.1 | 17 ms | 0 | 247 stable | 3.0 → 4.2 MB |
| DOM divs, same load | 60.1 | 17 ms | 0 | 86 stable | 5.0 → 4.9 MB |
| SVG, 200 points, all rewritten every 100 ms | 59.9 | 25 ms | 0 | 607 stable | 18 MB |
| DOM divs, 200 points / 100 ms | 60.0 | 17 ms | 0 | 206 stable | 20 MB |

Findings:

- **Live updates** come from Topcoat's own mechanism, not a browser
  EventSource: a `live!` region that calls `runtime::connected(cx)` makes
  the page open a WebSocket (`topcoat-runtime` subprotocol) at its own URL;
  the connected render loops `emit!` → await change. Each emission is a
  `swap` frame the runtime morphs in place (idiomorph-style, matched by
  `id`), so CSS transitions and keyframe animations run on the kept
  elements. The worst case (every agent changes every tick) costs nothing
  measurable at 80 points; 200 points at 10 Hz still hold 60 fps.
- **Reconnect**: killing the server mid-session and starting it again, the
  runtime reconnected by itself (1 s, 2 s, … backoff) and a fresh render
  replaced the cloud: 80 points before and after, no duplicates, the
  selection signal kept its value.
- **Selection**: a client signal read on the server inside a `live!` region
  triggers a page re-render over the open socket — click → panel content
  for the new agent in 6 ms. `$(...)` handlers survive morphs.
- **axum bridge works**, including the WebSocket upgrade (hyper's
  `OnUpgrade` extension passes through `axum::serve`). No `topcoat::serve`,
  no second port.
- **Single binary: one gap.** The browser runtime script is a Topcoat asset
  (`topcoat::runtime::SCRIPT`), normally written to disk by the
  `topcoat asset bundle` CLI after each build. Fix: vendor the runtime's
  `browser/dist/index.js` (MIT, 35 KB) into `tower-web`, serve it from
  memory, and hand Topcoat an in-code catalog
  (`AssetConfig::hosted_at("/ui/assets", Manifest { … SCRIPT.id() … })`).
  A test compares the vendored bytes to the registry copy (found through
  the asset record in the test binary), so a version bump cannot ship a
  stale script. No build step, no `topcoat` CLI.
- **Test harness caveat**: the headless browser throttles a page between
  tool calls, so the socket looked stalled from outside. Measure with
  in-page probes (one `evaluate` that samples for N seconds); the soak
  (T4.1) must do the same.

Decisions:

- **Topcoat passes; no fallback.** Pin `topcoat = "=0.9.0"`
  (`default-features = false`, features `view`, `router`, `runtime`,
  `asset`). MSRV 1.98, edition 2024 inside the dependency only.
- **SVG** points: a `<g>` per agent (core circle + halo ring); the viewBox
  scales with the window and edge lines (backlog) are native. DOM divs
  were cheaper per node but both are far inside budget.
- **Layout runs server-side, deterministic**: machine cluster centers,
  golden-angle spiral per cluster, a few repulsion passes; recomputed per
  render from the roster (≤100 points: microseconds), so a reconnect or a
  second browser gets identical positions. "Alive" is a per-point CSS
  drift animation (duration and phase from the agent id) — no client code.
- **Data path**: the server-side live loop follows the event log (the same
  `seq` cursor `/v1/events` uses) and pushes rendered HTML; the browser
  never parses events. D§12.3 amended.
- **Breaking-change risk**: 0.7 (09-04), 0.8 (09-09), 0.9 (09-24) each
  broke APIs. Topcoat types stay inside `tower-web`; upgrades are a
  one-crate job plus re-vendoring the script (the drift test fails
  until done).

### S4.B — metric semantics (2026-09-27)

Computed incrementally as events are appended (one pass per event, cached
per agent in fixed-size 10 s buckets), read at render time relative to
`now`. Attribution: an event belongs to an agent when its subject is
`agent:<id>`, or its payload names the agent (`owner_id`, `prior_owner`,
`agent_id`, `from`/`to` of an agent message).

| Channel | Formula | Window | Tooltip |
|---|---|---|---|
| Activity → point radius | `n` = events attributed to the agent (output chunks, state changes, messages to/from it, its job events); radius = `7 + 11 · min(1, ln(1+n) / ln(121))` px | 5 min (30 × 10 s buckets) | "events in the last 5 min" |
| Health → brightness | `h = 1`; × 0.5 if state `unknown`; × 0.6 if its owned job has < 25 % of `lease_s` left (heartbeat overdue); × 0.6 per fault (`task.leased_out`, `task.failed`, `approval.expired`) in the window, at most two; × 0.7 if `working` with no activity in the last 2 min. Clamp to [0.35, 1]; `dead` = 0.35 | 15 min faults, 2 min silence | "health: lease, faults, silence" |
| Attention → halo | **amber pulse**: state `blocked`, or a pending question/approval from the agent. **red steady ring**: a fault in the last 15 min | live / 15 min | "needs you" / "recent fault" |
| Message volume → edge (backlog) | agent↔agent `message.created` per unordered pair; stroke = `1 + 3 · min(1, n/20)` px | 15 min | "messages in 15 min" |
| Panel message rate | messages to/from the agent per minute | 5 min | "msgs/min" |
| Panel sparkline | the activity buckets | 5 min (30 bars) | — |

Memory bound: per agent 30 activity + 30 message buckets, ≤ 2 fault
timestamps, one output snippet (last 6 lines, ≤ 600 bytes); dropped on
`agent.removed` and pruned to the live roster. On boot the cache replays
only the last 15 min of the log, so retention (T1.3, 14 days) never
touches rows it needs.

### S4.C — UI token (2026-09-27)

Decision: **a scoped read-only UI token**, never the bearer token.

- Derived, not stored: `hex(HMAC-SHA256(key = bearer token,
  "tower-ui-read-v1"))`. Rotating the bearer token rotates it; no new file.
- Scope: only `/ui` and `/ui/*`, methods `GET`/`HEAD` (the runtime's
  WebSocket is a `GET` upgrade) plus Topcoat's page re-render `POST`
  (`X-Topcoat-Runtime: true`), which Topcoat rewrites to a `GET` before
  any handler runs. Everything else answers `401` with the UI token.
  `tower-web` registers no procedures, the only Topcoat mechanism that runs
  server code on a browser call.
- Delivery: `tower ui` asks the server (`GET /v1/ui/token`, bearer auth) and
  prints `http://<server>/ui/login?token=<ui token>`. `/ui/login` sets
  `tower_ui` (HttpOnly, SameSite=Strict, Path=/ui, 30 days) and redirects to
  `/ui`, so the token leaves the address bar. The bearer token also opens
  `/ui` (curl, tests); the unix socket stays exempt.
- Cross-site: SameSite=Strict keeps the cookie off cross-site requests, and
  Topcoat's default origin policy rejects cross-origin WebSocket handshakes
  and state-changing requests.
