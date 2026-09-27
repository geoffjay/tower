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
3. Web UI: Topcoat is chosen ([§12.3](12-web-ui.md)); spike S4.B (phase 4) validates
   cloud-scale reactivity + layout approach (SVG vs DOM, where the force
   sim runs) before full build-out.
4. Node transport security: TLS + token vs requiring SSH tunnel — phase 5.
5. Scoped read-only UI token vs full token — phase 4.
6. Event/artifact retention defaults and pruning UX — phase 3 tune.
7. Cloud metrics semantics: exact formulas for "activity volume" (size),
   "quality/health" (brightness), and message-volume edges — defined as
   event-log queries in phase 4 spike S4.B (the phase-4 plan owns it);
   documented in the UI as tooltips.
8. Orchestrator agent + agent router: which decision model dispatches jobs
   (jev vs laya — both have Rust libraries; laya can run locally as a GGUF
   via ollama/llama.cpp), and how the orchestrator's still-working checks
   are rate-limited — after the job-queue primitive is proven (phase 2);
   see the [job-queue decision](../../decisions/job-queue.md).
