---
type: Concept
title: Design §11 — TUI
description: ratatui client views and keybindings; tower shows coordination state, herdr shows terminals.
tags:
  - design
  - design-s11
  - tui
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §11 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 11. TUI

ratatui client of `/v1` + local herdr shell-outs. Division of labor
(openrig's split): tower TUI shows **coordination state**; herdr shows
**terminals**.

Views:

- **Fleet**: agents table — name, machine, kind, state glyph
  (`●` working, `○` idle, `◉` blocked, `✓` done, `✗` dead), current task,
  context note
- **Agent detail**: live output tail, prompt input line, message history;
  `o` opens the actual pane in herdr: `herdr agent focus <pane>` when the
  TUI itself runs inside herdr (`HERDR_PANE_ID` set; the TUI keeps its
  pane), else `herdr agent attach <pane>` with the terminal handed over
  until herdr's detach key returns it
- **Inbox**: pending questions/approvals; reply inline (`y`/`n`/text)
- **Tasks**: task list + state (queue/owned split, lease countdowns on owned
  tasks, reserved-for target on waiting jobs); schedules with next run;
  detail shows trail + assignment history
- **Events**: filtered feed of `/v1/events`
- **Machines**: inventory + node status

Chrome: command palette (`:`), vim-style navigation, state filter/sort.
Keybinding: `i` focuses prompt input on the agent detail view. A queue banner
(`N queued · M working · K blocked`) sits above the fleet table: open jobs
by state — `queued` (incl. reserved), `assigned`/`working`, and blocked =
`input-required` or owned by a `blocked` agent (the jobs waiting on you).

## 11.1 Data flow (phase 3 amendment)

One SSE subscription to `/v1/events` feeds everything; the TUI never polls
except a 10s `GET /v1/tasks?since=` sweep (heartbeats renew leases without
an event, and lease countdowns must stay true).

- Startup: open the SSE stream first (no cursor → live head), then load
  agents, tasks, schedules, inbox, machines. Events racing the snapshot
  only trigger idempotent refetches.
- Events are reduced per view: `agent.state` patches the row from its
  payload; other kinds name the object to refetch (task by id, inbox,
  agents, schedules, machines).
- Reconnect: resume with the last seen `id` as cursor (the log replays the
  gap), then resync the snapshot anyway.
- Output tail: `agent.output` payloads are deltas *or* whole-buffer rewrites
  (the driver re-emits the buffer when the screen reshapes), so appending
  them is wrong. The detail view re-reads `/v1/agents/{id}/read?format=ansi`
  (ANSI passthrough) whenever that agent's `agent.output` arrives —
  exact screen, no reassembly. `GET /v1/agents/{id}/stream` ([D§7](07-server-api.md))
  is not needed by the TUI.
- Message history: `GET /v1/messages?agent=<name>` — rows sent or addressed
  to the agent (by name or id).
  Operator prompts are rows too (`POST /v1/agents/{id}/prompt` records a
  `prompt` message, D§7).
- Bounded memory for long sessions: the events view keeps the last 2000
  events (output payloads reduced to their size), the store keeps every
  open job but only the latest 100 closed ones.
