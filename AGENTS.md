# AGENTS.md

Working agreements for coding agents in this repository.

## Read first

1. [Design](docs/knowledgebase/concepts/design/index.md) — the architecture,
   one knowledge-base doc per section, cited as `D§N`.
   [§18](docs/knowledgebase/concepts/design/18-phase-mapping.md) maps phases.
2. [Plans overview](docs/knowledgebase/plans/overview.md) — how plans work.
3. The active phase plan (currently [phase 1](docs/knowledgebase/plans/phase-1-mvp-core.md)).
4. [docs/getting-started.md](docs/getting-started.md) — the UX contract for
   CLI verbs. CLI-facing code must match it.

## Rules

- Work plan tasks in order; mark the plan's Verification log when a milestone
  verify passes. Spike findings go in the plan file, not loose notes.
- One commit per task where practical; conventional commits
  (`feat:`, `fix:`, `spike:`, `test:`, `docs:`, `chore:`).
- A milestone ends green: `cargo make lint` and `cargo make test` must pass before
  moving on. Never leave TODO comments at milestone boundaries.
- Design changes: amend the design section doc in
  `docs/knowledgebase/concepts/design/` first (cite the `D§` number), then the
  plan, then the code.
- herdr is the execution substrate — never spawn agent processes directly.
- No `unsafe` (workspace-enforced). No secrets in code, logs, or events.

## Commands

```
cargo make lint && cargo make test    # must be green before any commit
```