---
type: Reference
title: tinyhivemind research
description: tinyhivemind hive-mind mechanics (stigmergy, quorum, cross-inhibition, decay), its host contract, measured results including honest negatives, and what transfers to tower's knowledge hive.
resource: https://github.com/tinyhumansai/tinyhivemind
tags:
  - reference
  - research
  - tinyhivemind
  - hive
  - knowledge
  - deliberation
status: stable
sources:
  - resource: https://github.com/tinyhumansai/tinyhivemind
    title: tinyhivemind README and repository
  - resource: https://github.com/tinyhumansai/tinyhivemind/wiki/Host-integration
    title: Host integration (ports the host must supply)
  - resource: https://github.com/tinyhumansai/tinyhivemind/wiki/Architecture
    title: Architecture (crate boundaries)
  - resource: https://github.com/tinyhumansai/tinyhivemind/blob/main/ROADMAP.md
    title: Roadmap with phase table and retired experiments
generated:
  by: omp/glm-5.3
  at: "2026-10-26T22:45:37Z"
---

# tinyhivemind research

tinyhivemind (tinyhumansai, Rust, GPL-3.0-only, ~19 stars) is "hive mind
mechanics for agents": quorum sensing, cross-inhibition, stigmergy, salience
decay, and response thresholds, implemented as **pure integer folds over a
transcript the host already owns**. It is not a model, not a message bus, and
**not a knowledge store** — `tinyhivemind-core` "opens no storage, socket, or
model client". The host keeps the log, sessions, credentials, authorization,
and scheduling; the library reads snapshots and returns decisions.

## License is a hard constraint

GPL-3.0-only vs tower's Apache-2.0. Apache-2.0 code may be included *into* a
GPLv3 work (one-way compatibility), not the reverse: statically linking
tinyhivemind into `tower` would make the distributed combined work GPL-3.0,
which tower's Apache-2.0 distribution cannot do. A process boundary (separate
GPL executable over IPC) keeps the works separate but drops most of the
integration value. The crates are also `publish = false`, pinned as a git
submodule. **Conclusion: adopt the mechanics as design principles; do not
adopt the code.** (See the [knowledge-hive decision](../decisions/knowledge-hive.md).)

## The mechanics (what actually transfers)

- **Stigmergy** — work leaves a trace in a shared medium and the trace is the
  stimulus for the next work. No agent addresses another; nothing dispatches.
  For tower: the KB directory is the persistent trace field; the event log is
  the volatile pheromone channel. A knowledge deposit should stimulate reads,
  not send messages.
- **Pheromone decay** — a trace's salience decays exponentially with distance
  in the transcript, over a floor of standing importance; without it, whoever
  spoke first holds the floor forever. For tower: knowledge needs a recency
  term (event log, `okf` `stale_after`) layered on durable importance, not
  recency alone.
- **Quorum sensing** — an option carries when some number of *distinct
  participants* have grounded support for it inside a window. The count is
  local, order-independent, and idempotent: an agent that catches up late
  folds to the same standing as one that watched live. For tower: conflicting
  knowledge deposits should be adjudicated by counting grounded support from
  the events log — a pure, replayable fold — never by last-writer-wins.
- **Cross-inhibition** — an objection names a *message* and removes that
  message's author from the supporter set of whatever they advocated; it does
  not debit the option. Silencing an advocate can break a tie; subtracting
  scores cannot. For tower: disagreement events should carry the target
  (which claim, which deposit) and silence its advocate, not subtract from
  the topic.
- **Response thresholds / attention market** — every member computes an urge
  from the salience field and its own affinity; speaking raises the speaker's
  threshold, silence lowers others'. For tower: a fleet knowledge delta should
  be *pull* (agents query when a job needs it), and the dispatcher's delivery
  is the only push.
