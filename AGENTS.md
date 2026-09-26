# AGENTS.md

Working agreements for coding agents in this repository.

## Read first

1. [DESIGN.md](DESIGN.md) — the architecture. §18 maps phases.
2. [plans/README.md](plans/README.md) — how plans work.
3. The active phase plan (currently [plans/phase-1.md](plans/phase-1.md)).
4. [docs/getting-started.md](docs/getting-started.md) — the UX contract for
   CLI verbs. CLI-facing code must match it.

## Rules

- Work plan tasks in order; mark the plan's Verification log when a milestone
  verify passes. Spike findings go in the plan file, not loose notes.
- One commit per task where practical; conventional commits
  (`feat:`, `fix:`, `spike:`, `test:`, `docs:`, `chore:`).
- A milestone ends green: `just lint` and `just test` must pass before
  moving on. Never leave TODO comments at milestone boundaries.
- Design changes: amend DESIGN.md first (with the section number), then the
  plan, then the code.
- herdr is the execution substrate — never spawn agent processes directly.
- No `unsafe` (workspace-enforced). No secrets in code, logs, or events.

## Commands

```
just lint && just test    # must be green before any commit
```