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
- **Agent detail**: live output tail (`/v1/agents/{id}/stream`), prompt input
  line, message history; `o` opens the actual pane in herdr (shell out to
  `herdr` client attach)
- **Inbox**: pending questions/approvals; reply inline (`y`/`n`/text)
- **Tasks**: task list + state (queue/owned split, lease countdowns on owned
  tasks, reserved-for target on waiting jobs); schedules with next run;
  detail shows trail + assignment history
- **Events**: filtered feed of `/v1/events`
- **Machines**: inventory + node status

Chrome: command palette (`:`), vim-style navigation, state filter/sort.
Keybinding: `i` focuses prompt input on the agent detail view. A queue banner
(`N queued · M working · K blocked`) sits above the fleet table.
