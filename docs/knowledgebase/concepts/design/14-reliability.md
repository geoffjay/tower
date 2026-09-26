---
type: Concept
title: Design §14 — Reliability and failure modes
description: Behavior under server, herdr, node, coordinator, SSE, lease, claim-race, and DB-write failures.
tags:
  - design
  - design-s14
  - reliability
  - failure-modes
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §14 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 14. Reliability and failure modes

| Failure | Behavior |
|---|---|
| Server crash | Agents unaffected (herdr owns PTYs). On restart: snapshot reconcile, event log intact, agents re-bound by pane id. Lease sweeper resumes; surviving owners renew and keep work, dead owners' tasks requeue on expiry |
| herdr crash | herdr restores layout/sessions (its own persistence). tower reconciles on next snapshot/poll; agents marked `unknown` until then |
| Node offline | Its agents → `dead/unreachable` view state; queued prompts to it fail fast with `machine_offline` |
| Coordinator offline (node view) | Node keeps agents alive via herdr; reconnects, resyncs snapshot |
| Slow SSE client | Disconnect with resume cursor; lossless replay from event log + artifacts |
| Approval timeout | Sweeper expires it; agent notified; event emitted |
| Task lease expiry | Owner crashed/quiet → requeue within one lease window; next `task.leased_out` event; max-attempts → `failed` |
| Claim race (two agents, one task) | SQLite CAS loses exactly one bidder → clean `conflict`; no double-ownership window |
| DB write failure | Server degrades read-only + logs loudly; driver calls paused (never silently drop) |

Backups: the whole state is `~/.local/share/tower/` — copy the directory.
