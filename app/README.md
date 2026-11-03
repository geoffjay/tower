# tower app package

A [Herdr app](https://herdr.dev) layout + mods that make a window of it a
tower control deck: the fleet, the job queue, the operator inbox, and the
live event log as native panels, driven through the `tower` CLI.

This folder is the shareable artifact in the herdr.dev convention
([`RECIPE.md`](RECIPE.md) is the build brief; the files are one build of
it):

```text
app/
├── layouts/tower.json     the window: tower sidebar + panel stack + center
└── mods/
    ├── tower-agents/      fleet: rows per agent, prompt/interrupt/stop, spawn
    ├── tower-inbox/       pending questions/approvals, answer/approve/deny
    ├── tower-jobs/        queue, schedules, create/assign/cancel/release
    ├── tower-events/      /v1/events followed live (bounded SSE polls)
    └── tower-tabs/        the tab bar, with Jobs/Events panel buttons
```

Each mod folder carries an identical `tower.js` — the Herdr mod runtime
forbids imports across mod folders, so the shared CLI wrapper is copied
per mod (verified identical by hash).

## Install

With the Herdr app installed and a tower server running (`tower serve` or
`tower service install`):

```sh
cp app/layouts/tower.json ~/.config/herdr-app/layouts/
cp -R app/mods/tower-{agents,inbox,jobs,events,tabs} ~/.config/herdr-app/mods/
herdr-app do layout.switch --name tower
```

Or ask your agent: "add the tower layout from this repo's `app/` folder"
— it follows [`RECIPE.md`](RECIPE.md).

The panels need the `tower` CLI on PATH. They find the server exactly as
the CLI does (`TOWER_URL`/`TOWER_TOKEN` env or the tower home's
`config.toml`/`token`); the events panel additionally reads the default
token file directly.

## What it does

- **Agents** (sidebar, top): every seat grouped by stack with live state
  dots. Inline prompt bar per agent, interrupt, stop (two presses), and a
  spawn form with the harnesses installed on the machine.
- **Inbox** (sidebar, bottom): questions and approvals addressed to you.
  Answer in place, approve/deny with a confirm press (an approval lets an
  agent act), dismiss stale ones, peek at the asking agent's screen.
- **Jobs** (right panel): open jobs with owner/priority/age, closed ones
  below, schedules with next runs. Queue a job, assign to an agent
  (pills), release or cancel owned jobs, pause/resume/run schedules.
- **Events** (right panel): the tower event log followed live, filtered
  by type. Cursor replay makes the bounded polls lossless.

Center and tab bar are the shipped Herdr mods unchanged — the deck sits
around the terminals you already work in.

## Verification

`herdr-app check` must pass per mod (the gate the Herdr docs require);
done for all five as of writing. Live behavior was verified against a
tower 0.4.0 server: fleet state transitions, queue create/assign/
complete, inbox question + approval round-trips, event replay and
follow. See [`RECIPE.md`](RECIPE.md) "Done when" for the re-check steps.

## Limits

- The events panel polls `127.0.0.1:8266` (the tower default): the mod
  runtime cannot read `TOWER_URL` overrides. The other panels go through
  the CLI and honor them.
- No `task edit`/`schedule edit` (needs a tower 0.5 server); the verbs
  the panels do use exist on 0.4.
- `herdr-app shot` is not implemented on Linux in herdr-app 0.1.12, so
  pixel verification is manual there.