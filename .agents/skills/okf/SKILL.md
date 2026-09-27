---
name: okf
description: Use the `okf` CLI against this repo's OKF knowledge base at docs/knowledgebase/ — validate conformance, format, lint, audit trust and staleness, inspect links and the graph, scaffold concepts, and refactor with link rewriting. Covers which subcommand to reach for, which ones mutate files, the two that destroy the hand-written root index.md, the trust rules an agent must never forge, and the exit codes. Use whenever running an `okf` command, editing anything under docs/knowledgebase/, or diagnosing a failed `okf fmt`/`okf validate` pre-commit step.
---

# okf CLI

`okf` is a pure-Rust CLI for [Open Knowledge Format](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) bundles. This repo's bundle is `docs/knowledgebase/` — one bundle, so every command takes `docs/knowledgebase/` as its target, not an individual file.

This skill is about *driving the tool*. For the format itself (what a concept is, what frontmatter means, how to write a good concept body) the normative reference is `docs/knowledgebase/references/okf-spec.md` and the spec link above.

Installed build: `okf 0.3.0 (OKF spec v0.2)`. It is a fork — `make deps` installs from `github.com/geoffjay/okf --branch okf-web`, not crates.io.

## The two destructive commands

**Never run `okf index` or `okf lint --fix` on this bundle.**

Both regenerate every `index.md` from directory contents. `docs/knowledgebase/index.md` is *hand-written*: it holds the "For agents (policy)" section that tooling injects into every agent session, plus the curated one-line pointers to each entry. Regenerating replaces all of it with a bare `# Subdirectories` list and invents entries for `.claude/` and the gitignored `site/`.

Verified safe vs destructive on `docs/knowledgebase/index.md`:

| Command | Mutates files | Root `index.md` |
| --- | --- | --- |
| `okf fmt -w docs/knowledgebase/` | yes | safe |
| `okf validate --fix docs/knowledgebase/` | yes | safe |
| `okf lint --fix docs/knowledgebase/` | yes | **destroys it** |
| `okf index docs/knowledgebase/` | yes | **destroys it** |

Category indexes (`docs/knowledgebase/concepts/index.md` and siblings) are hand-curated too — the descriptions there are written prose, not copies of each concept's `description`. Maintain all indexes by editing them.

If you need the fixes `lint --fix` offers (see below), either hand-apply them or run it and immediately restore the indexes:

```sh
okf lint --fix docs/knowledgebase/ && git checkout -- docs/knowledgebase/index.md docs/knowledgebase/*/index.md
```

## Choosing a subcommand

**Read-only — safe to run any time, and the right default for answering a question.**

| Need | Command |
| --- | --- |
| Is the bundle conformant? | `okf validate docs/knowledgebase/` |
| Is it formatted? | `okf fmt --check docs/knowledgebase/` |
| Authoring-hygiene findings | `okf lint docs/knowledgebase/` |
| Trust tier, status, staleness per concept | `okf trust docs/knowledgebase/` |
| Bundle summary: counts, types, tiers, tags | `okf info docs/knowledgebase/` |
| Cross-links, and broken ones | `okf links docs/knowledgebase/` |
| Broken-link gate for CI | `okf links docs/knowledgebase/ --broken --check` |
| Link graph | `okf graph docs/knowledgebase/ --format mermaid` |
| One document's parsed structure | `okf parse docs/knowledgebase/concepts/<name>.md` |
| Attested Computation contracts | `okf computations docs/knowledgebase/` |
| Semantic diff of two bundles | `okf diff <a> <b>` |

**Mutating — state what you are about to run before running it.**

| Need | Command |
| --- | --- |
| Normalize formatting | `okf fmt -w docs/knowledgebase/` |
| Migrate legacy v0.1 fields | `okf validate --fix --author "<actor>" docs/knowledgebase/` |
| Scaffold a concept | `okf new docs/knowledgebase/<dir>/<slug> --type <Type> --title "..." --description "..."` |
| Rename/move, rewriting backlinks | `okf mv <old> <new> --bundle docs/knowledgebase/` |
| Delete with link safety | `okf rm <id> --bundle docs/knowledgebase/ [--redirect-to <id>]` |
| Extract a section to a new concept | `okf split <from> <to> --section "<heading>"` |
| Fold one concept into another | `okf merge <from> <into>` |

