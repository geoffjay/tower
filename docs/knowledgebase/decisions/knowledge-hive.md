---
type: Decision
title: Knowledge hive — a shared OKF knowledge base behind tower, with a derived SQLite index
description: Fleet knowledge lives as OKF markdown in one directory; tower owns deposit, query, and awareness through CLI verbs, REST routes, and MCP tools; retrieval is a derived SQLite index (FTS5 now, sqlite-vec spike); graph DBs are optional operator-run backends; tinyhivemind mechanics (grounded support, decay, cross-inhibition, evidence) are adopted as principles, not code.
tags:
  - decision
  - knowledge
  - hive
  - okf
  - mcp
  - graph
status: proposed
generated:
  by: omp/glm-5.3
  at: "2026-10-26T22:45:37Z"
---

# Knowledge hive

Date: 2026-10-26. Research-driven proposal — not yet planned or built. Inputs:
[tinyhivemind research](../references/tinyhivemind.md),
[knowledge-store research](../references/knowledge-stores.md), the existing
OKF knowledge base policy (`docs/knowledgebase/index.md`), and the tower
codebase as of v0.4.0.

## Problem

Agents in the fleet finish jobs with prose in the pane; `tasks.result` holds
only the owner-reported summary, questions expire from view, and nothing
makes one agent's hard-won finding reusable by the next. tower is the entry
point for the fleet — it owns identity (`TOWER_AGENT`), transport (REST/MCP/
CLI), the job loop, and the event log — so it is the natural place to own a
knowledge substrate the whole hive can deposit into and query. Knowledge is
the input to better decisions; today the fleet's knowledge is scattered
across panes, transcripts, and the operator's head.

## Decision

### 1 — The substrate of record is an OKF markdown directory

Fleet knowledge is OKF v0.2 markdown (one concept per file, YAML frontmatter,
cross-links, human- and agent-editable) in a directory tower serves. Files are
the record; indexes are derived and rebuildable. This mirrors the working
`docs/knowledgebase/` in this very repo, which already proved agents and
people can co-author durable knowledge — tower would productize the pattern
for any agent fleet, not just this repo's own tooling agents.

Why not a graph DB as the substrate: nothing license-clean is also
operationally free (FalkorDB SSPL, Memgraph BSL, Kuzu archived — details in
the [store research](../references/knowledge-stores.md)). Markdown keeps the
record human-readable, git-diffable, and portable; the graph is *derived*
from links + tags, so index corruption is a rebuild, never data loss.

### 2 — tower owns deposit, query, and awareness

A `knowledge` module in `tower-server` (following the flat-module pattern:
`pub mod knowledge` in `lib.rs`, `router()` merged in `serve.rs`, service fns
shared by REST and MCP like every other module):

- **CLI verbs** (skills drive the CLI, per
  [operator-skills](operator-skills.md)): `tower knowledge search <query>`,
  `tower knowledge show <id>`, `tower knowledge deposit` (file/stdin), a
  `tower-knowledge` skill teaching when/what to deposit and how to search
  first. Agents without MCP (pi) get the same loop via CLI, exactly like
  the task work loop.
- **REST**: `GET /v1/knowledge?q=` (search), `GET /v1/knowledge/{id}`,
  `POST /v1/knowledge` (deposit). Deposits are attributed rows: author agent
  (from `X-Tower-Agent`/`as`), job link (`task_id`), citations.
- **MCP tools**: `tower_knowledge_search`, `tower_knowledge_show`,
  `tower_knowledge_deposit` — same service fns, one transaction, no second
  implementation to drift (the standing tower rule, D§7).
- **Awareness**: the agent work-loop contract (`docs/agent-loop.md`,
  `tower contract`, MCP `initialize.instructions`) gains one line — search
  the hive before starting, deposit findings when reporting. The
  phase-2c standing brief restated per delivery can carry "search first".

### 3 — Retrieval is a derived index inside the existing SQLite

Phase the retrieval:

1. **FTS5** (available today, zero dependencies — the bundled SQLite already
   compiles FTS5 in): index doc bodies into `fts5` virtual tables synced on
   deposit; search = BM25 keyword match.
