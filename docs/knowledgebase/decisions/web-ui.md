---
type: Decision
title: Web UI — Topcoat inside the server, live regions, scoped UI token
description: The agent cloud runs as a Topcoat router bridged into the axum app under /ui, pushes re-rendered HTML over Topcoat's WebSocket instead of a browser SSE client, lays points out server-side, and gives the browser a derived read-only token.
tags:
  - decision
  - web-ui
  - topcoat
  - security
status: accepted
---

# Web UI — Topcoat inside the server, live regions, scoped UI token

Date: 2026-09-27. Outcome of phase-4 spikes S4.A–S4.C
([plan](../plans/phase-4-web-ui.md#spike-findings)). Amends
[D§12.3](../concepts/design/12-web-ui.md), [D§13](../concepts/design/13-security.md),
[D§7](../concepts/design/07-server-api.md), [D§10](../concepts/design/10-client-cli.md);
closes [D§17.3, 17.5, 17.7](../concepts/design/17-open-questions.md).

## Context

D§12 chose Topcoat (early-stage, pinned) for a read-only, highly graphical
agent cloud, fed by the same `/v1/events` stream every client uses. Open
were: whether Topcoat holds 50–100 live points, SVG vs DOM, where the force
layout runs, how the browser authenticates (`EventSource` cannot send a
bearer header), and whether Topcoat's asset pipeline breaks the single
binary.

## Decision

1. **One server, one port.** The Topcoat router is built inside `tower-web`
   and bridged into the axum app (`Router::handle` behind `/ui` and
   `/ui/{*rest}`); requests keep their extensions, so the runtime's
   WebSocket upgrade works through `axum::serve`. Topcoat types never leave
   `tower-web`; the server hands it a `UiSource` of plain `tower-core`
   types.
2. **Live regions, not a browser event client.** The page is a Topcoat
   connected page: `live!` regions loop on the server — follow the event
   log by `seq`, re-render, `emit!` — and the runtime morphs each emission
   in place. The browser never parses events, so it needs no API
   credential, and CSS transitions animate state changes on the kept
   elements. After a server restart the runtime reconnects and re-renders
   from current state.
3. **Server-side, deterministic layout**; SVG points; a CSS drift keeps the
   cloud alive with no client code.
4. **Metrics are an incremental cache**, not queries per render: one pass
   per event, fixed 10 s buckets per agent, booted from the last 15 min of
   the log (formulas: plan S4.B).
5. **Scoped read-only UI token**, derived from the bearer token
   (`HMAC-SHA256(token, "tower-ui-read-v1")`), valid only for `/ui` reads
   and the runtime's re-render `POST`. `tower ui` prints a login link; the
   login sets an HttpOnly, SameSite=Strict cookie on `/ui`.
6. **The runtime script is vendored** and served from memory with an
   in-code asset catalog; a test fails when the vendored copy differs from
   the pinned crate's.

## Rejected

- **Browser `EventSource` on `/v1/events`**: needs the API token in the
  browser (query string or a cookie that also opens `/v1`), and client
  JavaScript to turn events into DOM updates — the reason Topcoat was
  chosen.
- **`topcoat::serve` on a second port**: a second listener and auth
  surface for no gain.
- **`topcoat asset bundle`**: a post-build step and an asset directory
  next to the binary; breaks `cargo install` single-binary deployment.
- **Client-side force simulation**: continuous JS, different positions per
  browser, jumps on reconnect.
- **Full token in the browser**: any XSS or leaked link would grant every
  mutation route.

## Consequences

- A Topcoat upgrade is a `tower-web`-only change plus re-vendoring one
  file; 0.7 → 0.9 broke APIs three times, so expect it.
- `tower-web` builds with edition 2024 (Topcoat macros assume its prelude);
  the rest of the workspace stays on 2021.
- The page re-renders fully on selection change and on each coalesced
  event batch; fine at ≤ 100 agents (spike: 60 fps at 200 points × 10 Hz).
