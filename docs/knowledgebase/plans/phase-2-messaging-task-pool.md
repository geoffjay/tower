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
status: stable
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
  answer the on-screen dialog (deny = `esc`, approve = first plain "Yes") per [D§8.2](../concepts/design/08-harness-layer.md). Verify: end-to-end
  integration test; verify dedup prevents duplicate prompts to the agent.
- **T2.3** Deadline sweeper ([D§9.3](../concepts/design/09-server-modules.md)): `pending` questions/approvals expire at
  `deadline_at` (default 5 min, `deadline_s` override on send); expiry →
  message `expired` + agent notified (question: prompt "proceed with
  defaults or stop"; approval: denied via `esc`) + `approval.expired`
  event. Verify: clock-injected unit tests at boundary; sweep integration test.

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
  **Decision (2026-09-27)**: hand-rolled — JSON-RPC over `POST /mcp` with
  `application/json` replies, tools only (~450 lines, no deps). rmcp 3.x
  is a large macro/schemars dependency with frequent breaking releases;
  a tools-only server needs none of its transports. Identity: `as` arg,
  default `X-Tower-Agent` header; agent-identified callers can't assign
  ([D§7](../concepts/design/07-server-api.md) amended). Validated with the official
  `@modelcontextprotocol/inspector` CLI (tools/list, tools/call, isError).
- **T4.2** Agent work-loop skill doc (docs/agent-loop.md): check my assigned
  jobs → declare start → heartbeat cadence (20s) → report status →
  complete; how to ask/approve through MCP; explicitly: never pull or
  claim — wait for assignment. This is the contract agents' prompts
  reference. Verify: a scripted FakeHarness "agent" follows the doc's loop
  in a test. **Finding**: pi has no MCP (extensions only) but has a shell,
  so the loop is also CLI verbs (`tower task start|heartbeat|status|release`,
  [D§10](../concepts/design/10-client-cli.md) amended); spawn exports `TOWER_AGENT` into the pane (herdr
  `--env`, verified live) so `--as` defaults correctly.

## Milestone 5 — Phase exit verification

- **T5.1** Blocked→inbox→answered e2e with real herdr + pi (agent blocks on
  a question), answered via `tower approve`; replay via MCP tool.
  **Ran with claude** (2026-09-27): pi's provider auth fails in every shell
  (`401`), independent of tower; operator chose claude. claude's block is a
  permission dialog, so this exercised the approval path.
- **T5.2** Queue exclusivity e2e: two real pi agents, one queued job;
  operator assigns to agent A (success), then to agent B → clean `conflict`.
  A declares start, heartbeats, completes. Kill A mid-task
  (`kill -9` the pane process), job requeues within lease, operator
  reassigns, survivor B completes it. **Ran with omp** (2026-09-27): claude
  can't authenticate from a fresh shell on this machine (the operator's
  shell rc forces `CLAUDE_CODE_USE_FOUNDRY` without a Foundry URL); omp
  can, and runs the loop through the tower CLI like pi would.
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

