# tower

A tower control deck for the Herdr app: the fleet, the job queue, the
inbox and the live event log in a panel beside the terminals. Everything the `tower`
CLI can show, as panels that act — prompt an agent, answer an inbox item,
queue a job, pause a schedule — while the terminals you're already in stay
in the middle.

## Before you add it

The panels talk to a running tower server through the `tower` CLI
(`tower --version` to check). The CLI finds the server the way the TUI
does: `TOWER_URL`/`TOWER_TOKEN`, else the tower home's `config.toml` and
token file. No tower, and every panel says so.

## What you see

A clone of the shipped `main` layout with no sidebar (the terminals get
the full width) and tower panels in place of the Files/Changes panel:

- **Right panel** (380px), opened from the tab bar's Agents / Jobs /
  Events / Inbox buttons and remembered per space: a stack of:
  - **Agents**: every agent seat grouped by stack (agents named
    `<stack>-<role>`), each row a state dot (theme colors: `working`,
    `blocked`, `done`, `idle`; dead red `ansi1`), the name, the state word,
    and the job it owns (else its workdir). Click a row: a detail section
    opens below the list (the list keeps the top third) with a prompt
    input (Enter or Send runs `tower prompt <name> <text>`), the actions
    (`Interrupt`, `Stop` and `Remove` with a second press, `Resume` =
    `tower spawn <name> --continue` on a dead row, `Following` /
    `↓ Follow`), the agent's facts (job, reserved jobs waiting for it,
    workdir, kind, permissions, worktree, machine, pane, desired state,
    adopted, created/updated, id), then its terminal screen in the agent's
    own colors (the last 200 lines of `tower read --ansi`, re-read every 2s
    while open). The screen follows the newest line; scrolling up pauses
    that (the button turns solid `↓ Follow`) until the button is pressed.
    Click the row again or `×` to close. The header's `＋ Spawn…` opens a
    form: name, harness pills (installed kinds, via `knownAgents()`),
    workdir, first prompt → `tower spawn`.
  - **Jobs**: QUEUE section (open jobs: state dot, title, `P<n>` priority,
    state, owner/hold/tags/age), RECENT (last 10 closed), SCHEDULES
    (title, cron, next run, target). Per-job buttons: `Assign…` (pills of
    live agents), `Release` (owned jobs), `Cancel` (two presses; interrupts
    the owner). Per-schedule: `Run now`, `Pause`/`Resume`, `Remove`
    (two-press). Footer `＋ Queue a job…`: title, description, priority
    pills, owner pills (queue or an agent, with a "deliver when free"
    reserve checkbox).
  - **Events**: the newest 50 events (`/v1/events/history`), then the
    `/v1/events` feed live from there, newest first, two lines per event:
    time (date when not today), event kind (colored by the state it moved
    to: failures red), subject (agent name, job or schedule title,
    `from → to` for messages), then the detail in operator words
    ("assigned to s1-manage · by schedule …", "working → done", a job's
    result summary). Filter pills: all / agents / jobs / schedules /
    messages / screens. `agent.output` events (whole screen snapshots,
    most of the log) show only under screens, as the TUI hides them by
    default. Click a row: a detail section opens below the list with
    every payload field (ids resolved to names, timestamps local) and,
    when there is one, the long body in a read-only editor — an agent's
    screen, a job's result, a message's from/to. Click the row again or
    `×` to close. Live dot `● live` (dim) or `◌ connecting` (red). The
    list ends in `Load older (50)` (the next page back) while older
    matching events exist, else "Start of the log."
  - **Inbox**: pending questions/approvals addressed to you. Each item:
    kind dot (approvals `blocked` color, questions `accent`), sender, kind,
    deadline ("4m left", red when passed), text, then buttons: `Peek agent`
    (the sender's recent screen via `tower read`), `Approve?`/`Deny`
    (two-press confirm — an approval lets an agent act), `Answer…` (inline
    input, Enter sends `tower approve <id> --answer`), `Dismiss`.
- **Center**: the shipped pane cards, unchanged.
- **Tab bar**: a clone of the shipped `tabs` mod (same tabs, drag, `+`,
  window buttons) with Agents / Jobs / Events / Inbox panel buttons in
  place of Files / Changes. The clone is required: the shipped `tabs`
  polices the panel against `shared("workspace.panels")` (files/changes)
  and would close a tower panel on every space switch. The clone's
  `enabled` list is `["tower-agents", "tower-jobs", "tower-events",
  "tower-inbox"]`, so its per-space memory restores a
  space's last tower panel instead. The Inbox button turns the theme's
  `blocked` color (label and icon) while anything is pending; the tab
  bar runs its own `tower inbox` poll every 10s, so this works with the
  panel closed.

## How it behaves

- Every panel polls the `tower` CLI every 10s (events: bounded SSE polls
  every 6s). A panel action refreshes immediately after its command
  returns.
- Actions that stop work or let an agent act (Stop, Cancel, Approve,
  Deny, Remove) ask once with a second press.
- Errors from the server (conflict, not found, unauthorized) show in a
  red strip inside the panel that acted; they clear on the next success.
- The events feed loads the newest 50 matching events on load and after
  a filter change, then follows live from the page's `head`, so nothing
  between the page and the stream is lost or shown twice. Nothing is lost
  between polls: each SSE batch's `id:` is the seq it scanned to, so each
  bounded poll resumes exactly where the last complete batch ended (a
  batch cut by `--max-time` is replayed). A caught-up poll ends in curl
  exit 28 with no output: that is a quiet poll, not an error.
- Names come from `tower ps`, `tower task list` and `tower schedule list`
  (every 60s, sooner after an unknown id) and from the events themselves
  (`agent.created`/`removed` names, `task.created` titles — a removed
  schedule's title survives as its jobs' title).

## Built from

- One layout: `layouts/tower.json`, cloning the shipped `main` structure
  minus its sidebar (work / status blocks, shipped `pane` header,
  `workspace-status` mod reused as-is; the tab bar is the `tower-tabs`
  clone below, which pads for the macOS window buttons itself when there
  is no sidebar).
- Five mods, each a folder with `main.js`, `mod.json` (`place: "none"` —
  the layout is the only placement), and a shared `tower.js` (identical
  in every folder):
  - `tower-agents`: `tower ps`, `tower task list` (owned and reserved
    jobs), `tower read --ansi` (the screen, colors parsed by `ansi.js`),
    `tower prompt`, `tower interrupt`,
    `tower stop [--remove]`, `tower spawn [--continue]`; `knownAgents()`
    for the spawn form.
  - `tower-inbox`: `tower inbox`, `tower approve [--answer|--deny]`,
    `tower dismiss`, `tower read` for the peek.
  - `tower-jobs`: `tower task list`, `tower task create/assign/release/
    cancel`, `tower schedule list/run/pause/resume/rm`.
  - `tower-events`: the tower REST `/v1/events/history` page and the
    `/v1/events` SSE endpoint via bounded `curl --max-time 5` polls
    (cursor from the page's `head`); the token from the tower
    home for the platform (`window().platform`: `~/.local/share/tower`
    Linux, `~/Library/Application Support/tower` macOS,
    `~/AppData/Roaming/tower` Windows), or the shared-state override
    `tower.token_file`; the address from `tower.server_url` or the
    default bind. `events.js` formats rows and the detail section and
    resolves names (`tower ps`, `tower task list`, `tower schedule list`).
  - `tower-tabs`: a clone of the shipped `tabs` mod, `enabled` hardcoded
    to `["tower-agents", "tower-jobs", "tower-events", "tower-inbox"]`,
    panel buttons switched to those four (icons `mods/tower-tabs/agents.svg`,
    `jobs.svg`, `events.svg`, `inbox.svg`), plus a `tower inbox` poll for
    the Inbox button's color.
    Everything else is the shipped tab bar: tab pills, drag-to-reorder,
    `+` menu, window buttons.
- `herdr`: `exec`, `theme`, `readTextFile`, `knownAgents`, `layout`, `run`,
  `settings`, `shared`, `window`. `gpui-base`: `v_flex`, `h_flex`,
  `InputState`, `Input`, `Editor`, `EditorState`.
  `gpui-kit`: `View`, `div`.
- The mods run programs (`tower`, `curl`) and read one file outside their
  folders (the tower token file). They reach no network except the local
  tower server.

## Watch out

- Mods cannot import across folders ("resolves outside the application
  directory"): each mod carries its own copy of `tower.js` (identical
  md5 across the set).
- The `tower-tabs` clone already uses `this.pending` (its panel-restore
  guard); don't reuse that name for mod state.
- `tower read --json` prints plain text on tower 0.5.0 (the flag is
  ignored), so `tower.js` has `towerRead(name, { ansi })`, which runs
  plain `tower read` (with `--ansi` for colors) — the text is the API's
  `output` field either way.
- The shipped `tabs` mod must not be reused for this layout: its
  `syncPanel` closes any panel whose active child is not in
  `shared("workspace.panels")` on every space switch. That is why the
  tab bar here is the `tower-tabs` clone. Never point the layout's
  tab bar back at the shipped `tabs`.
- `herdr-app check` drops `gpui-kit.d.ts` and `jsconfig.json` into the
  folder it checks — don't commit those; delete them after checking.
- The mod runtime has no `setTimeout`/`process`/env access: timing is
  `cx.timer.every`, and `TOWER_URL`/`TOWER_TOKEN` overrides are invisible
  to the mods (they go through the CLI, which does see the environment —
  run the app from a shell with them set if you need an override).
- The events panel needs a tower 0.6.0 server (`/v1/events/history`,
  `exclude=`); an older one shows "this panel needs tower 0.6.0". The
  bounded-poll approach assumes the server keeps the default
  `127.0.0.1:8266` bind (mods cannot read the tower config).
- The events panel re-reads the token file on every poll until it finds
  one. A one-shot read at startup can miss: a restored layout can mount
  the panel before `window()` reports the platform, and a token written
  after the app opened would never be picked up.
- `InputState.new` belongs in handlers, never in `render`.
- Two-press confirms reset on the next refresh cycle if the row changed.

## Done when

- `herdr-app check` passes for each mod folder. The tab bar shows Agents /
  Jobs / Events / Inbox buttons; clicking Jobs opens the panel on the Jobs stack, and
  after switching to another space and back, that space's panel is as it
  was left (the shipped `tabs` would have closed it).
- With a tower server running: the Agents panel lists every agent
  `tower ps` shows, grouped by stack, with live states; the Jobs panel's
  QUEUE matches `tower task list`, and SCHEDULES matches
  `tower schedule list`; the Events panel opens on the newest events at
  once, follows live within ~6s, and `Load older` adds the page before
  the oldest row.
- Prompt an idle agent from its detail section → the agent's terminal
  shows the turn, and the section's screen shows it within ~2s;
  `tower prompt <name>` shows the same delivered message.
- A pending question appears in the Inbox within ~10s; Answer it in
  place → `tower inbox` no longer lists it, and `tower inbox --json`'s
  sender sees `responded_at`.
- Queue a job from the form → `tower task list` shows it; Assign it to an
  agent → its state moves to assigned, the owner's name shows in the row.
- `tower ui`'s web inbox and this panel agree on what needs you.