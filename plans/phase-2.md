# Phase 2 — Messaging + task pool

Goal: agents and humans communicate through one durable message store; blocked
agents surface in an inbox; agents pick up work from a shared task pool with
exclusive ownership, report status, and crashed owners' work requeues itself.

Exit criteria (DESIGN.md §18): agent blocks on a question; it appears in
inbox; answered via CLI or MCP; agent resumes; expiry path tested. Two agents
pulling the same pool: exactly one claims each task (race tested), a killed
owner's task requeues within one lease window, another agent picks it up and
completes it.

Depends on: phase 1 (server shell, driver, inventory, CLI skeleton).

---

## Milestone 1 — Messages (D§5.3, D§9.3)

- **T1.1** Message kinds, parts (A2A Part shapes: text/data/raw/url,
  media_type, filename, metadata), statuses (`pending | delivered | answered |
  expired | failed`), deadlines. Types in `tower-core` with serde tests;
  `messages` DDL is already live from phase 1 T2.2. Verify: round-trip + a
  compile-fail test on invalid kind.
- **T1.2** Messaging module + routes (D§7): `POST /v1/messages` (any
  direction: human/agent/service/external → human/agent/room/external),
  `POST /v1/messages/{id}/respond`, `GET /v1/messages?to=&status=&since=`.
  Delivery semantics per D§9.3: to-human = inbox row + event (UIs listen);
  to-agent = driver `prompt` on next settle. Verify: integration tests with
  FakeHarness for both delivery directions.
- **T1.3** CLI: `tower inbox`, `tower ask <name>`, `tower approve
  <msg-id> [--deny]`, `tower send`. Verify: inbox shape `--json` tests;
  approve path integration-tested against a mock blocked agent.

## Milestone 2 — Questions, approvals, blocked flow (D§5.3, D§9.3)

- **T2.1** Blocked-state → question flow: driver pump sees `blocked` →
  auto-creates a `question` message to the operator (human) with recent pane
  text as context part + the agent's own question if readable from
  detection. Dedup: one open question per blocked episode (keyed on agent +
  state episode, not per poll). Verify: FakeHarness scripts a blocked
  transition; assert exactly one inbox item + event.
- **T2.2** Answer flow: `respond` on a question → prompt delivered to agent
  → message `answered`, agent unblocks. Approval messages work the same but
  respond with send-keys option (`1`/`2`) per D§8.2. Verify: end-to-end
  integration test; verify dedup prevents duplicate prompts to the agent.
- **T2.3** Deadline sweeper (D§9.3): `pending` questions/approvals expire at
  `deadline_at` (default 5 min, `deadline_s` override on send); expiry →
  message `expired` + agent notified via prompt ("proceed with defaults or
  stop") + `approval.expired` event. Verify: clock-injected unit tests at
  boundary; sweep integration test.

## Milestone 3 — Task pool (D§5.2.1, D§9.4)

- **T3.1** Pool types + CAS: claim/pull/heartbeat/release/status SQL with
  `WHERE ... AND (owner IS NULL OR lease_expires_at < now)` atomicity via the
  single-writer connection (D§5.2.1); `attempt_count`/`max_attempts`
  bump-on-requeue; priority + tags matching for `pull`. Verify: this is the
  heart — stress test: N=32 concurrent claims on same task id → exactly one
  winner, 31 get `conflict`; pull respects `priority DESC, created_at ASC`
  and tag filters.
- **T3.2** Lease sweeper (10s tick, D§9.4): expiry → requeue + `task.leased_out`
  event; `input-required` pause (D§5.2 note); `attempt_count >= max_attempts`
  → `failed` with `lease_exhausted` result. Verify: clock-injected tests for
  all three branches including the pause.
- **T3.3** Routes (D§7): `POST /v1/tasks` (+`assign`, pre-claim), `/claim`,
  `/pull`, `/heartbeat`, `/status`, `/release`, `GET /v1/tasks?state=&tags=&owner=`,
  `GET /v1/tasks/{id}` with claim/lease trail. All task mutations emit
  `task.*` events. Verify: route integration tests; owner-only terminal
  writes (non-owner → `conflict`).
- **T3.4** CLI: `task list/create/cancel/release/show` per D§10. Verify:
  `--json` shape tests; `show` includes trail.

## Milestone 4 — MCP endpoint (D§7 MCP)

- **T4.1** Streamable-HTTP MCP server at `/mcp` (rmcp or hand-rolled per
  D§2). Tools: `tower_ps`, `tower_spawn`, `tower_prompt`, `tower_send`,
  `tower_ask`, `tower_approve`, `tower_task_list/show/create/claim/
  pull/heartbeat/status/release`, `tower_machine_list` — thin wrappers over
  the same service calls as REST (one code path, D§7). Verify: MCP client
  integration test drives a full pool work-loop against FakeHarness.
- **T4.2** Agent work-loop skill doc (docs/agent-loop.md): pull → heartbeat
  cadence (20s) → status → complete; how to ask/approve through MCP. This is
  the contract agents' prompts reference. Verify: a scripted FakeHarness
  "agent" follows the doc's loop in a test.

## Milestone 5 — Phase exit verification

- **T5.1** Blocked→inbox→answered e2e with real herdr + pi (agent blocks on
  a question), answered via `tower approve`; replay via MCP tool.
- **T5.2** Pool race e2e: two real pi agents, one queued task, both pulling;
  winner completes, loser gets clean conflict. Kill winner mid-task
  (`kill -9` the pane process), task requeues within lease, survivor
  completes it.
- **T5.3** Record all runs in Verification log; update DESIGN.md open
  questions resolved by this phase (none blocking — S1 spikes resolved in
  phase 1).

## Backlog (phase 3+ seeds)

- Rooms (`to_kind=room`) — types exist, no route sugar yet
- External/A2A direction on messages (phase 6)
- `agent.output` artifact chunking beyond the phase-1 append

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|