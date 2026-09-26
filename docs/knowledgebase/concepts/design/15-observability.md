---
type: Concept
title: Design §15 — Observability
description: Logs, event log as audit surface, /healthz contents, optional metrics.
tags:
  - design
  - design-s15
  - observability
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §15 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 15. Observability

- `tracing` JSON logs to stderr (journald via systemd)
- Event log is the primary audit surface (`tower` CLI queries it;
  `events?filter=` in UI)
- `/healthz` returns: db ok, driver ok (last snapshot age), node statuses
- Prometheus metrics endpoint: optional phase 5 (`/metrics`, default off)
