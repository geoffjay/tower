---
type: Plan
title: Agent output copy dialog (Herdr app)
description: A button on the Agents detail opens a dialog with a read-only, selectable snapshot of the agent's screen, so output can be copied. Plan A is a separate mod; plan B reuses tower-agents as the overlay mod.
tags:
  - plan
  - herdr-app
  - tower-agents
status: stable
---

# Agent output copy dialog

Problem: the Agents detail screen is a `v_virtual_list` of styled runs, so
it cannot select or copy text
([Herdr app package](../concepts/herdr-app.md)). The reason is recorded
there: an `EditorState` has no scroll API (so no follow-newest) and no
per-range colors.

Goal: leave the live screen as is and add a way to copy the output.

## What the Herdr app API gives

- `openOverlay({ mod, kind: "dialog", props, width, height })`
  (`herdr.d.ts`): a centered dialog, Esc closes, the mod gets
  `props().overlay` and may call `closeOverlay(id)`. `tower-tabs` and the
  shipped `confirm` mod already do this.
- `EditorState.new({ value, readonly: true, … })` + `Editor.new(state)`:
  "rejects user edits, but allows selection and copy". `tower-events`
  already uses it for event bodies. Release the state when done.
- The dialog shows a snapshot, so the editor's missing scroll API does not
  matter. No colors (acceptable for a copy view).
- Snapshot text: `this.lines.map(l => l.runs.map(r => r.t).join("")).join("\n")`,
  already ANSI-free.

## Plan A — a separate `tower-agent-output` mod

The original proposal, kept in case plan B does not work out.

1. New folder `app/mods/tower-agent-output/` with `main.js` and `mod.json`
   (`{ "place": "none" }`). No `tower.js` (it calls no program), so the
   "identical copy in every mod" rule does not apply.
2. `main.js` reads `props()` as `{ overlay, name, text }`, builds the
   readonly `EditorState` (`line_numbers: false`, `soft_wrap: false`,
   `font_family: settings().font_family`, `font_size: 12`), and renders a
   header (agent name, Close) over `Editor.new(state)`. It releases the
   state on close.
3. `tower-agents/main.js`: a `Copy…` button beside Follow in `details()`
   calls `openOverlay({ mod: "tower-agent-output", kind: "dialog", width: 0.7, height: 0.8, props: { name, text } })`.
4. Docs: `herdr-app.md` (drop "no text selection or copy"; six mods
   becomes seven), `app/RECIPE.md` (mod list, install and copy steps),
   `log.md`.

Trade-offs: monochrome; the snapshot is the 200-line `SCREEN_LINES`
window; props carry the whole text (size limit unchecked).

## Plan B — reuse `tower-agents` as the overlay mod

Prototype on branch `proto/agent-output-overlay`. Same dialog, no seventh
mod: `openOverlay({ mod: "tower-agents", props: { output: { name, text } } })`
and `init` branches on `props().output`. Concerns to check in the
prototype: the panel's poll timers must not start in the overlay copy,
the overlay copy must not disturb the panel's state, and two views in one
`main.js`. Record the outcome here and pick A or B.

## Outcome

Plan B works (verified in the app): the dialog opens from `Copy…`, text
selects and copies, Esc and Close dismiss it, and the panel behind keeps
running. Chosen over plan A (no seventh mod). Plan A stays above as the
fallback only.
