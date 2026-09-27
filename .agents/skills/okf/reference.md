# okf reference

Full rule tables and field lists for `okf 0.3.0 (OKF spec v0.2)`. Read this when you need to interpret a specific diagnostic code; `SKILL.md` covers which command to run.

## Conformance codes (`okf validate`)

A bundle is **conformant** iff (1) every non-reserved `.md` file has a parseable frontmatter block, (2) every frontmatter has a non-empty `type`, and (3) reserved files follow their structure when present.

Everything else is soft guidance: a consumer must not reject a bundle for missing optional fields, unknown types or keys, broken links, or missing `index.md` files. Accordingly only the `error` rows below make the bundle non-conformant. `warning` means a producer mistake worth fixing; `info` means a permitted state worth noting.

| Code | Severity | Finding |
| --- | --- | --- |
| V1 | error | unparseable concept document (frontmatter parse error) |
| V2 | error | missing or non-scalar required `type` field |
| V3 | warning | missing recommended frontmatter field (`title`, `description`, `generated`) |
| V4 | warning | concept body is empty |
| V5 | warning | `tags` is not a YAML list of strings |
| V6 | warning | `generated` is malformed, missing `by`, or `at` is not a valid ISO datetime |
| V7 | warning | `verified` is malformed, missing `by`, or `at` is not a valid ISO datetime |
| V8 | warning | latest `verified.at` predates `generated.at` (content modified after verification) |
| V9 | warning | timestamp in `generated`, `verified`, or `sources` is in the future |
| V10 | warning | unknown `status` value (not `draft`, `stable`, `deprecated`) |
| V11 | warning | `stale_after` is not a valid ISO datetime |
| V12 | info | stale concept (past `stale_after`; only with `--today`) |
| V13 | warning | `sources` malformed, missing `resource`, or duplicate `id` |
| V14 | warning | `sources.last_modified` or `usage_window` not a valid ISO datetime |
| V15 | warning | `sources.usage_count` without `usage_window`, or not an integer |
| V16 | warning | footnote attribution matches no `sources[].id` |
| V17 | warning | footnote cited in body but never defined |
| V18 | warning | circular concept derivation in the sources graph |
| V19 | warning | legacy v0.1 `timestamp` present (superseded by `generated`) |
| V20 | warning | legacy v0.1 body `# Citations` list present (superseded by `sources`) |
| V21 | warning | missing `runtime` or `computation` source on an `Attested Computation` |
| V22 | warning | contract `parameters`, `executor`, or `attester` missing required fields |
| V23 | warning | `executor`, `attester`, or `computation` resource missing on disk |
| V24 | warning | inline `# Computation` code block syntax error |
| V25 | warning | computation, executor, or attester script syntax error |
| V26 | info | computation fields on a non-computation concept type |
| V27 | info | explicit path field does not resolve to a file in the bundle |
| V28 | info | broken link (target does not resolve to a concept in the bundle) |
| V29 | warning | links to a `status: deprecated` concept |
| V30 | warning | `title` shared with another concept |
| V31 | warning | concept-id segment outside the portable ASCII set |
| V32 | error | reserved `index.md`/`log.md` unreadable, unparseable, or bad frontmatter |
| V33 | error | reserved `log.md` structural error or invalid ISO date format |
| V34 | warning | `log.md` contains a duplicate `## YYYY-MM-DD` date heading |
| V35 | warning | existing `index.md` is out of sync with its directory |
| V36 | info | bundle declares an unrecognized or non-target `okf_version` |

**V35 is expected in this repo and must not be "fixed".** The indexes here are hand-curated prose; the only tools that resolve V35 are `okf index` and `okf lint --fix`, both of which destroy `docs/knowledgebase/index.md`. Leave it.

`--fix` on `validate` remediates the mechanical legacy migrations (notably V19/V20) and takes `--author "<actor>"` to record who applied them. It does not touch indexes.

## Lint codes (`okf lint`)

Opinionated authoring hygiene. **No lint finding is a conformance failure** — a bundle with lint findings is still conformant if `okf validate` says so. `okf lint` exits `65` when there are warnings.

