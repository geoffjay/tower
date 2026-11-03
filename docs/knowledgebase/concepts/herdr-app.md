---
type: Concept
title: Herdr app package
description: The app/ layout and mods that put tower's agents, inbox, jobs and event log in the Herdr desktop app; how each mod reaches tower and the rules the events panel follows.
tags:
  - herdr
  - herdr-app
  - mods
  - events
status: stable
generated:
  by: omp/claude-opus-5-5
  at: "2026-11-02T23:30:00Z"
sources:
  - resource: app/README.md
    title: tower app package README
  - resource: app/RECIPE.md
    title: tower app recipe (install, behavior, watch-outs)
---

# Herdr app package

`app/` holds a layout (`layouts/tower.json`) and five mods for the Herdr
desktop app (`herdr-app`, the GUI over herdr-core). Installed under
`~/.config/herdr-app/`, they put tower's fleet beside the terminals:
Agents, Jobs, Events and Inbox as tabs of the right panel (one shown at a
time, so none takes room until opened; the layout has no sidebar); and a
`tower-tabs` tab bar. `app/RECIPE.md` is the install and behavior contract;
this doc records the facts and decisions behind it.

## How the mods reach tower

- **Agents, Inbox, Jobs** call the `tower` CLI with `--json` (`tower.js`
  `tower()`). The CLI resolves the server and token like every client, so
  these panels honor `TOWER_URL` / `TOWER_TOKEN`. Agent screens (the
  Agents detail section, the Inbox peek) use `towerRead()`, plain `tower
  read`: tower 0.5.0's `read` ignored `--json`. The Agents screen adds
  `--ansi`.
- **Agents detail.** Clicking an agent opens a section below the list,
  the same shape as the events detail: prompt input, actions (interrupt,
  stop/remove with a second press, resume = `spawn --continue` on a dead
  row), facts (owned job and reserved jobs from `tower task list`), and
  its screen, re-read every 2 s while open. The screen is a
  `v_virtual_list` of lines, not an `EditorState`: the editor has no
  scroll API, and the list's `VirtualListScrollHandle.scroll_to_bottom()`
  keeps it on the newest line. A wheel-up (`delta.y > 0`) stops following
  until the `↓ Follow` button; the trade-off is no text selection or copy.
- **Agent screen colors.** `tower read --ansi` keeps herdr's SGR escapes
  (truecolor, 256-color, bold/italic/inverse; no cursor moves).
  `tower-agents/ansi.js` parses each line into runs of one style, and each
  run is its own element. The 16 basic colors map to the theme's
  `ansi0`–`ansi15`, so they follow the theme; the rest are hex. Parsing
  costs about 0.5 ms per changed read, and the list only draws visible
  rows. herdr's `--ansi` window can start a few lines lower than the plain
  read; the content is the same.
- **Tab bar** (`tower-tabs`) polls `tower inbox` every 10 s itself and
  colors the Inbox button `blocked` while anything is pending. The Inbox
  panel is usually closed, so the always-shown tab bar carries the signal.
- **Events** reads `/v1/events` (SSE, [D§7](design/07-server-api.md))
  directly with bounded `curl --max-time 5` polls every 6 s. Mods cannot
  read the environment, so it reads the token file from the tower home for
  `window().platform` (or shared state `tower.token_file`) and uses the
  default bind (or `tower.server_url`).
- Every mod folder carries an identical copy of `tower.js`: mods cannot
  import across folders. Change one, copy it to all five.

## Events panel rules

- **Token read retries.** The token file is re-read on every poll until
  one is found. A one-shot read at startup left the panel stuck on "no
  tower token found": a layout restored at app start can mount the panel
  before `window()` reports the platform, so the read tried the wrong
  path. Before the platform is known, every platform's default is tried.
- **Cursor = batch id.** Each SSE batch's `id:` is the seq the server
  scanned to, filtered or not. The panel advances its cursor by the `id:`
  of each complete batch (one ending in a blank line); a batch cut by
  `--max-time` is replayed next poll. Filtered views never rescan skipped
  events.
- **Screens are separate.** `agent.output` events are whole screen
  snapshots (~16 KB each, ~98% of a working log). They show only under
  the "screens" filter, as the TUI hides them by default
  ([D§11](design/11-tui.md)). The server filter is include-only, so other
  views still receive them and drop them client-side.
- **Names, not ids.** Events carry ids. `events.js` `Names` resolves them
  from `tower ps`, `tower task list` (all states) and
  `tower schedule list` every 60 s (sooner after a new unknown id; an id a
  refresh missed is never retried), and from the events themselves:
  `agent.created`/`agent.removed` names, `task.created` titles. A removed
  schedule's title survives as the title of its jobs.
- **Rows and detail.** Rows are two lines (time · kind · subject, then the
  detail) to fit the narrow panel. Clicking a row opens a detail section
  below the list: payload fields with ids resolved and local timestamps,
  then the long body (agent screen, job result, message from/to) in a
  read-only `Editor`.

## Limits

- `message.created` payloads carry no text, so messages show sender,
  recipient, kind and status only.
- Load and every filter change replay the whole log from cursor 0; with a
  large log the panel takes a long time to reach the present. A
  server-side exclusion filter or a tail start would fix it (a design
  change: amend [D§7](design/07-server-api.md) first).
- `herdr-app check` validates a mod's load, not its runtime: verify live
  (`herdr-app shot`, `herdr-app do layout.select --stack panel-body
  --child tower-events`).
