---
type: Plan
title: Phase 3 — TUI
description: Daily monitoring driven entirely from the TUI client.
tags:
  - plan
  - phase-3
  - tui
status: draft
sources:
  - resource: git:340c189:plans/phase-3.md
    title: Original plan (removed from repo; full text in git history)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T23:50:57Z"
---

# Phase 3 — TUI

Goal: daily monitoring driven entirely from the TUI — no terminal hopping, no
curl. The TUI is a client like any other; it shows coordination state and
delegates terminal presentation to herdr.

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): daily monitoring driven entirely from TUI.

Depends on: phase 2 (messaging + queue APIs to render). Independent of
phase 4; both consume the same SSE feeds.

Reference: [D§11](../concepts/design/11-tui.md) (views, chrome), [D§10](../concepts/design/10-client-cli.md) (verbs the TUI wraps where useful).

---

## Milestone 1 — TUI skeleton ([D§2](../concepts/design/02-stack.md), [D§11](../concepts/design/11-tui.md))

- **T1.1** `tower-tui` crate: ratatui + crossterm app loop, view router,
  command palette (`:`), vim navigation, global keymap (q quit, tab/shift-tab
  cycle views, `?` help), graceful shutdown on terminal resize/hangup.
  Client library reuse: talk to `/v1` with the same `tower-client`
  transport (socket preferred). Verify: snapshot UI tests (ratatui test
  backend) for the empty-state frame of each view.
- **T1.2** Live data: SSE `/v1/events` subscription with cursor resume
  (reuse client transport); event bus → per-view state reducers. Verify:
  recorded event fixtures drive state changes in reducer unit tests.

## Milestone 2 — Fleet + agent detail ([D§11](../concepts/design/11-tui.md))

- **T2.1** Fleet view: agents table — name, machine, kind, state glyph
  (`●` working, `○` idle, `◉` blocked, `✓` done, `✗` dead), current task,
  queue banner (`N queued · M working · K blocked`) above the table.
  Sort/filter (state, machine). Verify: snapshot tests for mixed-state
  fixtures; filter keymap behavior.
- **T2.2** Agent detail view: live output tail (re-read
  `/v1/agents/{id}/read?format=ansi` on each `agent.output` for the agent —
  amendment, [D§11.1](../concepts/design/11-tui.md): output payloads can be
  whole-buffer rewrites, so appending them duplicates), message history
  (`GET /v1/messages?agent=`, new filter), prompt input (`i` focuses),
  interrupt key, `o` shells out to `herdr agent attach <pane>`
  ([D§11](../concepts/design/11-tui.md) division of labor). Verify:
  output-tail rendering with ANSI passthrough; prompt input posts to
  `/v1/agents/{id}/prompt`.

## Milestone 3 — Inbox + tasks ([D§11](../concepts/design/11-tui.md))

- **T3.1** Inbox view: pending questions/approvals with deadlines; inline
  reply (`y`/`n`/text respond via `/v1/messages/{id}/respond`). Verify:
  integration against a scripted FakeHarness server; approve flow tested.
- **T3.2** Tasks view: queue/owned split, states (`assigned` distinct from
  `working`), lease countdowns on owned tasks (live seconds from
  `lease_expires_at`), assignment trail in detail. Verify: countdown
  rendering under clock-injected fixtures; trail rendering.

## Milestone 4 — Events + machines views ([D§11](../concepts/design/11-tui.md))

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

### T5.1 friction notes (2026-09-27)

Live session against real herdr 0.8.2, entirely from `tower tui` (tmux,
150×40, outside herdr): `:spawn` two omp agents (stand-in for pi: pi's
provider auth still fails with `401` in any shell, independent of tower —
the phase 2 finding), prompt both with `i`, queue 3 jobs with `n`, assign
two with `a` and reserve the third (`d1 later`), watch the dispatcher
deliver it when d1 freed up and all three complete with results; answer
d2's question (asked via MCP `tower_ask`) with `r`; approve claude c1's
folder-trust dialog with `y` (→ idle) and deny c2's with `n` (→ esc, dead);
`o` to answer omp's own ask dialog inside `herdr agent attach`, `ctrl-b q`
back to the TUI.

