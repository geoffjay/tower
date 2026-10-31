---
type: Reference
title: Knowledge-store research
description: Graph stores and markdown-directory alternatives for a shared agent knowledge hive — FalkorDB, Memgraph, Kuzu forks, Graphiti, sqlite-vec, and FTS5 — with licenses, Rust clients, and tower fit.
resource: https://docs.falkordb.com
tags:
  - reference
  - research
  - knowledge-graph
  - graph-database
  - falkordb
  - memgraph
  - kuzu
  - okf
status: stable
sources:
  - resource: https://docs.falkordb.com
    title: FalkorDB documentation
  - resource: https://github.com/FalkorDB/falkordb-rs
    title: falkordb-rs Rust client
  - resource: https://github.com/memgraph/memgraph/blob/master/licenses/BSL.txt
    title: Memgraph BSL 1.1 license text
  - resource: https://github.com/kuzudb/kuzu
    title: Kuzu repository (archived October 2025)
  - resource: https://github.com/LadybugDB/ladybug
    title: LadybugDB (Kuzu successor, MIT)
  - resource: https://github.com/getzep/graphiti
    title: Graphiti temporal knowledge-graph framework
  - resource: https://github.com/asg017/sqlite-vec
    title: sqlite-vec SQLite vector-search extension
generated:
  by: omp/glm-5.3
  at: "2026-10-26T22:45:37Z"
---

# Knowledge-store research

Evaluated for tower's knowledge hive: what holds the shared knowledge agents
deposit into and query. tower is a single static binary, Apache-2.0, SQLite
via sqlx 0.8 (bundled libsqlite3-sys 0.30.1, WAL, one db file), no new
long-running services in the default shape ([stack](../concepts/design/02-stack.md)).

## The candidates

### FalkorDB — SSPL v1, Redis-module graph DB