- **Referral across correlation boundaries** — a desk is a correlation
  boundary: members of one room are wrong about the same things, and
  deliberating inside the channel cannot cancel a shared error. A referral is
  at most one turn on another desk with one answer carried back. Their
  federated benchmark: three desks each confidently wrong converge correctly
  **0.2%** of the time inside channels, **10.5%** when merged into one big
  room (factions never reach quorum), **77.5%** with cross-desk referral.
  For tower: a stack (`<stack>-<role>`) is exactly such a boundary; knowledge
  must be shared *across* stacks through the hive, not per-stack chat.
- **Evidence, not opinion** — planting a disqualifying fact on a desk other
  than the one that needs it: an evidence broadcast closes the gap to the
  free-information ceiling at a thousand agents where an opinion broadcast
  plateaus at 62.5%. For tower: deposits should carry citations/sources, and
  `okf` trust rules (never write `verified`) are the same discipline.
- **Bounded view with pins** — the projection is ~30 messages; "lost in the
  middle" means a bigger window relocates the problem into its own middle.
  Pins fold out of the transcript and ride into every turn. For tower: the
  phase-2c standing brief restated per delivery is the pin mechanism; a
  knowledge briefing must be bounded the same way.

## Host contract (why tower's shape already fits)

The host owns the append-only log and assigns a global sequence; core pages it
newest-first through `SessionLog::read_before` and gets attributed projections
(`project_session`), watermark deltas (`SharingState`), and digests through a
`Digester` port. Every turn is attributed so agent B never reads agent A's
words as its own — the two defects the project exists to fix (first-person
collapsed transcripts; non-continuous sharing).

tower's `messages` table already carries the attribution the projection needs
(`from_kind`, `from_id`, `to_kind`, `to_id`, `parts`, `created_at`; ULID ids are
time-ordered). A future deliberation feature would map the `SessionLog` port
onto the messages store with no schema change. That compatibility is a
finding, not a plan.

Crates: `tinyhivemind-core` (grammar, folds, ports), `-tools` (native tool
specs), `-hives` (durable coordination, async `Storage` port: memory/SQLite/
host), `-lang` (portable hive packages), `-openhuman` (adapter), `-exec`
(script tools). A step returns `Converged | Deadlocked | Exhausted | Idle` or
`Speak { turns, next_state }` — bounded rounds (`round_width` blind,
`revealed_width` revealed); every episode replays byte-for-byte from the same
transcript; all arithmetic is fixed-point.

## Measured results, including the negatives

- Five agents, four options, 5000 seeded rooms: single responder 57.6% (1
  turn), plurality vote 78.5% (15 turns), tuned hive episode **82.1%** (6.75
  turns). Simulated participants — it measures the protocol, not model gains.
- One task, one agent vs one seat per facet: soloist accuracy decays with
  width (78.7% → 7.8% at 8 facets); seats stay flat (78.7% → 13.0%). Divide
  along a line where competence differs or the advantage vanishes.
- **Asides lost** (2.5 points tuned, 15.3 on a hidden profile); the follow-up
  retracted the correlated-error explanation — *spending a floor turn* caused
  the loss. Private asides stay off by default.
- **Refutation caps and evidential quorum removed** — they reduced accuracy
  in the original benchmark and lost again under a hidden-profile run.
- **Delegation and deferral removed** — no `Knows` floor award and no
  deferral use across 27 live rounds.

The negatives are why the right adoption is the *principles* (grounded
support, decay, cross-inhibition, evidence, bounded views, named termination)
rather than the protocol wholesale: several of its own mechanisms measurably
hurt, and it self-reports that it is "a protocol for bounded deliberation with
an auditable termination reason, and nothing more".

## What it is not

Not a knowledge graph or any store; no embedding-backed retrieval in the
adoption path (an optional `embed` module exists for routing); no execution —
"the host owns agent construction, sessions, credentials, and authorization";
non-goals explicitly include a second journal, a web framework, and
unbounded fan-out.

## Reference material

- Repository: https://github.com/tinyhumansai/tinyhivemind (README, ROADMAP)
- Wiki: Host-integration, Architecture, Hive-episodes, Episode-policy,
  Cross-desk-referral, Benchmarks
