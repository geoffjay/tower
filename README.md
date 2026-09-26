# tower

Control and visibility for herds of coding agents. One server, one database,
one CLI; agents live in [herdr](https://herdr.dev) panes; harnesses: Claude
Code and pi (ohmypi).

**Status**: pre-phase-1 (research + design + execution plans complete; see
the [phase 1 plan](plans/phase-1.md) now in execution).

## Documents

- [DESIGN.md](DESIGN.md) — full design: stack, data model, API, modules
- [docs/getting-started.md](docs/getting-started.md) — target-state CLI
  walkthrough (UX contract)
- [RECOMMENDATIONS.md](RECOMMENDATIONS.md) — architecture synthesis
- [research/](research/) — agentd, herdr, openrig, IPC, cross-server, A2A
- [plans/](plans/) — per-phase execution plans

## Development

```
just lint     # fmt + clippy -D warnings
just test     # workspace tests
just run      # tower serve (once milestone 3 lands)
```

Rust 1.98+, tokio/axum/sqlx/SQLite. No configuration required to start.