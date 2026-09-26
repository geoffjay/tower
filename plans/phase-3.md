# Phase 3 — TUI

Goal: daily monitoring driven entirely from the TUI — no terminal hopping, no
curl. The TUI is a client like any other; it shows coordination state and
delegates terminal presentation to herdr.

Exit criteria (DESIGN.md §18): daily monitoring driven entirely from TUI.

Depends on: phase 2 (messaging + pool APIs to render). Independent of
phase 4; both consume the same SSE feeds.

Reference: D§11 (views, chrome), D§10 (verbs the TUI wraps where useful).

---

## Milestone 1 — TUI skeleton (D§2, D§11)

- **T1.1** `agentos-tui` crate: ratatui + crossterm app loop, view router,
  command palette (`:`), vim navigation, global keymap (q quit, tab/shift-tab
  cycle views, `?` help), graceful shutdown on terminal resize/hangup.
  Client library reuse: talk to `/v1` with the same `agentos-client`
  transport (socket preferred). Verify: snapshot UI tests (ratatui test
  backend) for the empty-state frame of each view.
- **T1.2** Live data: SSE `/v1/events` subscription with cursor resume
  (reuse client transport); event bus → per-view state reducers. Verify:
  recorded event fixtures drive state changes in reducer unit tests.

## Milestone 2 — Fleet + agent detail (D§11)

- **T2.1** Fleet view: agents table — name, machine, kind, state glyph
  (`●` working, `○` idle, `◉` blocked, `✓` done, `✗` dead), current task,
  pool banner (`N queued · M working · K blocked`) above the table.
  Sort/filter (state, machine). Verify: snapshot tests for mixed-state
  fixtures; filter keymap behavior.
- **T2.2** Agent detail view: live output tail from `/v1/agents/{id}/stream`,
  message history, prompt input (`i` focuses), interrupt key, `o` shells out
  to `herdr` attach (D§11 division of labor). Verify: output-tail rendering
  with ANSI passthrough; prompt input posts to `/v1/agents/{id}/prompt`.

## Milestone 3 — Inbox + tasks (D§11)

- **T3.1** Inbox view: pending questions/approvals with deadlines; inline
  reply (`y`/`n`/text respond via `/v1/messages/{id}/respond`). Verify:
  integration against a scripted FakeHarness server; approve flow tested.
- **T3.2** Tasks view: pool/owned split, states, lease countdowns on owned
  tasks (live seconds from `lease_expires_at`), claim trail in detail.
  Verify: countdown rendering under clock-injected fixtures; trail rendering.

## Milestone 4 — Events + machines views (D§11)

- **T4.1** Events view: filtered feed of `/v1/events` (filter by type,
  subject; follow mode). Verify: filter application in reducer tests; live
  follow with fixture stream.
- **T4.2** Machines view: inventory + node status (phase 1-4: only `local`;
  designed so phase 5's node rows render without changes). Verify: renders
  `local` + a fixture offline node row.

## Milestone 5 — Phase exit verification

- **T5.1** Dogfood: run a full work session (spawn 2 pi agents, queue 3
  tasks, answer a question, approve an action, complete a task) entirely
  from TUI + one herdr attach. Record friction notes in this file; fix the
  top issues before closing.
- **T5.2** Soak: TUI left running 8h against live agents (overnight ok);
  no memory growth (heaptrack or process RSS log), SSE reconnects survive
  server restart mid-session.

## Backlog (phase 4+ seeds)

- Multi-select actions on fleet rows (broadcast prompt)
- Saved filter sets / named views
- TUI-side command history

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|