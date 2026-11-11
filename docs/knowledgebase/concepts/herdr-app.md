---
type: Concept
title: Herdr app package
description: The app/ layout and mods that put tower's agents, workflows, inbox, jobs and event log in the Herdr desktop app; how each mod reaches tower, which machine the panels follow, how the workflows graph is laid out, and the rules the events panel follows.
tags:
  - herdr
  - herdr-app
  - mods
  - workflows
  - events
status: stable
generated:
  by: claude-code/1.0
  at: "2026-11-10T00:00:00Z"
sources:
  - resource: app/README.md
    title: tower app package README
  - resource: app/RECIPE.md
    title: tower app recipe (install, behavior, watch-outs)
---

# Herdr app package

`app/` holds a layout (`layouts/tower.json`) and seven mods for the Herdr
desktop app (`herdr-app`, the GUI over herdr-core). Installed under
`~/.config/herdr-app/`, they put tower's fleet beside the terminals:
Agents, Workflows, Jobs, Events and Inbox as tabs of the right panel (one
shown at a time, so none takes room until opened; the layout has no
sidebar); a `tower-tabs` tab bar; and a `tower-spaces` dock strip on the far left.
`app/RECIPE.md` is the install and behavior contract; this doc records the facts and
decisions behind it.

## How the mods reach tower

