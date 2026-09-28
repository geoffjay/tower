---
type: Plan
title: Execution plans overview
description: How tower's phase plans work, their dependency graph, cross-cutting rules, and status.
tags:
  - plan
  - roadmap
  - process
status: stable
sources:
  - resource: git:340c189:plans/README.md
    title: Original plan (removed from repo; full text in git history)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T23:50:57Z"
---

# tower — Execution Plans

Derived from [design §18](../concepts/design/18-phase-mapping.md). One plan per phase. Each plan is
self-contained enough to be executed top-to-bottom by an agent or a human, with
verifiable exit criteria tied to the design.

## Reading a plan

- **Spike** — a bounded investigation that de-risks later tasks; output is a
  written decision, merged into the design's open questions
- **Task** — a unit of work: build something, verify it, commit
- Tasks within a milestone are ordered; milestones can interleave where noted
- Every plan ends with its phase exit criteria from [design §18](../concepts/design/18-phase-mapping.md)

## UX contract

[docs/getting-started.md](../../getting-started.md) is the target-state CLI
walkthrough. CLI-facing tasks (verbs, flags, table output, `--json` shapes)
must implement what that document shows; deviations found during execution
are recorded as amendments there first, then implemented.

## Plan index

| Plan | Phase | Depends on | Status |
|---|---|---|---|
| [Phase 1 — MVP core](phase-1-mvp-core.md) | MVP core | — | **complete** (2026-09-26) |
| [Phase 2 — Messaging + job queue](phase-2-messaging-task-pool.md) | Messaging + job queue | phase 1 | **complete** (2026-09-27) |
| [Phase 2b — Scheduled jobs](phase-2b-scheduled-jobs.md) | Scheduled + recurring jobs | phase 2 | **complete** (2026-09-27) |
| [Phase 3 — TUI](phase-3-tui.md) | TUI | phase 2 | not started |
| [Phase 4 — Web UI (agent cloud)](phase-4-web-ui.md) | Web UI | phase 2 (independent of 3) | built (2026-09-27); exit pending: real-agent `working` transitions (glance test passed) |
| [Phase 5 — Multi-machine](phase-5-multi-machine.md) | Multi-machine | phase 2 | not started |
| [Phase 6 — A2A edge](phase-6-a2a-edge.md) | A2A edge | phase 2 | not started |

## Dependency graph

```
phase-1 ──► phase-2 ──┬──► phase-3 (TUI)
                       ├──► phase-4 (web UI)
                       ├──► phase-5 (multi-machine)
                       └──► phase-6 (A2A edge)
```

Phases 3–6 are parallelizable after phase 2, though recommended order is
3 → 4 → 5 → 6 (monitoring before distribution before external exposure).
Phase 1 spike A (herdr CLI driver) should be done first of all — it gates the
whole project.

## Cross-cutting rules (all phases)

1. **Workspace hygiene**: one commit per task where practical; conventional
   commits (`feat:`, `fix:`, `spike:`, `test:`, `docs:`, `chore:`)
2. **Every milestone ends green**: `cargo fmt --check`, `cargo clippy -- -D
   warnings`, `cargo test`, and the plan's own verify commands must pass
3. **Traceability**: task IDs reference [design](../concepts/design/) sections (`D§`) so design
   changes can be traced back to affected tasks
4. **No TODO-comments left at milestone boundaries** — open issues go in the
   plan's Backlog section instead
5. **Spikes write decisions, not code**: output is an appended section in the
   plan file itself (date, question, finding, decision, design deltas)

## Definition of done (applies to every phase)

- All milestones in the plan completed and verified
- Exit criteria from [design §18](../concepts/design/18-phase-mapping.md) demonstrated (commands + observed output
  recorded in the plan's Verification log)
- [design open-questions list](../concepts/design/17-open-questions.md) updated for anything the phase resolved
- Plans index table above updated with status
- CHANGELOG.md entry added (created in phase 1, milestone M0)
