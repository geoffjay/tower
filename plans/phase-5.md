# Phase 5 — Multi-machine

Goal: agents on a second machine appear beside local ones; the coordinator
stays single (one database); tasks are claimed across machines; node loss
requeues work and coordinator loss idles nodes without killing agents.

Exit criteria (DESIGN.md §18): agent runs on second machine, appears in
local ps/TUI; node disconnect handles gracefully; a task queued on the
coordinator is claimed by an agent on the remote machine; node loss mid-task
requeues the lease.

Depends on: phase 2 (task pool must be coordinator-owned, D§5.2.1 rationale).

Reference: D§9.5 (machine hub, node protocol), D§13 (node security), D§14
(node/coordinator failure modes), research/cross-server.md.

---

## Milestone 0 — Decision spike (D§17.4)

- **S5.A** Node transport security: (a) plain WS + per-machine token over
  SSH tunnel (operator runs `ssh -L` or WireGuard), (b) TLS + token with
  self-signed/pinned certs shipped by `tower machines add`. Constraints:
  no inbound holes on nodes (nodes dial out, D§9.5); keep zero-config local
  case unaffected. Recommend (a) for v1 (herdr's machines model is SSH;
  the factory notes assume SSH between hosts), record decision + a migration
  note for adding TLS later. Output: findings section + DESIGN.md §17.4
  update.

## Milestone 1 — Machine registry + hub (D§9.5)

- **T1.1** Machine registry: `machines` rows already exist (phase 1);
  add lifecycle routes + CLI: `tower machines add <name>` (issues
  per-machine token, prints once, D§13), `machines remove`, `machines list`.
  Node token storage: hashed in db, raw shown once. Verify: integration
  tests for issue/remove; token-perms assertions.
- **T1.2** Hub endpoint `/nodes`: WS upgrade, per-machine token auth,
  frame protocol (length-prefixed JSON) with three message classes:
  downstream RPC (driver calls), upstream events, snapshot/resync
  (D§9.5). Backpressure: bounded channel per node; slow node disconnects
  and resyncs. Verify: loopback integration test with two in-process
  nodes; reconnect + resync sequence tested; token rejection tested.

## Milestone 2 — Node agent (D§3, D§9.5)

- **T2.1** `tower node` command: loads `[node]` config (coordinator URL,
  token), runs a local HerdrDriver, registers on connect (machine name,
  capabilities), answers RPCs (spawn/prompt/read/stop against local
  herdr), forwards driver events upstream. Reconnect with exponential
  backoff; resync on reconnect. Verify: against a hub fixture; kill
  coordinator mid-stream → node idles, agents keep running (herdr owns
  them), reconnects when it returns (D§14).
- **T2.2** Coordinator-side driver fan-out: sessions/inventory modules route
  driver calls to the right machine — local driver or hub → node RPC.
  One `Harness` impl that proxies (node driver = same trait, remote
  transport). Machine tagging on all forwarded events. Verify:
  integration test spawning an agent "on" a fixture node via `POST
  /v1/agents {machine: ...}`; events arrive tagged.

## Milestone 3 — Cross-machine pool (D§5.2.1)

- **T3.1** Pool already coordinator-owned; nothing structural changes.
  Add: machine awareness in `ps`/TUI (already in schemas), and lease
  behavior under node loss — node disconnect marks its agents unreachable;
  sweeper requeues their owned tasks after lease expiry (this falls out of
  T1/T2 + the phase-2 sweeper, but must be verified end-to-end, not
  assumed). Verify: scripted test — remote owner claims task, node killed,
  task requeues within lease + `task.leased_out` event, local agent picks
  it up and completes.
- **T3.2** `machines` view surfaces in TUI (phase 3 built the view with
  fixture nodes; wire real node rows) and web UI machine strip gets live
  last_seen/status. Verify: TUI snapshot + UI render with one online and
  one offline node.

## Milestone 4 — Phase exit verification

- **T4.1** Two-real-machine run: coordinator on host A, node on host B
  (or second local user/namespace if hardware-poor): remote pi agent
  spawned, appears in `tower ps`, streams output locally over SSE,
  claims a queued task, completes it. Record transcript.
- **T4.2** Chaos pass: kill node mid-task (requeue path), kill coordinator
  mid-stream (nodes idle + reconnect), kill both (agents survive via
  herdr, full resync on restart). Record all three.

## Backlog (phase 6+ seeds)

- Node-to-node traffic (none in v1 — coordinator relays everything)
- Herdr `machine` passthrough (herdr's own multi-machine views alongside)
- Metrics endpoint (D§15 optional Prometheus)

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|

## Spike findings

### S5.A — node transport security (filled during execution)