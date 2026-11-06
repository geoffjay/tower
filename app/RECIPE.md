# tower

A tower control deck for the Herdr app: the fleet, workflows, the job
queue, the inbox and the live event log in a panel beside the terminals.
Everything the `tower` CLI can show, as panels that act — prompt an agent,
start a workflow run, answer an inbox item, queue a job, pause a schedule
— while the terminals you're already in stay in the middle.

## Before you add it

The panels talk to a running tower server through the `tower` CLI
(`tower --version` to check). The CLI finds the server the way the TUI
does: `TOWER_URL`/`TOWER_TOKEN`, else the tower home's `config.toml` and
token file. No tower, and every panel says so. The panels follow the
shown space's machine (or a pinned one, from the tab bar's machine
button), so a remote machine needs its own `tower` CLI and server.

## What you see

A clone of the shipped `main` layout with no sidebar (the terminals get
the full width) and tower panels in place of the Files/Changes panel:

- **Right panel** (380px), opened from the tab bar's Agents / Workflows /
  Jobs / Events / Inbox buttons and remembered per space: a stack of:
  - **Agents**: every agent seat grouped by stack (agents named
    `<stack>-<role>`) under a heading: the stack's name from
    `<app home>/data/tower/stacks.json` (`{"<stack>": "<name>"}`, this
    computer's file, read with `readTextFile` on every poll; missing or
    invalid = no names), else the prefix in capitals. Each row a state dot (theme colors: `working`,
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
  - **Workflows**: every workflow (`tower workflow list`): state dot
    (`working` while a run is running, `idle` enabled, dim disabled), the
    name, `v<n>`, the state word ("2 running" / "enabled" / "disabled"),
    then steps · triggers · description. Click a row: the panel shows that
    workflow (`tower workflow show`), with `‹ Workflows` to go back:
    name, description, actions (`Run…` while enabled, `Edit`,
    `Disable`/`Enable`, `Remove` with a second press), facts (triggers,
    inputs with `*` for required, result node, next scheduled run,
    updated), RUNS (the newest 5, `show all` for the 20 the server keeps:
    state dot, start time, state, result gist or trigger), then the GRAPH
    as a top-down list of nodes in the shape of a PR timeline: a mark per
    node on a vertical rail, the node id, its agent (a mapped agent as
    `builder | designer (by triage)`), its outcomes as chips, `join all` /
    `≤3 visits` badges. A chain of single edges stays in line; where a
    node fans out, each target is a branch indented under it with its
    condition as a chip (`on design`, or `then` for an unconditional fan
    out); an edge back to a node already on the path is a `↺ back to
    <node>` row, never a second copy; a node several edges enter (a join)
    shows once, after the branches it waits for, with `after a, b`;
    several start nodes hang under a `Start` row. Click a node: its prompt
    and the nodes it reads. Click a run: the graph shows that run (the
    newest one opens by itself when it is running) — run facts (trigger,
    started, input, ended, the result or error), and each node's latest
    step as its mark (filled check done, filled cross failed, spinner
    running, ring waiting, small dim ring not reached) with visit,
    agent, state, outcome and time, then the summary's first line. Click
    a node: every visit with its full summary, error and job id. A
    running run re-reads every 3s and has `Cancel run` (second press);
    `× Graph` (or the run again) goes back to the definition. A run of an
    older version says so; steps of nodes the current version lacks are
    listed after the graph. `Run…` opens a field per declared input
    (required ones checked before sending) → `tower workflow run <name>
    --input k=v`, then shows the new run. `Edit` opens the stored TOML
    source in a code editor; Save applies it (a change is a new version).
    Footer `＋ New workflow…`: name, description, inputs (comma list, each
    required), and steps, each with an id, agent pills, a prompt, optional
    outcomes, and THEN: per outcome (or "when done" without outcomes)
    pills for `next (<step>)`, `end`, or any later or earlier step except
    step 1 (`↺` marks a loop; the loop's target gets `max_visits = 3`).
    `Create` builds the TOML and applies it; `TOML…` continues in the
    code editor with the form's TOML (a starter definition when the form
    is empty), `‹ Form` goes back. Applying writes the source to the
    mod's draft file and runs `tower workflow apply` on it; the server's
    validation errors show in the red strip, and success opens the
    workflow with "<name> v<n> created/saved/unchanged".
  - **Jobs**: QUEUE section (open jobs: state dot, title, `P<n>` priority,
    state, owner/hold/tags/age), RECENT (the last 10 to close), SCHEDULES
    (title, cron, next run, target). Per-job buttons: `Assign…` (pills of
    live agents), `Release` (owned jobs), `Cancel` (two presses; interrupts
    the owner). Per-schedule: `Run now`, `Pause`/`Resume`, `Remove`
    (two-press). Click a job or schedule: a detail section opens below the
    list (the list keeps the top third). For a job it shows the actions
    for its state (`Assign…`, `Release`, `Edit` / `Edit post-run`,
    `Cancel`, `Open schedule`), facts (waiting for, owner, lease, attempts,
    priority, schedule and occurrence, tags, id), the prompt, pre-run and
    post-run, the hook in flight, the result, the trail ("assigned to
    s1-manage by schedule ...") and its messages (the first lines; click
    for all). For a schedule it shows `Run now`, `Pause`/`Resume`, `Edit`,
    `Remove`, its cadence and next/last run, its prompts and its RUNS
    (time, state, result gist). Click a run to open it as a job; `‹
    <schedule>` goes back. `Edit` swaps the body for a form: title,
    prompt, `at` (reserved jobs), pre-run and post-run for a job, and
    cadence and timezone for a schedule. A delivered job's fields other
    than post-run show dim. Save sends the changed fields; an emptied
    field clears it. Click the row again or `×` to close. Footer `＋ New…`:
    one form, first `a job` or `a workflow run`. A job: title,
    description, TAGS (comma list → `--tag`; under it, "Completing it
    starts workflow X" for every enabled workflow whose event trigger on
    `task.completed` / `task.failed` / `task.created` matches only a tag
    the job carries), priority pills, owner pills (queue or an agent),
    then WHEN: `now` (with an owner, a "deliver when free" reserve
    checkbox) → `tower task create`; `at a time` (an AT field; needs an
    owner) → `tower task create --assign --at`; `repeats` (CADENCE `daily
    at` HH:MM or `cron`, an optional IANA timezone; an owner gets each job
    reserved) → `tower schedule create`. PRE-RUN and POST-RUN fields go
    with any of the three (`--pre-run` / `--post-run`). A workflow run:
    pills of the enabled workflows (`tower workflow list`), a field per
    declared input of the picked one (`tower workflow show`; required
    ones checked), then WHEN `now` → `tower workflow run <name> --input
    k=v`, or `repeats` (the same cadence fields) → a `[[triggers]] kind =
    "schedule"` table with the inputs as its `input` appended to the
    workflow's TOML source and applied as a new version (the definition
    owns its schedules). The submit button says what it does: `Queue
    job`, `Reserve job`, `Create schedule`, `Start run`, `Add schedule`;
    a run or a schedule added to a workflow is confirmed in a grey line
    under the header. `jobs.js` builds every command (`createArgs`,
    `runArgs`, `withScheduleTrigger`, `tagTriggers`) and names what is
    missing.
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
  window buttons) with Agents / Workflows / Jobs / Events / Inbox panel
  buttons in place of Files / Changes. The clone is required: the shipped `tabs`
  polices the panel against `shared("workspace.panels")` (files/changes)
  and would close a tower panel on every space switch. The clone's
  `enabled` list is `["tower-agents", "tower-workflows", "tower-jobs",
  "tower-events", "tower-inbox"]`, so its per-space memory restores a
  space's last tower panel instead. The Inbox button turns the theme's
  `blocked` color (label and icon) while anything is pending; the tab
  bar runs its own `tower inbox` poll every 10s, so this works with the
  panel closed. `cmd-shift-right` / `cmd-shift-left` show the next /
  previous tower panel in button order, wrapping; with the panel closed,
  the first / last.