Fixed before closing (`8530591`):

1. Operator prompts never reached agent history — the prompt route drove
   the harness but skipped the `prompt` message row D§7 already specified.
2. Approval context was one line of escaped JSON; the inbox now shows the
   captured screen's tail, where the dialog sits.
3. A 60s lease read `1m01s` and toasts vanished right after an attach: the
   app clock only advanced on ticks (the attach blocks the loop). It now
   tracks the real clock per message.
4. Long input ran off the right edge; the input line scrolls with the cursor.
5. Closed jobs showed `lease —` in detail.

Not fixed here (backlog below): omp's ask dialog is not reported as
`blocked` by herdr detection, so it raises no inbox item; shell harnesses
have no CLI verb to ask the operator (d2 had to curl `/mcp`).

## Backlog (phase 4+ seeds)

- Multi-select actions on fleet rows (broadcast prompt)
- Saved filter sets / named views
- TUI-side command history
- Agent → operator questions from the CLI (`tower ask me …` under
  `$TOWER_AGENT`): pi/omp-style harnesses without MCP have no verb today
- herdr detection doesn't flag omp's ask dialog as `blocked` (no inbox
  item); raise upstream or map omp's dialog in the pump like claude's
- `o` inside herdr (`herdr agent focus`) is implemented but was not
  exercised live — it would have moved the operator's own herdr focus
  mid-session; the attach path was

## Verification log

| Date | Check | Result |
|---|---|---|
| 2026-09-27 | Prep: `GET /v1/messages?agent=` — the agent's history both directions, by name or id | pass (`tests/messaging.rs`) |
| 2026-09-27 | Found while building: `EventKind::as_str` leaked a `String` per call — the server's SSE filter runs it per event per subscriber, the TUI per rendered row | fixed `a07b985` (names pinned to serde by a test) |
| 2026-09-27 | Found: NULL agent columns decoded as `""` (spawn printed `worktree `); `tower-client`'s single 65s total timeout also cut SSE streams (`tower stream` died at 65s) | fixed `4d73601` (regression: null `worktree`/`workdir` in the spawn route test) |
| 2026-09-27 | T1.1 empty-state frame of every view (TestBackend + insta, pinned clock, UTC) | pass (`tests/views.rs`) |
| 2026-09-27 | T1.2 recorded live SSE session (herdr + omp `f1`) parsed in 97-byte chunks drives the reducers: fetches per event kind, output payloads reduced to their size, a replay after reconnect applies nothing, `agent.state` patched in place, screen reads only for the open agent and coalesced while in flight | pass (`tests/feed.rs`, 7 tests) |
| 2026-09-27 | T2.1 mixed-state fleet snapshot; banner `1 queued · 1 working · 2 blocked` (blocked = `input-required` + a job under a blocked owner); `f`/`m`/`s`/`c` keys; selection follows the agent across re-sorts | pass |
| 2026-09-27 | T2.2 ANSI passthrough (red cell styled, no escape bytes, trailing blank rows dropped, tail at the bottom), scroll/follow; `i` → prompt against a FakeHarness server over TCP reaches the harness and shows in history off the bus | pass (`tests/views.rs`, `tests/tui.rs`) |
| 2026-09-27 | T3.1 approve from the inbox against a scripted FakeHarness server on TCP: `enter` sent once (plain "Yes"), message answered, inbox empties via `message.status` | pass (`tests/tui.rs`) |
| 2026-09-27 | T3.2 clock-injected lease countdowns (42s → 12s → expired; `input-required` → held), reservations (`→backend when available`, `→api in 5m00s`), trail detail snapshot, `x x` cancel | pass |
| 2026-09-27 | T4.1 events filters (type substring, subject by agent name, output toggle), follow/pause; T4.2 `local` + offline node row | pass |
| 2026-09-27 | Feed resume across a server crash (HTTP task aborted, an event appended while down, same port back): the missed event is replayed via `?cursor=` exactly once | pass (`tests/tui.rs`) |
| 2026-09-27 | T5.1 dogfood (see friction notes): full session from the TUI + one herdr attach; the live server was restarted mid-session and the TUI went `reconnecting` → `● live` with the rebuilt server's data | pass; 5 friction fixes `8530591` |
