---
type: Plan
title: Phase 2 — Messaging + job queue
description: Durable message store, blocked-agent inbox, and an assignment-only job queue with leases.
tags:
  - plan
  - phase-2
  - messaging
  - job-queue
  - mcp
status: draft
sources:
  - resource: git:340c189:plans/phase-2.md
    title: Original plan (removed from repo; full text in git history)
generated:
  by: omp/claude-opus-5.5
  at: "2026-09-26T23:50:57Z"
---

# Phase 2 — Messaging + job queue

Goal: agents and humans communicate through one durable message store; blocked
agents surface in an inbox; work lives in a job queue where agents receive
assigned work, own it exclusively, report status, and crashed owners' work
requeues itself. Dispatch is assignment-only — no agent claims or pulls from
the queue ([job-queue decision](../decisions/job-queue.md)).

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): agent blocks on a question; it appears in
inbox; answered via CLI or MCP; agent resumes; expiry path tested. A queued
job is assigned to an agent; agent declares start, heartbeats, completes; a
second assignment conflicts (race tested); a killed owner's job requeues
within one lease window and another agent completes it after reassignment.

Depends on: phase 1 (server shell, driver, inventory, CLI skeleton).

---

## Milestone 1 — Messages ([D§5.3](../concepts/design/05-core-objects.md), [D§9.3](../concepts/design/09-server-modules.md))

- **T1.1** Message kinds, parts (A2A Part shapes: text/data/raw/url,
  media_type, filename, metadata), statuses (`pending | delivered | answered |
  expired | failed`), deadlines. Types in `tower-core` with serde tests;
  `messages` DDL is already live from phase 1 T2.2. Verify: round-trip + a
  compile-fail test on invalid kind.
- **T1.2** Messaging module + routes ([D§7](../concepts/design/07-server-api.md)): `POST /v1/messages` (any
  direction: human/agent/service/external → human/agent/room/external),
  `POST /v1/messages/{id}/respond`, `GET /v1/messages?to=&status=&since=`.
  Delivery semantics per [D§9.3](../concepts/design/09-server-modules.md): to-human = inbox row + event (UIs listen);
  to-agent = driver `prompt` on next settle. Verify: integration tests with
  FakeHarness for both delivery directions.
- **T1.3** CLI: `tower inbox`, `tower ask <name>`, `tower approve
  <msg-id> [--deny]`, `tower send`. Verify: inbox shape `--json` tests;
  approve path integration-tested against a mock blocked agent.

## Milestone 2 — Questions, approvals, blocked flow ([D§5.3](../concepts/design/05-core-objects.md), [D§9.3](../concepts/design/09-server-modules.md))

- **T2.1** Blocked-state → question flow: driver pump sees `blocked` →
  auto-creates a `question` message to the operator (human) with recent pane
  text as context part + the agent's own question if readable from
  detection. Dedup: one open question per blocked episode (keyed on agent +
  state episode, not per poll). Verify: FakeHarness scripts a blocked
  transition; assert exactly one inbox item + event.
- **T2.2** Answer flow: `respond` on a question → prompt delivered to agent
  → message `answered`, agent unblocks. Approval messages work the same but
  respond with send-keys option (`1`/`2`) per [D§8.2](../concepts/design/08-harness-layer.md). Verify: end-to-end
  integration test; verify dedup prevents duplicate prompts to the agent.