- **Agents, Workflows, Inbox, Jobs** call the `tower` CLI with `--json` (`tower.js`
  `tower()`) on the machine the panels follow (see Machines below). The
  CLI resolves the server and token like every client, so these panels
  honor `TOWER_URL` / `TOWER_TOKEN` where it runs. Agent screens (the
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
  until the `↓ Follow` button; the live screen has no text selection. The
  `Copy…` button next to Follow opens `tower-agents` itself as a dialog
  (`openOverlay({ mod: "tower-agents", kind: "dialog", props: { output:
  { name, text } } })`): `init` sees `props().output`, builds a readonly
  `EditorState` of the plain-text snapshot (no colors) and starts no
  timers; selection and Cmd-C work, Esc and Close dismiss it.
- **Detailed prompt.** `Detailed…` beside Send opens `tower-agents` as a
  dialog the same way (`props: { compose: { name, machine, text } }`),
  holding a `TextareaState` (soft wrap). Send or Cmd-Enter runs `tower
  prompt` on that machine and closes it; herdr delivers a prompt with
  newlines as one message (checked with omp). Esc closes any dialog
  without asking the mod, so the text survives in `close()`, which writes
  it to shared state `tower-agents.drafts` (`{"<machine>/<agent>":
  text}`, `""` = this computer) unless it was sent. The next `Detailed…`
  for that agent starts from the draft, with the line's text after it.
- **Connect: a view, not the agent.** Tower's agents run in the herdr
  server's `default` session, not in Herdr-app's terminals, so no
  `terminal.*` command reaches them. `herdr agent attach <name>` is a
  client of that one pane: the agent keeps running when the client ends
  (checked by killing it, and by closing its tab). Herdr allows one
  `agent attach` client per pane; a second fails with `already has an
  attached client`, so Connect passes `--takeover`. An open `herdr
  session attach` does not count, and keeps working. Connect looks for a
  named tab with the agent's name in a space on the followed machine and
  shows it; else `tab.new { name, space, argv }` in the shown space on
  that machine (or its first space), else `space.new { name, machine,
  tab, argv }`. A space's terminals run on its machine, so on kobo the
  argv runs kobo's `herdr`, which has the agent.
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
  (`task create`; with an owner it is delivered when the owner is free,
  else reserved for it), `at a time` (`task create --assign --at`; a
  hold needs an owner, the CLI's rule) or `repeats` (`schedule create
  --daily|--cron [--tz]`); a run
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
- **Reserved, not refused.** Every assignment waits for a free agent
  ([agent availability](../decisions/agent-availability.md)). When the
  agent picker's `task assign` or the form's `task create --assign`
  answers with `waiting`, the panel's notice line shows `Reserved for
  <agent>: <waiting.reason>` (`waitingNotice()` in `jobs.js`). There is
  no separate reserve choice: every assignment reserves when it must.
- **Tab bar** (`tower-tabs`) polls `tower inbox` every 10 s itself and
  colors the Inbox button `blocked` while anything is pending. The Inbox
  panel is usually closed, so the always-shown tab bar carries the signal.
- **Panel keys live in the person's `keys.json`.** Herdr has no way for a
  layout or mod to ship default bindings: `keys.json` is the only source,
  and a shared mod brings a `keys.json` to merge. `app/keys.json` binds
  `cmd-shift-left` / `cmd-shift-right` to `tower-tabs.prev` / `.next`
  (delivered to the tab bar's `nextCommand()`) in a `layout:tower`
  section, so the keys act only while this layout shows. Install adds a
  key only if no section of the person's file binds it: `layout:<name>`
  beats `global`, so adding a key they already use would silently take
  it. After that the binding is theirs to change.
- **Events** reads `/v1/events/history` (one page, newest first) and then
  `/v1/events` (SSE) from the page's `head`
  ([D§7](design/07-server-api.md)), directly with `curl`: bounded
  `--max-time 5` polls every 6 s for the stream. It needs a tower 0.6.0
  server; an older one gets a "needs tower 0.6.0" error. `tower.js`
  `towerHttp()` runs an `sh` script on the followed machine that resolves
  the URL and token the way the CLI does (`TOWER_URL` / `TOWER_TOKEN`, else
  the tower home's `config.toml` bind and token file), then `curl`. Mods
  cannot read the environment, but `exec` runs programs in the user's
  real one.
- **Spaces strip** (`tower-spaces`) sits on the far left of the workspace as
  a thin 38px vertical panel. It renders a 28x28 square icon with a status dot
  for each space (`spaces()`). Clicking selects the space (`space.select`),
  right-clicking opens the space context menu, and dragging reorders spaces
  within their machine (`space.move`). The selected space is highlighted with
  an accent border. Machines are separated by a horizontal divider with a
  machine tooltip. Dot colors reflect the most urgent agent or terminal status
  in the space: `blocked` > `done` > `working` > `idle`, with plain shell
  terminals showing a quiet foreground/dim dot.
- Every mod folder carries an identical copy of `tower.js`: mods cannot
  import across folders. Change one, copy it to all seven.
## Machines

- **One machine at a time, the shown space's by default.** herdr spaces
  live on a machine (`spaces()[].machine`, `hc machine ls`); the panels
  follow the shown space's. Shared state `tower.machine` pins one
  instead: `""` is this computer, a name a remote, unset (null) follows.
  The tab bar's machine button names the machine (a dot marks a pin) and
  opens `tower-tabs` itself as a popup picker.
- **Calls run on the machine, not to it.** Every call is `exec` with `on`:
  that machine's own `tower` CLI (or the events `sh` script) through
  herdr-core's ssh connection, reaching its server on loopback with its
  token. Chosen over a host/port per machine because nothing has to
  listen beyond loopback (tower has no TLS; D§17 open question 4) and no
  token is copied between machines. A remote call costs about 20–30 ms.
  A remote CLI is found on PATH or at home-relative fallbacks
  (`.cargo/bin`, `.local/bin`); a found CLI is cached per machine, a miss
  is asked again next call.
- **Stack names are a person's, on this computer.** Tower has no stack
  object (D§2): a stack is the `<stack>-` name prefix. The Agents panel
  shows a stack's heading from `<app home>/data/tower/stacks.json`
  (`{"kb": "Knowledgebase"}`), else the prefix in capitals. The file is read
  on this computer, not the followed machine, so names stay put across a
  switch; it is chosen over a server-side stack record because only the
  app shows the name.
- **A switch starts over.** Each panel checks `target()` every 500 ms
  (`followMachine`; `render` has no `cx`), clears everything from the old
  server, reloads, and drops replies and action results started on the
  old machine; actions go to the machine they started on. Event seqs and
  ids are per server, so the events cursor and name cache reset; open
  forms close.
- **Workflow definitions.** `exec` has no stdin and `writeTextFile` no
  `on`, so `applyWorkflow()` writes the local scratch file here and, on a
  remote, passes the source as an `sh` argument that writes a `mktemp`
  file there before `tower workflow apply`.
- **Phase 5 is the fleet-wide answer.** One coordinator with node agents
  ([D§18](design/18-phase-mapping.md)) would show every machine's agents
  in one list; the switch is the view until then.

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
  shows every visit. Opening a workflow whose newest run is waiting or
  running shows that run, and an open run re-reads every 3 s. A
  `waiting` run (it waits for agents another run claims) has no steps
  yet: its facts show `waits for` with each agent and the run ahead of
  it (`waiting_for` from `run show`), and `Cancel run` works on it. The
  state color for `waiting` is the theme's `blocked` (`STATE_COLOR` in
  every `tower.js` copy).
- **Creating.** The form writes TOML (`draftToml()`): one node per step,
  one edge per outcome (or "when done") that doesn't go to `end`; `next`
  is the following step. A route to the same or an earlier step is a
  loop, and its target gets `max_visits = 3`. Step 1 is never offered as
  a target: a start is a node with no incoming edge, loops included, so
  the server would refuse it. Everything else (mapped agents, joins,
  triggers, context) is the TOML editor's job; `TOML…` carries the form
  over. Mods cannot pass stdin to a program, so a definition reaches the
  server as a file: `applyWorkflow()` writes
  `<herdr-app home>/data/tower-workflows/draft.toml` (a `mktemp` file on
  a remote machine), then `tower workflow apply <path>`.
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

- The events panel and remote workflow applies need `sh` (and the events
  panel `curl`) on the followed machine, so Windows is not supported there.
- `message.created` payloads carry no text, so messages show sender,
  recipient, kind and status only.
- `herdr-app check` validates a mod's load, not its runtime: verify live
  (`herdr-app shot`, `herdr-app do layout.select --stack panel-body
  --child tower-events`). `shot` saves the last rendered frame: with the
  window in the background it can be minutes old, so bring Herdr to the
  front (`osascript -e 'tell application "Herdr" to activate'`) first.
  Inspect the frame itself; `status.errors` can be empty while a mod shows a
  render-error card. Color arguments accept theme values or hex literals,
  not CSS names: `tower-spaces` uses `#00000000` for inactive borders because
  `transparent` causes a live render failure that `check` did not catch.