| Code | Severity | Fixable | Finding |
| --- | --- | --- | --- |
| L1 | warning | yes | body has no top-level `#` heading |
| L2 | info | yes | frontmatter keys not in canonical preferred order |
| L3 | warning | no | heading hierarchy drift (levels skipped, or multiple `#`) |
| L4 | warning | no | empty/stub section heading with no content |
| L5 | warning | no | source declared in frontmatter but never cited with a footnote |
| L6 | info | no | non-standard actor identity in `generated`, `verified`, or `sources.author` |
| L7 | warning | yes | `# Computation` code block missing a language tag |
| L8 | info | yes | trailing whitespace or excess blank lines in the body |
| L9 | warning | no | orphan: no inbound links and not listed in any `index.md` |
| L10 | info | no | self-link (concept links to itself) |
| L11 | info | no | no `verified` events; trust tier is `unverified` |
| L12 | info | no | `status: draft` |
| L13 | warning | yes | `okf_version` in root `index.md` is unquoted |

Only `L1`, `L2`, `L7`, `L8`, `L13` are auto-fixable — and `lint --fix` regenerates indexes as a side effect, so prefer hand-fixing. See `SKILL.md`.

`L11` fires on all seven concepts here and is correct: nothing has been human-reviewed yet. Do not silence it by writing `verified` entries.

## Frontmatter keys

**Required** (the one hard rule): `type`.

**Recommended** — absence is never a conformance failure: `title`, `description`, `generated`.

**Canonical order** (what `L2` checks; presentational only, no semantic meaning):

```
type, resource, title, description, tags, status, generated, verified, stale_after, sources, usage_window
```

**All spec-meaningful keys** — anything else is a producer extension and is preserved untouched on round-trip:

- Core: `type`, `title`, `description`, `resource`, `tags`
- Provenance: `sources`, `usage_window`
- Trust: `generated`, `verified`
- Lifecycle: `status`, `stale_after`
- Attested Computation: `runtime`, `parameters`, `computation`, `executor`, `attester`
- Legacy (v0.1, retired): `timestamp`

**Values:** `status` is one of `draft`, `stable`, `deprecated`; absent means stable. `stale_after` is an absolute date.

## Actors

Every identity field — `generated.by`, `verified[].by`, `sources[].author` — uses one of three forms:

| Form | Kind | Example |
| --- | --- | --- |
| `human:<id>` | human | `human:geoff` |
| `process:<id>` | process | `process:ci-nightly` |
| `<producer>/<version>` | agent | `claude-code/1.0` |

Anything else parses as kind `Other` and trips `L6`. For the agent form, both the producer and version must be non-empty and the version must not itself contain `/`.

**This bundle's agent actor is `claude-code/1.0`.** All seven current concepts carry it, because all seven were AI-authored; `human:<id>` is reserved for prose a person actually wrote.

The `human:` prefix is the sole input to the `human-reviewed` trust tier. That is why an agent must never write it into a `verified` entry — and why it does not belong in `generated.by` on agent-written text either.

## Trust tiers

Derived at query time, never stored:

| Tier | Condition |
| --- | --- |
| `human-reviewed` | at least one `verified[].by` starts with `human:` |
| `machine-confirmed` | verified, but only by process or agent actors |
| `unverified` | no `verified` entries |

## Links

Both forms are valid OKF: bundle-absolute (`/concepts/foo.md`) and relative (`../concepts/foo.md`).

**Default to relative.** GitHub resolves the absolute form against the repository root and serves a 404, and Google's reference visualizer builds no graph edges from it. The cost is that a relative link breaks when the linking file moves — so re-run `okf links docs/knowledgebase/ --broken --check` after any `okf mv`.

A link asserts a relationship whose meaning lives in the surrounding prose, not in the link itself.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 2 | `EX_USAGE` — bad command line |
| 65 | `EX_DATAERR` — non-conformant, unformatted, or lint findings |
| 66 | `EX_NOINPUT` — input path not found |

## Reserved files

`index.md` and `log.md` are never concepts.

- Every `index.md` is a directory listing and carries no frontmatter — except the bundle-root `index.md`, which may declare `okf_version` (quoted; `L13`).
- `log.md` is a dated change history under `## YYYY-MM-DD` headings, newest first. Duplicate date headings trip `V34`; a malformed date is `V33`, an `error`.
