---
type: Concept
title: Herdr app package
description: The app/ layout and mods that put tower's agents, workflows, inbox, jobs and event log in the Herdr desktop app; how each mod reaches tower, how the workflows graph is laid out, and the rules the events panel follows.
tags:
  - herdr
  - herdr-app
  - mods
  - workflows
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

`app/` holds a layout (`layouts/tower.json`) and six mods for the Herdr
desktop app (`herdr-app`, the GUI over herdr-core). Installed under
`~/.config/herdr-app/`, they put tower's fleet beside the terminals:
Agents, Workflows, Jobs, Events and Inbox as tabs of the right panel (one
shown at a time, so none takes room until opened; the layout has no
sidebar); and a `tower-tabs` tab bar. `app/RECIPE.md` is the install and
behavior contract; this doc records the facts and decisions behind it.

## How the mods reach tower

- **Agents, Workflows, Inbox, Jobs** call the `tower` CLI with `--json` (`tower.js`
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
- **Jobs detail.** Clicking a job or schedule opens a section below the
  list, the shape of the Agents detail and the TUI's job / schedule side
  panel ([D§11](design/11-tui.md)). A job shows facts, prompts, the
  hook in flight, its result and its trail and messages from `tower task
  show`. The trail reads like the TUI's `trail_line`, with names in place
  of ids. A schedule shows its cadence, prompts and recent runs from
  `tower schedule show`. A run opens as a job, with a `‹ schedule` way
  back. The actions depend on state: Assign / Edit / Cancel while queued,
  Release / Edit post-run / Cancel while delivered, and Run now / Pause /
  Resume / Edit / Remove for a schedule. Edit replaces the body with a
  form that follows D§5.2.3: a queued job takes every field, a delivered
  one only post-run, and `at` shows only for a reserved job. Save sends
  one `edit` with the fields that changed, each as `--flag=value` so a
  leading `-` stays a value; an emptied field clears it.
  `tower-jobs/jobs.js` holds these rules and the formatters. RECENT sorts
  closed jobs by `updated_at`, because `task list` puts the oldest first.
- **Jobs create form.** `＋ New…` is one form for the ways work enters
  tower, first `a job` or `a workflow run`, then WHEN: a job `now`
  (`task create`, optionally reserved with "deliver when free"), `at a
  time` (`task create --assign --at`; a hold needs an owner, the CLI's
  rule) or `repeats` (`schedule create --daily|--cron [--tz]`); a run
  `now` (`workflow run --input`) or `repeats`. One form, not a second
  button, because the fields overlap and only the timing differs.
  Pre-run and post-run are on the form so a job needs no edit after it
  is created.
- **A job that starts a workflow.** Jobs carry tags, and
  `task.completed` / `task.failed` events carry the job's title and tags
  ([D§5.4](design/05-core-objects.md)), so a workflow event trigger with
  `match = { tags = "deploy" }` starts when such a job closes. The form
  names the workflows a job's tags would start (`tagTriggers`: enabled
  workflows whose trigger on a job event matches only a tag). A tower
  0.7.0 server leaves title and tags out of those two events, so
  completion triggers on tags need a newer server.
- **A repeating workflow run** is a schedule trigger in the workflow's
  TOML, not a `schedule create`: the definition owns its schedules
  (decision: workflows §5). The form appends a `[[triggers]]` table to
  the source from `workflow show` and applies it as a new version; a
  source that declares `triggers = [...]` inline is refused by the
  server's parser, and its error shows in the strip.
  `jobs.js` builds every command, `--flag=value` like edit.
- **Tab bar** (`tower-tabs`) polls `tower inbox` every 10 s itself and
  colors the Inbox button `blocked` while anything is pending. The Inbox
  panel is usually closed, so the always-shown tab bar carries the signal.
- **Events** reads `/v1/events/history` (one page, newest first) and then
  `/v1/events` (SSE) from the page's `head`
  ([D§7](design/07-server-api.md)), directly with `curl`: bounded
  `--max-time 5` polls every 6 s for the stream. It needs a tower 0.6.0
  server; an older one gets a "needs tower 0.6.0" error. Mods cannot
  read the environment, so it reads the token file from the tower home for
  `window().platform` (or shared state `tower.token_file`) and uses the
  default bind (or `tower.server_url`).