- **T2.3** Deadline sweeper ([D§9.3](../concepts/design/09-server-modules.md)): `pending` questions/approvals expire at
  `deadline_at` (default 5 min, `deadline_s` override on send); expiry →
  message `expired` + agent notified via prompt ("proceed with defaults or
  stop") + `approval.expired` event. Verify: clock-injected unit tests at
  boundary; sweep integration test.

## Milestone 3 — Job queue ([D§5.2.1](../concepts/design/05-core-objects.md), [D§9.4](../concepts/design/09-server-modules.md), [job-queue decision](../decisions/job-queue.md))

- **T3.1** Queue types + CAS: assign/start/heartbeat/release/status SQL with
  `WHERE ... AND (owner IS NULL OR lease_expires_at < now)` atomicity via the
  single-writer connection ([D§5.2.1](../concepts/design/05-core-objects.md)); `assigned` state + `start`
  transition (owner declares work); `attempt_count`/`max_attempts`
  bump-on-requeue; priority + tags for queue ranking. Verify: this is the
  heart — stress test: N=32 concurrent assigns on same task id → exactly one
  winner, 31 get `conflict`; queue listing respects `priority DESC,
  created_at ASC` and tag filters; no claim/pull endpoint exists (their
  absence is the contract).
- **T3.2** Lease sweeper (10s tick, [D§9.4](../concepts/design/09-server-modules.md)): expiry → requeue + `task.leased_out`
  event; `input-required` pause ([D§5.2](../concepts/design/05-core-objects.md) note); `attempt_count >= max_attempts`
  → `failed` with `lease_exhausted` result. Verify: clock-injected tests for
  all three branches including the pause.
- **T3.3** Routes ([D§7](../concepts/design/07-server-api.md)): `POST /v1/tasks` (+`assign`, pre-assign), `/assign`,
  `/start`, `/heartbeat`, `/status`, `/release`, `GET /v1/tasks?state=&tags=&owner=&mine=`,
  `GET /v1/tasks/{id}` with assignment/lease trail. All task mutations emit
  `task.*` events (incl. `task.assigned`). Verify: route integration tests;
  owner-only start/status/release writes (non-owner → `conflict`);
  assign-conflict on live owner.
- **T3.4** CLI: `task list/create/assign/cancel/release/show` per [D§10](../concepts/design/10-client-cli.md). Verify:
  `--json` shape tests; `show` includes trail; `assign` conflicts visibly
  (exit non-zero, `conflict` error).

## Milestone 4 — MCP endpoint ([D§7](../concepts/design/07-server-api.md) MCP)

- **T4.1** Streamable-HTTP MCP server at `/mcp` (rmcp or hand-rolled per
  [D§2](../concepts/design/02-stack.md)). Tools: `tower_ps`, `tower_spawn`, `tower_prompt`, `tower_send`,
  `tower_ask`, `tower_approve`, `tower_task_list/show/create/assign/start/
  heartbeat/status/release`, `tower_machine_list` — thin wrappers over
  the same service calls as REST (one code path, [D§7](../concepts/design/07-server-api.md)). Verify: MCP client
  integration test drives a full work-loop against FakeHarness: agent is
  assigned a job via `tower_task_assign` (as operator), sees it with
  `tower_task_list --mine`, declares start, heartbeats, completes.
- **T4.2** Agent work-loop skill doc (docs/agent-loop.md): check my assigned
  jobs → declare start → heartbeat cadence (20s) → report status →
  complete; how to ask/approve through MCP; explicitly: never pull or
  claim — wait for assignment. This is the contract agents' prompts
  reference. Verify: a scripted FakeHarness "agent" follows the doc's loop
  in a test.

## Milestone 5 — Phase exit verification

- **T5.1** Blocked→inbox→answered e2e with real herdr + pi (agent blocks on
  a question), answered via `tower approve`; replay via MCP tool.
- **T5.2** Queue exclusivity e2e: two real pi agents, one queued job;
  operator assigns to agent A (success), then to agent B → clean `conflict`.
  A declares start, heartbeats, completes. Kill A mid-task
  (`kill -9` the pane process), job requeues within lease, operator
  reassigns, survivor B completes it.
- **T5.3** Record all runs in Verification log; update [design open
  questions](../concepts/design/17-open-questions.md) resolved by this phase (none blocking — S1 spikes resolved in
  phase 1; the orchestrator/router question [#17.8](../concepts/design/17-open-questions.md) stays open by design).

## Backlog (phase 3+ seeds)

- Rooms (`to_kind=room`) — types exist, no route sugar yet
- External/A2A direction on messages (phase 6)
- `agent.output` artifact chunking beyond the phase-1 append
- Orchestrator role + agent router (jev/laya decision model; [design §17.8](../concepts/design/17-open-questions.md),
  [job-queue decision](../decisions/job-queue.md)) — automated dispatch after the queue primitive
  is proven

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|
