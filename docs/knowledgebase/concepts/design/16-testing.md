---
type: Concept
title: Design §16 — Testing
description: Unit, integration (FakeHarness), E2E smoke, and schema contract testing strategy.
tags:
  - design
  - design-s16
  - testing
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §16 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 16. Testing

- **Unit**: state machines, claim/pull CAS + lease sweeper + max-attempts,
  message/timeout sweeper, event cursor math
- **Integration**: `FakeHarness` implementing the trait — scripted state
  transitions; full API + SSE flow against in-memory SQLite; concurrent-claim
  race tests (N clients claim same task, exactly one wins; loser gets 409)
- **E2E smoke**: temp `HOME`, real herdr, spawn `pi` (and `claude` when
  authed): prompt → working → done → events observed over SSE; recorded as
  `tower doctor --e2e`
- **Contract**: `GET /v1/schema` output diffed in CI (route/event registry
  changes are deliberate)
