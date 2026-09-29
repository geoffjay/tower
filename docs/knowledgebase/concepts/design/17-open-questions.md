---
type: Concept
title: Design §17 — Open questions
description: Unresolved design questions and the phase that resolves each.
tags:
  - design
  - design-s17
  - open-questions
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §17 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 17. Open questions

1. herdr socket protocol stability (SemanticFrame v20) — resolve in phase 1
   spike: validate CLI-driver first, socket later. (Owning risk.)
2. pi RPC runner surface (0.87) — is pane prompt/read enough, or does the
   adapter need pi's programmatic mode? Phase 1 spike.
3. ~~Web UI: Topcoat cloud-scale reactivity + layout approach~~ — closed
   by phase-4 spike S4.A: Topcoat 0.9 holds 60 fps at 200 points × 10 Hz;
   SVG points, server-side deterministic layout, live updates over
   Topcoat's WebSocket, runtime script vendored ([§12.3](12-web-ui.md)).
4. Node transport security: TLS + token vs requiring SSH tunnel — phase 5.
5. ~~Scoped read-only UI token vs full token~~ — closed by S4.C: a derived,
   scoped read-only UI token that opens only `/ui` ([§13](13-security.md)).
6. ~~Event retention defaults and pruning~~ — closed in phase 4 (T1.3):
   the sweeper deletes events older than `event_retention_days` (default
   14, `0` keeps everything) once an hour, never past what an open
   `/v1/events` stream has yet to send; `seq` is never reused, so an old
   cursor resumes at the oldest kept event. The TUI bounds its side (last
   2000 events, 100 closed jobs, [§11.1](11-tui.md)). Artifact retention
   stays open until artifacts are written (no producer yet).
7. ~~Cloud metrics semantics~~ — closed by S4.B: formulas and windows in
   the [phase-4 plan](../../plans/phase-4-web-ui.md#s4b--metric-semantics-2026-09-27),
   shown in the UI as tooltips.
8. Orchestrator agent + agent router: which decision model dispatches jobs
   (jev vs laya — both have Rust libraries; laya can run locally as a GGUF
   via ollama/llama.cpp), and how the orchestrator's still-working checks
   are rate-limited — after the job-queue primitive is proven (phase 2);
   see the [job-queue decision](../../decisions/job-queue.md).
9. Supervisor agent for fleet self-governance: whether tower needs a
   dedicated agent (possibly pi with custom extensions, surfaced as a
   distinct harness type) that watches the fleet — decides context
   compaction or reset before a job, records run questions and results,
   and holds an agent that waits on a human. The phase-2c
   [outcomes-and-brief decision](../../decisions/outcomes-and-brief.md)
   solves the data side without it: results are durable, open questions
   hold the job, the brief is restated per delivery. Revisit when fleet
   self-governance (auto-assign, compaction control) is wanted; the
   [job-queue decision](../../decisions/job-queue.md) already reserves an
   orchestrator role that may absorb this.
