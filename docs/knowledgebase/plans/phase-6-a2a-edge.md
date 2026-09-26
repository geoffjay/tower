---
type: Plan
title: Phase 6 — A2A edge
description: Agent Card discovery and A2A delegation/streaming for foreign agents.
tags:
  - plan
  - phase-6
  - a2a
status: draft
sources:
  - resource: git:340c189:plans/phase-6.md
    title: Original plan (removed from repo; full text in git history)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T23:50:57Z"
---

# Phase 6 — A2A edge

Goal: foreign agents (and other tower deployments) can discover this
deployment via its Agent Card, delegate work to named agents, and stream it to
completion — without touching internal routes.

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): external A2A client delegates a task to a named
agent and streams it to completion.

Depends on: phase 2 (tasks/messages are already A2A-shaped by design, [D§5](../concepts/design/05-core-objects.md)).
Recommended after phase 5 (external exposure after multi-machine is stable),
but only hard-depends on phase 2.

Reference: [D§7](../concepts/design/07-server-api.md) (A2A section), [D§5](../concepts/design/05-core-objects.md) (state mappings), research/a2a.md.

---

## Milestone 1 — Agent Card + inbound delegation ([D§7](../concepts/design/07-server-api.md))

- **T1.1** `GET /.well-known/agent-card.json`: card built from live agent
  roster (one skill per named agent with kind + description), capabilities
  `{streaming: true}` (pushNotifications: false v1), auth requirement
  (bearer, [D§13](../concepts/design/13-security.md)), `external_url` from config ([D§4](../concepts/design/04-paths-and-configuration.md)). Verify: golden-file test
  with fixture roster; invalid `external_url` config fails doctor.
- **T1.2** `POST /a2a` JSON-RPC 2.0: `message/send` — inbound message →
  `prompt` message + task (origin `a2a`, `external_ref` = A2A task id,
  `context_id` preserved, [D§6](../concepts/design/06-data-model.md)); routed to the named agent (skill → agent
  mapping from T1.1). Response carries task state per A2A shapes.
  Verify: integration tests with the official a2a-python or a2a-js client
  (reference clients prove interop, not just self-tests); error paths
  (unknown skill → A2A-compliant error).
- **T1.3** `message/stream`: SSE with `TaskStatusUpdateEvent` and
  `TaskArtifactUpdateEvent` streams mapped 1:1 from internal events
  ([D§5.2](../concepts/design/05-core-objects.md) mapping table: working↔working, blocked↔input-required, terminal
  states direct); `SubscribeToTask` resubscription after drops (internal
  cursor makes this natural, [D§7](../concepts/design/07-server-api.md)). Stream closes on terminal state.
  Verify: scripted FakeHarness task streams through a real A2A client;
  drop-and-resubscribe mid-stream replays correctly.

## Milestone 2 — Outbound + artifacts ([D§7](../concepts/design/07-server-api.md))

- **T2.1** Outbound replies: agent responses to `a2a`-origin tasks post back
  per A2A message shapes (role, messageId, parts). Task completion →
  artifact delivery: results written as artifacts ([D§6](../concepts/design/06-data-model.md) artifacts table)
  stream as chunked `TaskArtifactUpdateEvent` (`append`/`lastChunk`).
  Verify: chunk reassembly test on the client side; large-result fixture
  (> 1 chunk).
- **T2.2** External message direction on the internal bus: `from_kind/
  to_kind = external` (types exist since phase 1, [D§5.3](../concepts/design/05-core-objects.md)) wired through the
  messaging module so foreign-agent correspondence shows in inbox/history
  with clear provenance. Verify: provenance visible in CLI `task show`
  trail.

## Milestone 3 — Interop + hardening ([D§7](../concepts/design/07-server-api.md), [D§13](../concepts/design/13-security.md))

- **T3.1** Two-deployment test: tower A ↔ tower B over A2A only; a task
  delegated from A lands in B's pool, B's agent claims it (A2A-origin tasks
  enter the shared pool like any other, [D§5.2.1](../concepts/design/05-core-objects.md) — no special casing),
  completes, streams back to A. Verify: recorded transcript; both sides'
  event logs show consistent origin/ref.
- **T3.2** Edge hardening: rate limit per token on `/a2a`; payload size
  caps; unauthenticated card reads are fine (card is public metadata),
  authenticated everything else; SSE connection cap per token. Verify:
  limit tests; review against [D§13](../concepts/design/13-security.md) checklist.

## Milestone 4 — Phase exit verification

- **T4.1** Reference-client run: a2a-python or a2a-js sample client
  delegates "run <task> on agent X" to the deployment and streams to
  completion, using only the card + protocol. Record transcript.
- **T4.2** Docs: docs/a2a.md — how to point any A2A client at tower;
  update [design §17](../concepts/design/17-open-questions.md) items closed; final [plans overview](overview.md) status sweep.

## Backlog (post-1.0 seeds)

- Push notifications (`pushNotifications` capability, webhook + JWT per
  research/a2a.md — deliberately deferred, [D§7](../concepts/design/07-server-api.md))
- Agent-card skills beyond the roster (capability advertising, task-type
  routing)
- Outbound A2A client mode (tower agents delegating to *foreign* A2A
  servers — the reverse direction)

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|