In-memory property graph built on Redis with sparse-matrix (GraphBLAS)
representation; Cypher subset (openCypher 9 + extensions); native **vector
index** for GraphRAG; official Rust client
[falkordb-rs](https://github.com/FalkorDB/falkordb-rs) (MIT) with async/tokio
support, typed results, and an `embedded` feature (module downloaded at
runtime) or `embedded-bundle` (bundled at build time) to run a self-contained
server without a separate Redis install.

- **Strengths**: fastest GraphRAG story in this list (its positioning);
  vector + full-text indexes; embedded mode avoids "install Redis first".
- **Costs**: SSPL v1 is **not OSI open source** and is stricter than AGPL on
  service offering; embedding it in tower's distributed binary is a licensing
  non-starter for an Apache-2.0 project, so it could only ever be an optional
  operator-run sidecar. Second process, second persistence dir, second
  backup/restore story; embedded mode downloads a binary at runtime or
  vendors it into the build. Memory-resident (durability via Redis
  persistence options, not a write-ahead log of record).
- **Fit**: optional backend only, if fleet knowledge outgrows SQLite and
  genuine multi-hop Cypher traversal becomes load-bearing. Not the default.

### Memgraph — BSL 1.1, in-memory Cypher

High-performance in-memory graph DB (C++), Cypher-compatible, Bolt protocol;
works with `neo4rs` (MIT) or `rsmgclient` in Rust.

- **License**: BSL 1.1 (Jan 2026 text): production use is an "Authorised
  Purpose" — internal business use allowed; **embedded/distributed to third
  parties prohibited**; DBaaS/competing uses prohibited; Change Date
  2030-10-21, then Apache-2.0. Permits tower-the-software shipping *support*
  for Memgraph (tower never distributes Memgraph), but forbids tower-the-app
  bundling it; a company distributing tower+Memgraph internally is fine, as a
  product is not.
- **Costs**: separate server process (Docker image or native install);
  in-memory (p Durability via periodic snapshots; not a durable log of
  record); BSL adds terms tower users would have to read; no embedded mode.
- **Fit**: same bucket as FalkorDB — operator-run optional backend. Slightly
  more permissive license than SSPL, same operational shape (second process).

### Kuzu — MIT embedded graph DB — **archived**

Embedded columnar property-graph DB, Cypher, built-in vector + full-text
search; `kuzu` Rust crate (MIT). Would have been the natural embedded choice
for tower... except Kùzu Inc was **acquired by Apple and the project was
archived in October 2025**; no releases since 0.11.3 (Sept 2025); the Rust
crate's docs note it pins the 2022–2025 C++ code.

- **Forks**: [LadybugDB](https://ladybugdb.com) (Kùzu Inc. alumni, MIT,
  active — blog posts and releases through mid-2026, "DuckDB for graphs", Rust
  client installable via Cargo) and **Bighorn** (Kineviz community fork).
  The Data Quarry and OneUptime write-ups (Apr–Aug 2026) treat LadybugDB as
  the maintained successor.
- **Fit**: pinning dead Kuzu is not viable; LadybugDB inherits the embedded
  MIT profile that tower wants, but it is a very young project (post-Apple,
  one-year-old fork) — a real dependency risk for a fleet coordinator whose
  KB outages degrade agent decision-making. Watch-list candidate, not a
  phase-1 dependency. The `embedded-bundle` story (like falkordb-rs) matters
  for tower's single-binary rule.

### Graphiti — Apache-2.0, but Python

getzep/Graphiti (Apache-2.0): the reference design for **temporally-aware,
episodic knowledge graphs for agents** — ingests "episodes" (events or
messages), extracts entities/relations with LLMs, resolves against existing
nodes (dedup/merge), keeps `valid_at`/`invalid_at` temporal edges, hybrid
search (semantic + BM25 + graph). FalkorDB is one of its supported backends;
the design transfers cleanly to a Rust port of just the ideas.

- **Fit**: don't adopt the framework (Python runtime does not belong in
  tower's process model), **steal the schema**: episode → extraction →
  temporal edge resolution is exactly how a tower knowledge graph should
  ingest agent messages and results. Its bi-temporal edge model is the right
  mental model for "what was believed, when, and who said it".

### sqlite-vec — MIT Rust crate, vector search in SQLite

`sqlite-vec` (MIT, by Alex Garcia, successor to sqlite-vss): tiny C extension
(loads via `sqlite3_auto_extension`, no `unsafe` in tower's code — the crate
exposes a safe `sqlite3_vec_init` entry point; note the example uses
`unsafe` for auto-extension registration; sqlx loads extensions through its
own mechanism, but sqlx's `SqliteConnectOptions` does not expose a
`load_extension` gate yet — verifying feasibility is a spike). Pre-v1, expect
breaking changes; brute-force KNN with ANN on the roadmap.

- **Fit**: the natural upgrade for semantic search over knowledge docs
  **inside tower's existing SQLite**, no second database. Feasibility
  spike: load the extension at pool setup (one-time `load_extension`), store
  embeddings as BLOBs, query via `vec_distance_cosine`-style SQL. If sqlx
  can't load it, fall back to the `rusqlite` pool for the knowledge tables
  only, or short windows. This is the one dependency that keeps the
  single-binary + single-database story intact while adding semantic recall.

### FTS5 — already in tower's SQLite

libsqlite3-sys 0.30.1 bundled build compiles with `-DSQLITE_ENABLE_FTS5`
(and FTS3/RTREE/JSON1) — verified in the rusqlite build script. sqlx's
bundled SQLite therefore supports `CREATE VIRTUAL TABLE … USING fts5(...)` out
of the box, zero new dependencies. Keyword full-text search over knowledge
docs is available *today*.

- **Fit**: baseline retrieval primitive, free. Pair with `snippet()`/`rank`
  for relevance. A hybrid FTS5 (keyword) + sqlite-vec (semantic) is the
  DuckDB-era standard hybrid search and stays inside SQLite.

## The markdown-directory alternative (OKF)

Instead of a graph DB, keep knowledge as **OKF markdown in a directory** the
way this repo's own `docs/knowledgebase/` works, and make tower the aware-
ness layer: agents deposit/query through tower tools, tower indexes the
directory into SQLite (FTS5 + optional sqlite-vec), and the graph lives in
the **link structure** (`okf graph --format json` already emits it) plus
frontmatter tags as ad-hoc edge types.

- **Strengths**: human-readable, human-editable, git-diffable, OKF-conformant
  (agents and people author together — the exact thesis of this repo's KB);
  zero new infrastructure; the graph is *derived* (rebuildable), so a corrupted
  index is a rebuild, not a data-loss event; the `okf` CLI already validates/
  lints/links-checks/searches. tower's [operator-skills](../decisions/operator-skills.md)
  pattern already ships CLI-driven skills, and `tower knowledge` verbs would
  follow `tower-task-*` precedent.
- **Costs**: no ad-hoc multi-hop Cypher; entity resolution is manual (front-
  matter `tags` + wiki-links instead of a real entity graph); write path needs
  discipline (frontmatter required, generated/verified provenance) — which is
  exactly what OKF frontmatter + `okf validate` already enforce; concurrent
  writes from multiple agents need a merge queue (serialize via the single
  SQLite writer) rather than free file mutation.

## Summary

| Option | License | Process model | Rust client | Retrieval | tower verdict |
|---|----|---|----|---|---|
| FalkorDB | SSPL v1 (not OSI) | separate server / embedded module | falkordb-rs (MIT) | Cypher + vector + FTS | optional backend, not default |
| Memgraph | BSL 1.1 → Apache-2.0 2030 | separate server | neo4rs (MIT) | Cypher + vector (enterprise) | optional backend, not default |
| Kuzu | MIT (archived Oct 2025) | embedded | kuzu (MIT) | Cypher + vector + FTS | dead upstream; pin nothing |
| LadybugDB | MIT | embedded | cargo client | Cypher + vector + FTS | watch-list successor, too young to adopt |
| Graphiti | Apache-2.0 | Python framework | — | temporal KG hybrid | steal the schema, not the runtime |
| sqlite-vec | MIT | extension in existing SQLite | sqlite-vec (MIT) | brute KNN → ANN | adopt pending sqlx load-extension spike |
| FTS5 | public domain (bundled) | already inside SQLite | (sqlx) | BM25 keyword | free today — baseline primitive |
| OKF directory | Apache-2.0 (the format) | files + derived index | okf (Rust) | okf search + links/graph | the primary substrate |

Nothing among the true graph databases is both license-clean and
operationally free. The evidence points to: **OKF markdown directory as the
substrate of record, SQLite-resident derived index (FTS5 today, sqlite-vec
spike for semantic) for retrieval, graph DBs as optional operator-run
backends if multi-hop traversal ever becomes load-bearing.** See the
[knowledge-hive decision](../decisions/knowledge-hive.md).
