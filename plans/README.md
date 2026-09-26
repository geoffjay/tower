# tower — Execution Plans

Derived from [DESIGN.md](../DESIGN.md) §18. One plan per phase. Each plan is
self-contained enough to be executed top-to-bottom by an agent or a human, with
verifiable exit criteria tied to the design.

## Reading a plan

- **Spike** — a bounded investigation that de-risks later tasks; output is a
  written decision, merged into the design's open questions
- **Task** — a unit of work: build something, verify it, commit
- Tasks within a milestone are ordered; milestones can interleave where noted
- Every plan ends with its phase exit criteria from DESIGN.md §18

## UX contract

[docs/getting-started.md](../docs/getting-started.md) is the target-state CLI
walkthrough. CLI-facing tasks (verbs, flags, table output, `--json` shapes)
must implement what that document shows; deviations found during execution
are recorded as amendments there first, then implemented.

## Plan index

| Plan | Phase | Depends on | Status |
|---|---|---|---|
| [plans/phase-1.md](phase-1.md) | MVP core | — | **complete** (2026-09-26) |
| [plans/phase-2.md](phase-2.md) | Messaging + task pool | phase 1 | not started |
| [plans/phase-3.md](phase-3.md) | TUI | phase 2 | not started |
| [plans/phase-4.md](phase-4.md) | Web UI | phase 2 (independent of 3) | not started |
| [plans/phase-5.md](phase-5.md) | Multi-machine | phase 2 | not started |
| [plans/phase-6.md](phase-6.md) | A2A edge | phase 2 | not started |

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
3. **Traceability**: task IDs reference DESIGN.md sections (`D§`) so design
   changes can be traced back to affected tasks
4. **No TODO-comments left at milestone boundaries** — open issues go in the
   plan's Backlog section instead
5. **Spikes write decisions, not code**: output is an appended section in the
   plan file itself (date, question, finding, decision, design deltas)

## Definition of done (applies to every phase)

- All milestones in the plan completed and verified
- Exit criteria from DESIGN.md §18 demonstrated (commands + observed output
  recorded in the plan's Verification log)
- `DESIGN.md` open-questions list updated for anything the phase resolved
- Plans index table above updated with status
- CHANGELOG.md entry added (created in phase 1, milestone M0)