## How it behaves

- Every panel polls the `tower` CLI every 10s (events: bounded SSE polls
  every 6s; a running workflow run on screen: every 3s). A panel action
  refreshes immediately after its command returns.
- Actions that stop work or let an agent act (Stop, Cancel, Approve,
  Deny, Remove) ask once with a second press.
- Errors from the server (conflict, not found, unauthorized) show in a
  red strip inside the panel that acted. They stay until the next action
  succeeds or another item or form opens; the 10s poll leaves them.
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
- Six mods, each a folder with `main.js`, `mod.json` (`place: "none"` —
  the layout is the only placement), and a shared `tower.js` (identical
  in every folder):
  - `tower-agents`: `tower ps`, `tower task list` (owned and reserved
    jobs), `tower read --ansi` (the screen, colors parsed by `ansi.js`; `Copy…` reopens the mod as a read-only text dialog),
    `tower prompt`, `tower interrupt`,
    `tower stop [--remove]`, `tower spawn [--continue]`; `knownAgents()`
    for the spawn form.
  - `tower-workflows`: `tower workflow list/show/run/enable/disable/rm/
    apply`, `tower run show/cancel`, `tower ps` (agent names, the form's
    agent pills); `workflows.js` lays a definition out as graph rows,
    lays a run's steps over them, and builds the create form's TOML.
    Definitions go through `tower.js` `applyWorkflow()`: here, the draft
    file `<herdr-app home>/data/tower-workflows/draft.toml`
    (`writeTextFile`, then `tower workflow apply <path>`); on a remote
    machine, a `mktemp` file there. `check.svg` and
    `cross.svg` are the done/failed marks, the spinner is the shipped
    `mods/workspace-chrome/spinner.svg`.
  - `tower-inbox`: `tower inbox`, `tower approve [--answer|--deny]`,
    `tower dismiss`, `tower read` for the peek.
  - `tower-jobs`: `tower task list/show`, `tower task create/assign/
    release/cancel/edit`, `tower schedule list/show/run/pause/resume/rm/
    edit`, `tower workflow list/show/run/apply` (the New form's workflow
    runs; the applied file is `<herdr-app home>/data/tower-jobs/
    workflow.toml`); `jobs.js` formats the trail and holds the edit and
    create rules.
  - `tower-events`: the tower REST `/v1/events/history` page and the
    `/v1/events` SSE endpoint via bounded `curl --max-time 5` polls
    (cursor from the page's `head`), through `tower.js` `towerHttp()`:
    an `sh` script on the followed machine resolves the URL and token
    the way the CLI does, then runs `curl`. `events.js` formats rows and
    the detail section and resolves names (`tower ps`, `tower task
    list`, `tower schedule list`).
  - `tower-tabs`: a clone of the shipped `tabs` mod, `enabled` hardcoded
    to `["tower-agents", "tower-workflows", "tower-jobs", "tower-events",
    "tower-inbox"]`, panel buttons switched to those five (icons
    `mods/tower-tabs/agents.svg`, `workflows.svg`, `jobs.svg`,
    `events.svg`, `inbox.svg`), plus a `tower inbox` poll for the Inbox
    button's color and the machine button, which opens `tower-tabs`
    itself as a popup (`props: { machines: true }`) to pick the machine.
    Everything else is the shipped tab bar: tab pills, drag-to-reorder,
    `+` menu, window buttons. A `nextCommand()` loop takes `next` / `prev`
    and selects the neighbouring `panel-body` child, then shows `panel`.
- `keys.json` (the folder's root, herdr.dev convention for a mod's keys):
  `"layout:tower": { "cmd-shift-left": "tower-tabs.prev",
  "cmd-shift-right": "tower-tabs.next" }`. Install merges it into the
  person's `keys.json`, adding only keys no section there binds yet; a
  layout section beats `global`, so adding a key they use would take it.
- `herdr`: `exec`, `theme`, `readTextFile`, `writeTextFile`, `appHome`,
  `knownAgents`, `layout`, `nextCommand`, `run`, `settings`, `shared`, `window`.
  `gpui-base`: `v_flex`, `h_flex`, `InputState`, `Input`,
  `TextareaState`, `Textarea`, `Editor`, `EditorState`. `gpui-kit`:
  `View`, `div`, `svg`.
- The mods run programs (`tower`, `curl`), read one file outside their
  folders (the tower token file) and write one (the workflows draft, in
  the app's data folder). They reach no network except the local tower
  server.

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
  `cx.timer.every`. `exec` runs programs with the user's real
  environment, so the CLI and the events panel's `sh` script see
  `TOWER_URL`/`TOWER_TOKEN` where they run.
- The machine: shared state `tower.machine` pins one (`""` this computer,
  else a name from `hc machine ls`); unset follows the shown space's
  `machine`. Every call goes to that machine (`exec` with `on`). A panel
  checks it every 500 ms (`followMachine`), clears everything from the
  old server and reloads; replies started on the old machine are
  dropped, and actions go to the machine they started on. Event seqs are
  per server, so the events cursor resets on a switch.
- The events panel needs a tower 0.6.0 server (`/v1/events/history`,
  `exclude=`); an older one shows "this panel needs tower 0.6.0".
- `InputState.new` belongs in handlers, never in `render`.
- Two-press confirms reset on the next refresh cycle if the row changed.
- The Workflows panel needs a tower 0.7.0 server (`tower workflow`,
  `tower run`). Its graph is the workflow's current version: a run of an
  older version is drawn on it, with a note.
- A workflow's start is a node with no incoming edge, loops included, so
  the create form never offers step 1 as a route target.
- Action errors stay until the next action; a poll's success must not
  clear them, so every panel that acts keeps the poll's failure in its
  own field (`loadError`). Agents, Inbox and Jobs used to share one
  field, and the refresh after a failed action erased the error before
  it was drawn.

## Done when

- `herdr-app check` passes for each mod folder. The tab bar shows Agents /
  Workflows / Jobs / Events / Inbox buttons; clicking Jobs opens the panel on the Jobs stack, and
  after switching to another space and back, that space's panel is as it
  was left (the shipped `tabs` would have closed it).
- With a tower server running: the Agents panel lists every agent
  `tower ps` shows, grouped by stack, with live states; the Jobs panel's
  QUEUE matches `tower task list`, and SCHEDULES matches
  `tower schedule list`; a job's detail trail matches `tower task show`,
  and a schedule's RUNS match `tower schedule show`; Edit → Save on a
  queued job → `tower task show` has the new fields and an `edited:` trail
  line; the Events panel opens on the newest events at
  once, follows live within ~6s, and `Load older` adds the page before
  the oldest row.
- The Workflows panel lists what `tower workflow list` shows; a
  workflow's graph has one row per node, branches indented with their
  outcome, loops as `↺` rows; `Run…` with its inputs starts a run that
  `tower run list` shows, and the graph follows it to completion (marks
  and summaries match `tower run show`); a workflow made with
  `＋ New workflow…` matches its TOML in `tower workflow show`, and
  `Edit` → Save of a changed source makes `v2`.
- Prompt an idle agent from its detail section → the agent's terminal
  shows the turn, and the section's screen shows it within ~2s;
  `tower prompt <name>` shows the same delivered message.
- A pending question appears in the Inbox within ~10s; Answer it in
  place → `tower inbox` no longer lists it, and `tower inbox --json`'s
  sender sees `responded_at`.
- Queue a job from the form → `tower task list` shows it; Assign it to an
  agent → its state moves to assigned, the owner's name shows in the row.
  `repeats` with an owner, `daily at` and a pre-run → `tower schedule
  show` has that cron, timezone, target and pre-run.
- `a workflow run` with its inputs and `now` → `tower run list` shows the
  run; with `repeats` → `tower workflow show` has a new version with the
  schedule trigger and `tower schedule list` its row. A job tagged like a
  workflow's `task.completed` trigger shows "Completing it starts
  workflow X", and completing it starts that run.
- `tower ui`'s web inbox and this panel agree on what needs you.
- With a machine added (`hc machine ls`) that runs its own tower: select
  a space on it → within ~1s the tab bar's machine button and every
  panel header name that machine, and the panels match `hc exec --on
  <machine> tower ps` / `task list` / `inbox`; the Events panel shows that
  server's events. Pin "this computer" from the machine button → the
  panels show the local fleet while the remote space stays shown, and a
  dot marks the pin; "Follow space" undoes it.
- `cmd-shift-right` four times from Agents → Workflows, Jobs, Events,
  Inbox; once more → Agents. `cmd-shift-left` from Agents → Inbox. With
  the panel closed, `cmd-shift-right` opens Agents. A key already bound in
  the person's `keys.json` before install still does what it did.