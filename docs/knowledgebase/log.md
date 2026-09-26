# Knowledge Base Update Log

## 2026-09-26
* **Restore**: Replaced the plan summaries in `plans/` with the full original text of `plans/README.md` and `plans/phase-1..6.md` (from git `340c189`): milestones, tasks, backlogs, verification logs, spike findings. Only links changed (design `§`/`D§` refs, phase cross-links, getting-started, cross-server research).
* **Relink**: Original `plans/` removed; `docs/knowledgebase/plans/` is now canonical. Plan docs' `resource` → `sources` git provenance; `AGENTS.md`, `README.md`, `docs/getting-started.md` links repointed to the KB.
* **Ingest**: Converted `DESIGN.md` into `concepts/design/` (one `type: Concept` doc per section, §-numbered filenames, cross-section `§`/`D§` refs linked), `RECOMMENDATIONS.md` into `decisions/architecture-recommendations.md`, `research/ipc.md` and `research/cross-server.md` into `decisions/`, and `research/{a2a,agentd,herdr,openrig}.md` into `references/`. Full content, not summaries; originals removed from the repo (provenance in each doc's `sources`). Plan docs' `D§` refs now link to the design docs.
* **Scaffold**: Created the tower knowledge base at `docs/knowledgebase/` using the `okf-ify` skill. Initial structure: `index.md`, `log.md`, and concept directories (concepts, decisions, patterns, references, plans). OKF v0.2 conformant.
* **Ingest**: Ingested `plans/` (README + phase-1..6) into `docs/knowledgebase/plans/` as `type: Plan` docs scaffolded with `okf new`. Summaries only; `plans/*.md` remain canonical (linked via `resource`).