- Every mod folder carries an identical copy of `tower.js`: mods cannot
  import across folders. Change one, copy it to all six.

## Workflows panel

The panel over [workflows](../decisions/workflows.md) (D§5.6); it needs a
tower 0.7.0 server.

- **A list, then a screen.** The panel lists workflows; clicking one
  replaces the list with that workflow (`‹ Workflows` goes back), the way
  PR Flow's PR timeline does, instead of the other panels' detail section
  below the list: a graph needs the panel's height.
- **The graph is a top-down list, not a node canvas.** `workflows.js`
  `graphRows()` turns a definition into rows: a chain of single edges
  stays at one depth; where a node fans out, each target is a branch one
  level deeper with its edge's condition as a chip. Edges back to a node
  on the current depth-first path (loops) are `↺ back to <node>` rows, so
  a loop never draws a node twice. A node with several forward edges in
  (a join) waits until all its predecessors are drawn and then shows
  once, at the depth where its branches split, with `after a, b`. Several
  start nodes hang under one `Start` row. Rails are computed from the
  bottom up: a level's line continues while a later row sits at that
  level before a shallower one. Conditional paths read as indented
  branches, which is why a full node graph was not needed.
- **Runs over the graph.** `tower run show` has no definition, so a run
  is drawn on the workflow's current version; a run of another version
  says so, and steps of nodes the version lacks are listed after the
  graph. Each node's mark and line come from its latest step; a click
  shows every visit. Opening a workflow whose newest run is running
  shows that run, and a running run re-reads every 3 s.
- **Creating.** The form writes TOML (`draftToml()`): one node per step,
  one edge per outcome (or "when done") that doesn't go to `end`; `next`
  is the following step. A route to the same or an earlier step is a
  loop, and its target gets `max_visits = 3`. Step 1 is never offered as
  a target: a start is a node with no incoming edge, loops included, so
  the server would refuse it. Everything else (mapped agents, joins,
  triggers, context) is the TOML editor's job; `TOML…` carries the form
  over. Mods cannot pass stdin to a program, so a definition reaches the
  server as a file: `writeTextFile` to
  `<herdr-app home>/data/tower-workflows/draft.toml`, then `tower
  workflow apply <path>`.
- **Errors.** An action's error stays until the next action; the 10 s
  poll keeps its own error, so a successful poll right after a refused
  apply does not erase the server's reason.

## Events panel rules

- **Token read retries.** The token file is re-read on every poll until
  one is found. A one-shot read at startup left the panel stuck on "no
  tower token found": a layout restored at app start can mount the panel
  before `window()` reports the platform, so the read tried the wrong
  path. Before the platform is known, every platform's default is tried.
- **Newest first, then live.** Load and every filter change fetch the
  newest 50 matching events, then follow the stream from the page's
  `head`: nothing between the two is lost or shown twice. "Load older" at
  the end of the list pages back with `before` = the oldest seq held. The
  panel keeps 2000 rows; trimming the oldest brings "Load older" back. A
  generation counter drops pages and polls from a filter that has since
  changed, and a change mid-poll loads the new page as soon as that poll
  returns.
- **Cursor = batch id.** Each SSE batch's `id:` is the seq the server
  scanned to, filtered or not. The panel advances its cursor by the `id:`
  of each complete batch (one ending in a blank line); a batch cut by
  `--max-time` is replayed next poll. Filtered views never rescan skipped
  events. A caught-up stream sends nothing (keep-alives are 15 s apart),
  so curl's exit 28 (`--max-time`) with no output is a normal quiet poll,
  not "cannot reach the tower server".
- **Screens are separate.** `agent.output` events are whole screen
  snapshots (~16 KB each, ~99% of a working log). They show only under
  the "screens" filter, as the TUI hides them by default
  ([D§11](design/11-tui.md)); every other view sends
  `exclude=type:agent.output`, so the server never sends them.
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
- `herdr-app check` validates a mod's load, not its runtime: verify live
  (`herdr-app shot`, `herdr-app do layout.select --stack panel-body
  --child tower-events`).