| Date | Check | Result |
|---|---|---|
| 2026-09-27 | M1: messaging routes vs FakeHarness — to-human inbox row (pending + deadline), to-agent prompt, respond CAS (double respond → 409, no re-delivery), explicit agent recipient must exist | pass (`tests/messaging.rs`) |
| 2026-09-27 | M1 live CLI: `send`/`inbox`/`approve --answer` round trip; `ask` to unknown agent → `not_found`, exit 1 | pass |
| 2026-09-27 | M2: pump — blocked → exactly one inbox item per episode; claude → `approval`; approval respond answers the dialog once; approval without decision → 400, stays pending | pass (`tests/pump.rs`, `tests/messaging.rs`) |
| 2026-09-27 | M2: deadline sweeper, clock-injected — expires at `deadline_at` (inclusive), question → "proceed" prompt, approval → denied, respond after expiry → 409 | pass (`tests/sweeper.rs`) |
| 2026-09-27 | M3: 32 concurrent assigns on one job → exactly 1 winner, 31 `conflict`, one delegation prompt; 20/20 repeat runs; mutating the CAS guard away makes it fail | pass (`tests/tasks.rs`) |
| 2026-09-27 | M3: owner-only writes, release (no attempt bump), late-but-unswept completion, sweeper requeue / `lease_exhausted` / `input-required` pause, routes + trail | pass (`tests/tasks.rs`, 19 tests) |
| 2026-09-27 | M3 live, real herdr 0.8.2 + pi: spawn into `tower-agents` workspace; create → assign → second assign `conflict` (exit 1) → `--mine`; delegation text landed in the pi pane; unstarted lease lapsed and was swept within one 10s tick → `queued`, attempt 1/3; reassign → release → cancel trail | pass |
| 2026-09-27 | Live runs surfaced 3 phase-1 driver bugs: unnamed herdr agents failed the whole snapshot (reconcile + pump dead); error envelopes on non-zero exit never mapped; spawn split the operator's first pane and ignored `workdir` | fixed `15181b6` (regression test: `snapshot_skips_unnamed_agents`) |
| 2026-09-27 | M4: MCP protocol (version negotiation, notifications 202, JSON-RPC errors), work loop over MCP, agents can't assign, `/v1/schema` lists only mounted routes (phantom-route mutation caught) | pass (`tests/mcp.rs`) |
| 2026-09-27 | M4 live: official `@modelcontextprotocol/inspector` CLI — `tools/list`, `tools/call` (create, start via `X-Tower-Agent`), agent assign → `isError` `unauthorized`; herdr `--env` propagation probe (workspace + tab); CLI loop with `$TOWER_AGENT` (impostor → `not_found`, no identity → usage error) | pass |
| 2026-09-27 | `stop --remove` on an agent that had owned a job → FK error after the pane was closed | fixed `65f223d` (regression test fails before, passes after) |
| 2026-09-27 | T5.1 prep: pi's provider auth fails in any shell (`401`), independent of tower → claude for T5.1 (operator's choice) | env finding |
| 2026-09-27 | T5.1 live exposed: claude blocked on its folder-trust dialog failed `spawn` (`agent_not_ready`) and closed the pane; a block first seen by reconcile/`AgentUp` never reached the inbox | fixed `a0b5538` (regression: `blocked_first_seen_by_reconcile_then_pump_opens_one_item`) |
| 2026-09-27 | **T5.1 live exposed a safety bug**: approvals sent fixed keys `1`/`2`. The trust dialog lists "No, exit" first; claude's tool prompt has option 2 = "Yes, and don't ask again", so *deny* (and the sweeper's expired-approval deny) would have granted a permanent permission. Caught before any real permission prompt was answered | fixed `a0b5538`: deny = `esc`, approve = navigate to the first plain "Yes" or fail (`dialog.rs`, captured-screen fixtures) |
| 2026-09-27 | **T5.1** real herdr + claude: c1 blocked (trust dialog) → approval in inbox within 4s → `tower approve` → `down`,`enter` selected "Yes, I trust this folder" → idle; c2 (second untrusted dir) → approval answered via the MCP `tower_approve` tool (official inspector CLI) → idle; inbox empty | pass |
| 2026-09-27 | T5.2 prep: claude fails from a fresh shell (operator rc forces `CLAUDE_CODE_USE_FOUNDRY` with no Foundry URL; verified in a plain `zsh -i`, no tower) → omp (operator's choice) | env finding |
| 2026-09-27 | **T5.2** real herdr + 2 omp agents: J1 assigned to o1, second assign to o2 → `conflict` naming o1 (exit 1); o1 ran start → heartbeat → `completed` via the tower CLI unprompted (12s). J2 (lease 40s): o1 started 22:19:55, `kill -9` 22:20:04 → o1 `dead` within 4s; lease swept 22:20:36 → `queued` 1/3; assign to dead o1 refused; reassigned to o2 22:20:37 → o2 heartbeated mid-job and completed 22:21:35 (47s run on a 40s lease) | pass |
| 2026-09-27 | T5.2 cleanup exposed: `stop --remove` on a crashed agent left its shell pane (lookup by name; herdr no longer lists it) | fixed: `Harness::stop` falls back to the row's `pane_id`; re-run live → no pane left |