2. **sqlite-vec** spike (MIT crate; the one dependency that preserves
   single-binary + single-database): store embeddings as BLOBs, brute KNN.
   Open question: sqlx `load_extension` gate — if sqlx can't load it at pool
   setup, fall back to a dedicated `rusqlite` connection for the knowledge
   index only. Embeddings require a model call — where the embedding comes
   from (operator's API key, harness sidecar, local GGUF) is an open
   question, since tower deliberately owns no model client today.
3. **Graph DB backends** (FalkorDB/Memgraph/LadybugDB): optional,
   operator-run, config-gated `[knowledge] backend` — never bundled, never
   default. Adopt only when multi-hop traversal over accumulated knowledge
   is a demonstrated need, not an anticipated one.

The index stores: doc path, title, description, tags, links (from
`okf graph --format json` semantics — a derived edge table), author, task
link, timestamps — enough to implement salience-style ranking
(recency/importance) in SQL without a graph engine.

### 4 — Provenance and trust travel with the deposit

OKF discipline maps directly: `generated.by` = the depositing agent (AI
attribution, never a human claim), optional `verified` stays human-only,
`stale_after` for time-sensitive facts. A deposit carries citations
(evidence, not opinion — the tinyhivemind benchmark finding), and
`task_id` links knowledge back to the job that produced it, closing the
loop with the run-outcomes decision ([outcomes-and-brief](outcomes-and-brief.md)):
the durable `result` requirement is the funnel; the hive is where results
become reusable knowledge.

### 5 — tinyhivemind mechanics, adopted as principles not code

License rules out linking (GPL-3.0 vs Apache-2.0, and the crates are
unpublished submodule pins). What transfers (details in the
[tinyhivemind research](../references/tinyhivemind.md)):

- **Stigmergy**: deposits stimulate pulls, not pushes — knowledge is queried
  when a job needs it; the dispatcher's delivery is the only push.
- **Decay + importance floor**: ranking blends recency (events, `stale_after`)
  over standing importance; a stale doc is not gone, just demoted.
- **Grounded quorum**: conflicts between deposits are adjudicated by counting
  distinct grounded supporters visible in the event log — never
  last-writer-wins, never averaging.
- **Cross-inhibition**: an objection event names the *deposit* and silences
  its advocate on that claim; it does not debit the topic.
- **Correlation boundaries**: stacks are desks — knowledge must cross stacks
  through the hive (the 77.5% vs 0.2% referral result), so the hive is
  fleet-global, not per-stack.
- **Evidence not opinion**: deposits cite sources; unsupported deposits
  rank below cited ones. The benchmark where evidence broadcasts reached
  the free-information ceiling and opinion plateaued at 62.5% is the reason.
- **Bounded views**: what an agent gets per query is bounded (top-k, short
  context, citations) — "lost in the middle" punishes bigger windows.
- **Named termination**: a knowledge query that finds nothing says so.

Their own retired experiments (asides, refutation caps, delegation bonuses)
are the caution list: don't cargo-cult mechanisms that measurably hurt.

## Alternatives rejected

- **Link tinyhivemind-core into tower**: GPL-3.0 contamination of an
  Apache-2.0 binary; also pulls the transcript/desk model in before tower
  needs it. Revisit only if fleet deliberation becomes a phase.
- **FalkorDB/Memgraph as the default substrate**: second process + non-OSI
  /BSL license + memory-resident durability; wrong default for a single
  static binary. Keep as config-gated optional backends.
- **Adopt LadybugDB now**: the right *shape* (MIT embedded graph, Kuzu
  successor) but a one-year-old post-acquisition fork — too young for a
  fleet coordinator's knowledge-of-record. Watch-list.
- **SQLite tables as the substrate (markdown as rendering)**: markdown files
  as record keep human/git portability and OKF tooling; SQL rows would lose
  the format's existing ecosystem (`okf validate/search/graph`) and the
  human-editability that makes co-authoring work.
- **Do nothing**: phase 2c results make outcomes durable but not
  *discoverable*; the next job still starts blind. The hive is the missing
  discovery half.

## Consequences

- New `knowledge` module + migration 0007 (index tables), `[knowledge]`
  config section, `tower knowledge` verbs, three MCP tools, one skill,
  contract text update, `GET /v1/schema` registry entries, two event kinds
  (`knowledge.deposited`, `knowledge.queried` is wrong — query events would
  flood the log; emit `knowledge.deposited` only).
- The `artifacts` table finally gets a producer relationship (knowledge
  deposits may attach as artifacts) — or it stays unused; decide at plan
  time.
- Embedding source for sqlite-vec is an open question (tower owns no model
  client).
- Graph-DB backend abstraction is deliberately *not* built now; the config
  key reserves the option.
- Design amendments at plan time: D§9 (new module), D§7 (routes/tools),
  D§6 (index tables), D§10 (CLI verbs), D§17 (open questions).

## Open questions

1. sqlx `load_extension` feasibility for sqlite-vec (spike).
2. Embedding source for semantic search (spike; depends on operator model
   access; may defer the whole semantic tier).
3. Where the hive directory lives: per-fleet (tower state dir) vs per-project
   (workdir-local, like the repo's own `docs/knowledgebase/`). Lean
   per-fleet with per-project scoping via tags, but the repo case suggests
   per-project may be what operators actually run.
4. Whether deposits are ever *pushed* to agents (brief-style restatement of
   a top-k knowledge digest per job delivery) — the bounded-view principle
   says yes, the stigmergy principle says pull-only. Resolve in a spike
   informed by phase-2c delivery mechanics.
5. Pruning/supersession policy for knowledge docs (the `okf` bundle has
   none) — defer until the hive has content and real conflicts.

## Non-goals

- No deliberation protocol, no quorum engine, no scheduler changes — this
  decision is only the knowledge substrate, its tools, and its ranking.
- No per-agent memory/persona store — the hive is shared, attributed, and
  job-linked, not a per-agent memory product.
- No GUI beyond what the web UI already renders (a knowledge pane can come
  later; it is not load-bearing).
- No change to `docs/knowledgebase/` in this repo — that bundle stays the
  project's own KB under its own policy; the hive is a tower feature for
  fleets, informed by it.