For every `mv`/`rm`/`split`/`merge`, run it with `--dry-run` first and show the user the plan. These rewrite links across the whole bundle; a wrong target silently edits many files.

**Interactive / build:** `okf studio docs/knowledgebase/` (terminal UI), `okf site docs/knowledgebase/` (static HTML; prefer `make site`, which sets the title and output dir).

## Trust: what an agent may and may not write

Trust tiers are **derived**, never stored. `okf trust` computes them from `generated` and `verified`:

- `human-reviewed` — some `verified[].by` starts with `human:`
- `machine-confirmed` — verified only by a process or agent actor
- `unverified` — no `verified` entries

That derivation is the whole reason the trust layer means anything, so:

1. **Never write a `verified:` entry.** Not for yourself, not on the user's behalf. A `verified.by: human:<id>` you wrote is a forged claim that a person checked the content. Verification is recorded by the human who actually did the review. Every concept in this bundle is currently `unverified`, and that is correct until someone reviews one.
2. **AI-authored content gets AI attribution.** `generated.by` is whoever produced the text, in one of the three actor forms — `human:<id>`, `process:<id>`, or `<producer>/<version>`. Anything else is flagged `L6`. This bundle's agent actor is **`claude-code/1.0`**; use it for anything you author or substantially rewrite, and reserve `human:<id>` for prose a person actually wrote. Do not attribute your own output to the user, even when they directed the research — that is the same misrepresentation as a forged `verified` entry, one tier down.
3. **A material rewrite invalidates an existing `verified` entry.** If you substantially change a concept that carries one, remove it — it vouched for text that no longer exists. Say so in your summary.
4. **`generated.at` marks the last *meaningful* change.** A mechanical, meaning-preserving pass (reflowing, a link-form change, `okf fmt`) leaves it alone. A prose rewrite refreshes it.
5. **Staleness is wall-clock dependent and therefore opt-in.** `okf validate` is deterministic by default; pass `--today YYYY-MM-DD` to evaluate `stale_after`. Use a pinned date when you need a reproducible result.

## Frontmatter

`type` is the only required key — a bundle is conformant iff every non-reserved `.md` has parseable frontmatter with a non-empty `type`. Recommended: `title`, `description`, `generated`. Types in use here: `Concept`, `Pattern`, `Plan`, `Project`, `Reference` (and `Decision` once `docs/knowledgebase/decisions/` is populated).

`okf fmt` does **not** reorder frontmatter keys — that is `L2`, and only `lint --fix` touches it. To clear an `L2` by hand, order keys as: `type`, `resource`, `title`, `description`, `tags`, `status`, `generated`, `verified`, `stale_after`, `sources`, `usage_window`. Unknown keys are preserved and sort after those.

`index.md` and `log.md` are reserved and are never concepts. The bundle-root `index.md` carries only `okf_version: "0.2"` — quoted, or `L13` fires.

## Exit codes and JSON

`0` success · `2` bad command line · `65` data error (non-conformant, unformatted, or lint findings) · `66` input not found.

`okf lint` exits `65` on warnings, which is why the pre-commit hook deliberately runs only `fmt` and `validate` — lint findings are advisory here and would otherwise block every commit.

Every subcommand takes `--json` (`-j`) for machine-readable output. Prefer it when you need to act on specific findings rather than show the user a report.

## This repo's gates

`hk.pkl` registers, on `pre-commit` (and `hk check` / `hk fix`), against `docs/knowledgebase/` as a whole:

- `okf fmt --check docs/knowledgebase/` (fix: `okf fmt --write docs/knowledgebase/`)
- `okf validate docs/knowledgebase/`

So before committing a `docs/knowledgebase/` change, run:

```sh
okf fmt -w docs/knowledgebase/ && okf validate docs/knowledgebase/
```

`okf lint docs/knowledgebase/` is worth reading but is not a gate. The standing findings are `L11` (unverified — expected, see above) and one `L3` on `docs/knowledgebase/references/okf-spec.md`.

## Reference

`reference.md` has the full `V1`–`V36` conformance and `V`/`L` lint code tables, the complete frontmatter key lists, and the actor and link-form rules.

## After changing docs/knowledgebase/

The knowledge-base policy in `docs/knowledgebase/index.md` applies: add the one-line pointer to the matching category index and a dated entry to `docs/knowledgebase/log.md` (newest date first, matching the existing bullet style). Those two files are hand-maintained — see the destructive-commands section.
