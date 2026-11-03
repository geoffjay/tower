# tower

A tower control deck for the Herdr app: the fleet in the sidebar, the job
queue and the live event log beside the terminals. Everything the `tower`
CLI can show, as panels that act — prompt an agent, answer an inbox item,
queue a job, pause a schedule — while the terminals you're already in stay
in the middle.

## Before you add it

The panels talk to a running tower server through the `tower` CLI
(`tower --version` to check). The CLI finds the server the way the TUI
does: `TOWER_URL`/`TOWER_TOKEN`, else the tower home's `config.toml` and
token file. No tower, and every panel says so.

## What you see

A clone of the shipped `main` layout with tower panels in place of the
spaces/agents sidebar and the Files/Changes panel:

- **Sidebar** (300px, chrome): the brand block, then **Agents** (top,
  weighted 3) and **Inbox** (bottom, weighted 2).
  - **Agents**: every agent seat grouped by stack (agents named
    `<stack>-<role>`), each row a state dot (theme colors: `working`,
    `blocked`, `done`, `idle`; dead red `ansi1`), the name, the state word,
    and the workdir; hover shows Prompt / Interrupt / Stop buttons per
    agent. Prompt opens an inline input on the row; Enter or Send runs
    `tower prompt <name> <text>`. Stop needs a second press. The footer's
    `＋ Spawn…` opens a form: name, harness pills (installed kinds, via
    `knownAgents()`), workdir, first prompt → `tower spawn`.
  - **Inbox**: pending questions/approvals addressed to you. Each item:
    kind dot (approvals `blocked` color, questions `accent`), sender, kind,
    deadline ("4m left", red when passed), text, then buttons: `Peek agent`
    (the sender's recent screen via `tower read`), `Approve?`/`Deny`
    (two-press confirm — an approval lets an agent act), `Answer…` (inline
    input, Enter sends `tower approve <id> --answer`), `Dismiss`.
- **Right panel** (380px), opened from the tab bar's Jobs / Events
  buttons and remembered per space: a stack of:
  - **Jobs**: QUEUE section (open jobs: state dot, title, `P<n>` priority,
    state, owner/hold/tags/age), RECENT (last 10 closed), SCHEDULES
    (title, cron, next run, target). Per-job buttons: `Assign…` (pills of
    live agents), `Release` (owned jobs), `Cancel` (two presses; interrupts
    the owner). Per-schedule: `Run now`, `Pause`/`Resume`, `Remove`
    (two-press). Footer `＋ Queue a job…`: title, description, priority
    pills, owner pills (queue or an agent, with a "deliver when free"
    reserve checkbox).
  - **Events**: the `/v1/events` feed, newest first: time, event kind
    (colored: failures red), one-line summary. Filter pills: all / agents /
    jobs / schedules / messages. Live dot `● live` (dim) or `◌ connecting`
    (red).
- **Center**: the shipped pane cards, unchanged.
- **Tab bar**: a clone of the shipped `tabs` mod (same tabs, drag, `+`,
  window buttons) with Jobs / Events panel buttons in place of Files /
  Changes. The clone is required: the shipped `tabs` polices the panel
  against `shared("workspace.panels")` (files/changes) and would close a
  tower panel on every space switch. The clone's `enabled` list is
  `["tower-jobs", "tower-events"]`, so its per-space memory restores a
  space's last tower panel instead.

## How it behaves

- Every panel polls the `tower` CLI every 10s (events: bounded SSE polls
  every 6s). A panel action refreshes immediately after its command
  returns.
- Actions that stop work or let an agent act (Stop, Cancel, Approve,
  Deny, Remove) ask once with a second press.
- Errors from the server (conflict, not found, unauthorized) show in a
  red strip inside the panel that acted; they clear on the next success.
- The events feed replays the whole log from cursor 0 on load and after a
  filter change, then follows live. Nothing is lost between polls: the
  cursor holds the place, so each bounded poll resumes exactly where the
  last ended.

## Built from

- One layout: `layouts/tower.json`, cloning the shipped `main` structure
  (brand / work / status blocks, shipped `pane` header,
  `workspace-chrome`, `workspace-status` mods reused as-is; the tab bar
  is the `tower-tabs` clone below).
- Five mods, each a folder with `main.js`, `mod.json` (`place: "none"` —
  the layout is the only placement), and a shared `tower.js`:
  - `tower-agents`: `tower ps`, `tower prompt`, `tower interrupt`,
    `tower stop`, `tower spawn`; `knownAgents()` for the spawn form.
  - `tower-inbox`: `tower inbox`, `tower approve [--answer|--deny]`,
    `tower dismiss`, `tower read` for the peek.
  - `tower-jobs`: `tower task list`, `tower task create/assign/release/
    cancel`, `tower schedule list/run/pause/resume/rm`.
  - `tower-events`: the tower REST `/v1/events` SSE endpoint via bounded
    `curl --max-time 5` polls (cursor replay); the token from
    `~/.local/share/tower/token` (`readTextFile`), matching the CLI's
    default resolution.
  - `tower-tabs`: a clone of the shipped `tabs` mod, `enabled` hardcoded
    to `["tower-jobs", "tower-events"]`, panel buttons switched to those
    two (icons `mods/tower-tabs/jobs.svg`, `events.svg`). Everything else
    is the shipped tab bar: tab pills, drag-to-reorder, `+` menu, window
    buttons.
- `herdr`: `exec`, `theme`, `readTextFile`, `knownAgents`, `layout`, `run`.
  `gpui-base`: `v_flex`, `h_flex`, `InputState`, `Input`. `gpui-kit`:
  `View`, `div`.
- The mods run programs (`tower`, `curl`) and read one file outside their
  folders (the tower token file). They reach no network except the local
  tower server.

## Watch out

- Mods cannot import across folders ("resolves outside the application
  directory"): each mod carries its own copy of `tower.js` (identical
  md5 across the set).
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
- The events endpoint is SSE-only in tower 0.4; there is no `tower events`
  CLI verb yet. The bounded-poll approach assumes the server keeps the
  default `127.0.0.1:8266` bind (mods cannot read the tower config).
- `InputState.new` belongs in handlers, never in `render`.
- Two-press confirms reset on the next refresh cycle if the row changed.

## Done when

- `herdr-app check` passes for each mod folder. The tab bar shows Jobs /
  Events buttons; clicking Jobs opens the panel on the Jobs stack, and
  after switching to another space and back, that space's panel is as it
  was left (the shipped `tabs` would have closed it).
- With a tower server running: the Agents panel lists every agent
  `tower ps` shows, grouped by stack, with live states; the Jobs panel's
  QUEUE matches `tower task list`, and SCHEDULES matches
  `tower schedule list`; the Events panel shows the same log the TUI's
  events view does, following live within ~6s.
- Prompt an idle agent from its row → the agent's terminal shows the turn;
  `tower prompt <name>` shows the same delivered message.
- A pending question appears in the Inbox within ~10s; Answer it in
  place → `tower inbox` no longer lists it, and `tower inbox --json`'s
  sender sees `responded_at`.
- Queue a job from the form → `tower task list` shows it; Assign it to an
  agent → its state moves to assigned, the owner's name shows in the row.
- `tower ui`'s web inbox and this panel agree on what needs you.