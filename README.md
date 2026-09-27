[![CI][ci-badge]][ci-url]
[![codecov][codecov-badge]][codecov-url]
[![MIT licensed][mit-badge]][mit-url]
[![Apache licensed][apache-badge]][apache-url]

[ci-badge]: https://github.com/geoffjay/tower/actions/workflows/ci.yml/badge.svg
[ci-url]: https://github.com/geoffjay/tower/actions/workflows/ci.yml
[codecov-badge]: https://codecov.io/gh/geoffjay/tower/graph/badge.svg?token=knPW8TUmoJ
[codecov-url]: https://codecov.io/gh/geoffjay/tower
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/geoffjay/tower/blob/main/LICENSE-MIT
[apache-badge]: https://img.shields.io/badge/License-Apache_2.0-yellowgreen.svg
[apache-url]: https://github.com/geoffjay/tower/blob/main/LICENSE-APACHE

---

# Tower

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
