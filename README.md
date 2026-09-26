# tower

Control and visibility for herds of coding agents. One server, one database,
one CLI; agents live in [herdr](https://herdr.dev) panes; harnesses: Claude
Code and pi (ohmypi).

**Status**: pre-phase-1 (research + design + execution plans complete; see
the [phase 1 plan](docs/knowledgebase/plans/phase-1-mvp-core.md) now in execution).

## Documents

- [Design](docs/knowledgebase/concepts/design/index.md) — full design: stack,
  data model, API, modules
- [docs/getting-started.md](docs/getting-started.md) — target-state CLI
  walkthrough (UX contract)
- [Architecture recommendations](docs/knowledgebase/decisions/architecture-recommendations.md)
  — architecture synthesis
- Research: [references](docs/knowledgebase/references/index.md) (agentd,
  herdr, openrig, A2A) and [decisions](docs/knowledgebase/decisions/index.md)
  (IPC, cross-server)
- [Plans](docs/knowledgebase/plans/index.md) — per-phase execution plans

## Development

```
cargo make lint     # fmt + clippy -D warnings
cargo make test     # workspace tests
cargo make run      # tower serve (once milestone 3 lands)
```

Rust 1.98+, tokio/axum/sqlx/SQLite. No configuration required to start.