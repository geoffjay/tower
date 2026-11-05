# tower app package

A [Herdr app](https://herdr.dev) layout + mods that make a window of it a
tower control deck: the fleet, workflows, the job queue, the operator
inbox, and the live event log as native panels, driven through the
`tower` CLI.

This folder is the shareable artifact in the herdr.dev convention
([`RECIPE.md`](RECIPE.md) is the build brief; the files are one build of
it):

```text
app/
├── layouts/tower.json     the window: center terminals + tower panel stack
└── mods/
    ├── tower-agents/      fleet: rows per agent, detail with prompt/actions/screen, spawn
    ├── tower-workflows/   workflows: list, top-down graph, runs over the graph, run/edit/create
    ├── tower-inbox/       pending questions/approvals, answer/approve/deny
    ├── tower-jobs/        queue, schedules, job/schedule detail and edit, create/assign/cancel/release
    ├── tower-events/      newest events first, then /v1/events live (bounded SSE polls)
    └── tower-tabs/        the tab bar, with Agents/Workflows/Jobs/Events/Inbox panel buttons
```

Each mod folder carries an identical `tower.js` — the Herdr mod runtime
forbids imports across mod folders, so the shared CLI wrapper is copied
per mod (verified identical by hash).

## Install

With the Herdr app installed and a tower server running (`tower serve` or
`tower service install`):

### Using an Agent

Tell your agent:

```
```
Read ~/.config/herdr-app/AGENTS.md, then add this mod: https://github.com/geoffjay/tower/tree/main/app
```
```

### During Development

```sh
cp app/layouts/tower.json ~/.config/herdr-app/layouts/
cp -R app/mods/tower-{agents,workflows,inbox,jobs,events,tabs} ~/.config/herdr-app/mods/
herdr-app do layout.switch --name tower
```

Or ask your agent: "add the tower layout from this repo's `app/` folder"
— it follows [`RECIPE.md`](RECIPE.md).

The panels need the `tower` CLI on PATH. They find the server exactly as
the CLI does. The events panel reads the token file directly from the
tower home for its platform — `~/.local/share/tower` (Linux),
`~/Library/Application Support/tower` (macOS) — or a path set as shared
state `tower.token_file`; its server address honors `tower.server_url`
the same way (mods cannot read the environment).

## What it does

- **Agents** (right panel): every seat grouped by stack with live state
  dots and the job each owns. Click an agent for a detail section: prompt
  input, interrupt / stop / remove (two presses) / resume, its facts, and
  its terminal screen. Spawn form with the harnesses installed on the
  machine.
- **Workflows** (right panel): every workflow with its version, triggers
  and running runs. Click one for its graph as a top-down list of nodes:
  a chain stays in line, branches indent under the node they leave with
  the outcome that takes them, a loop is a `↺` row, a join shows after
  its branches. Pick a run and each node shows its latest step (done,
  failed, running, not reached) and summary; click a node for every
  visit. Run with the declared inputs, enable/disable, remove, edit the
  TOML source, cancel a run. `＋ New workflow…` builds one from a form of
  steps (agent, prompt, outcomes, where each outcome goes) or from TOML.
  Needs a tower 0.7.0 server.
- **Inbox** (right panel): questions and approvals addressed to you.
  Answer in place, approve/deny with a confirm press (an approval lets an
  agent act), dismiss stale ones, peek at the asking agent's screen.
- **Jobs** (right panel): open jobs with owner/priority/age, the last ten
  to close, schedules with next runs. Queue a job, assign it to an agent
  (pills), release or cancel owned jobs, pause/resume/run schedules. Click
  a job for its facts, prompts, result, trail and messages, or a schedule
  for its cadence, prompts and runs (a run opens as a job). Edit either
  in place.
- **Events** (right panel): the tower event log, newest 50 first, then
  followed live, filtered by type, in names rather than ids; "Load older"
  pages back. Click an event for every field and its full body (an
  agent's screen, a job's result). Agent screen snapshots stay under
  their own filter. Needs a tower 0.6.0 server.

Center and tab bar are the shipped Herdr mods unchanged — the deck sits
around the terminals you already work in.

## Verification

`herdr-app check` must pass per mod (the gate the Herdr docs require);
done for all six as of writing. Live behavior was verified against a
tower 0.4.0 server: fleet state transitions, queue create/assign/
complete, inbox question + approval round-trips, event replay and
follow. See [`RECIPE.md`](RECIPE.md) "Done when" for the re-check steps.

## Limits

- The events panel talks to the default bind `127.0.0.1:8266`; set the
  shared state `tower.server_url` for a non-default server, and
  `tower.token_file` for a non-default token path (mods cannot read
  `TOWER_URL`/`TOWER_TOKEN`). The other panels go through the CLI and
  honor the environment.
- Editing jobs and schedules needs a tower 0.5 server (`task edit`,
  `schedule edit`), and the Workflows panel a tower 0.7.0 one; the other
  verbs the panels use exist on 0.4.
- `herdr-app shot` is not implemented on Linux in herdr-app 0.1.12, so
  pixel verification is manual there.
