# tower agent

An [oh-my-pi](https://omp.sh) (omp) agent that is your interface to tower.
Ask it what the fleet is doing, triage the inbox, queue work, spawn agents,
or deploy a stack — it knows tower's model and has the operations as tools,
so a fresh session acts without reading docs first.

It is plain omp plus a set of extensions in this folder, started by
[`tower-agent`](tower-agent). Nothing here touches the tower Rust code; the
extensions talk to the tower server's REST API (D§7) the way the `tower` CLI
does.

## Setup

Requirements:

- omp on `PATH` (`omp --version`; built and tested with 18.8.7), logged in to
  a model provider
- a running tower server (`tower serve`, or `tower service install`) —
  `tower doctor` checks it
- the `tower` CLI on `PATH` for spawning agents from named definitions
  (everything else goes over REST)

Run it from anywhere:

```console
$ /path/to/tower/agent/tower-agent
```

or put it on your `PATH`:

```console
$ ln -s /path/to/tower/agent/tower-agent ~/.local/bin/tower-agent
$ tower-agent
$ tower-agent --continue          # resume the last operator conversation
$ tower-agent --model anthropic/claude-sonnet-5-5 'what needs me?'
```

Every argument passes through to omp. The script adds:

| Flag | Why |
|---|---|
| `--extension <this folder>` | loads both extensions (`package.json` → `omp.extensions`) and the bundled skills in `skills/` |
| `--config config.yml` | the agent's settings overlay: turns off omp's startup banner so the tower welcome card replaces it |
| `--session-dir ~/.local/state/tower-agent/sessions` | operator conversations live in one place whatever directory you start in |

The agent finds the server exactly like the `tower` CLI: `TOWER_URL`, else
`[server] bind_tcp` from the tower home's `config.toml`, else
`127.0.0.1:8266`; the token from `TOWER_TOKEN`, else the token file in the
tower home (`TOWER_HOME`, else `~/Library/Application Support/tower` on macOS
and `~/.local/share/tower` on Linux).

| Variable | Effect |
|---|---|
| `TOWER_HOME`, `TOWER_URL`, `TOWER_TOKEN` | which server, as for the CLI |
| `TOWER_BIN` | the `tower` binary used for `spawn` from a definition (default `tower`) |
| `TOWER_AGENT_SESSION_DIR` | where the launcher keeps sessions |
| `OMP_BIN` | the omp binary the launcher runs (default `omp`) |

## What it offers

### A tower-aware model

The `tower-tools` extension adds a tower operator block to the system prompt:
the objects and their states (agents, jobs, leases, reservations, lifecycle
prompts, schedules, the inbox, stacks), which tool to reach for, and the
operating rules — report what needs you first, never answer an inbox item for
you, confirm before anything that stops work, never put secrets in briefs.

The tools are top-level (not behind omp's tool discovery):

| Tool | Actions |
|---|---|
| `tower_status` | fleet overview: what needs you, agents, open jobs, schedules, last 24h; optional `stack` filter |
| `tower_agent` | `list` `show` `read` (screen) `prompt` `interrupt` `spawn` `stop` `remove` |
| `tower_inbox` | `list` (with the blocked agent's screen) `answer` `approve` `deny` `dismiss` `ask` |
| `tower_task` | jobs: `list` `show` `create` `assign` `edit` `cancel` `release` |
| `tower_schedule` | `list` `show` `create` `edit` `pause` `resume` `run` `remove` |

Reads run without a prompt. Changes are `write`-tier, so your omp approval
mode (`tools.approvalMode`) decides whether they ask first; omp's default,
`yolo`, runs them without asking. Two actions always ask, whatever the mode:
`tower_agent remove` (deletes the agent and its worktree) and
`tower_inbox approve` (lets an agent act).

The tower operator skills ship with the agent through `skills/` (links to
`.agents/skills/`): `tower-status`, `tower-queue-job`, `tower-agent-add`,
`tower-agent-remove`, and `tower-deploy`. They hold the multi-step procedures;
the agent runs their steps with the tools.

### An operator console

The `tower-console` extension changes what you see and where:

- **Welcome card** — at session start, a fleet card takes the place of omp's
  banner: what needs you, agents grouped by stack with state glyphs (`●`
  working, `◉` blocked, `○` idle, `◌` launching, `✕` dead), open jobs, the
  next schedules, and the last 24h. The same card is the model's first
  snapshot of the fleet, so the first answer needs no tool call.
- **Live strip** above the editor — stacks and agent states, inbox count, the
  job queue, and the most urgent item, kept current from the tower event
  stream (`● live`, `◌ reconnecting`, `✕ offline`).
- **Toasts** when an inbox item arrives, an agent dies, or a job fails. The
  terminal title reads `tower ⚠N` while N things wait on you (inbox items,
  blocked or dead agents).
- **Activity digest** — what changed since the model's last turn (inbox
  items, agents blocked or dead, jobs created, assigned, completed, failed,
  requeued) rides along with your next message, so the model stays current
  in a long-running session.

Slash commands act at once, with no model turn:

| Command | What it does |
|---|---|
| `/fleet [stack]` | the fleet card, in the transcript (an overlay while a turn runs) |
| `/inbox` | walk pending items: answer, approve, deny, dismiss, or peek at the agent's screen |
| `/peek [agent]` | the agent's recent screen in an overlay; completes agent names |

What you do in `/inbox` is noted in the transcript, so the model knows.

## Using the extensions in your own omp

The extensions also load into an ordinary omp session:

```console
$ omp -e /path/to/tower/agent                         # everything: tools, console, skills
$ omp -e /path/to/tower/agent/extensions/tower-tools  # only the tools and the operator prompt
```

or permanently, in `~/.omp/agent/config.yml`:

```yaml
extensions:
  - /path/to/tower/agent/extensions/tower-tools
```

`tower-tools` alone suits a coding session that should queue work or check
the fleet without the console's welcome card and strip.

## Layout

```text
agent/
├── tower-agent            launcher
├── config.yml             omp settings overlay (startup.quiet)
├── package.json           omp package manifest; dev dependencies for type checking
├── skills/                links to the tower operator skills
└── extensions/
    ├── lib/               REST + SSE client, models, snapshot and fleet view, text and TUI rendering
    ├── tower-tools/       the five tools and the operator system prompt
    └── tower-console/     welcome card, live strip, toasts, digest, slash commands
```

Only the main session runs the console and carries the operator prompt;
subagents that omp lets use extension tools see the tools alone.

## Development

omp loads the TypeScript directly; there is no build step. Restart the agent
to pick up changes. Load failures and extension logs go to
`~/.omp/logs/omp.<date>.*.log`.

Type checking needs the omp SDK types (a large install, about 1 GB):

```console
$ cd agent
$ npm install
$ npm run check
$ biome check .    # the repo's formatter and linter (see biome.json)
```

Keep `@oh-my-pi/pi-coding-agent` in `devDependencies` at the omp version you
run. At runtime omp maps every `@oh-my-pi/*` import to its own copy.

## Notes and limits

- omp 18.8's interactive mode ignores extension headers and footers
  (`setHeader`, `setFooter` are no-ops), so the welcome card lives in the
  transcript and the live view is a widget above the editor.
- The REST caller is always the operator. `TOWER_AGENT` in the environment
  does not change what the tools may do.
- `tower_task edit` and `tower_schedule edit` need a tower 0.5 server; an
  older server answers that the route is not supported.
- The event stream carries every agent's output; the console drops output
  chunks unparsed and refreshes its snapshot only on fleet changes. A slow
  periodic refresh (60s) repairs anything the stream missed.

## Where this goes next

The folder is a staging area for the agent's shape. A `tower` CLI subcommand
is the intended way to start it, and the extensions may move to their own
repository.
