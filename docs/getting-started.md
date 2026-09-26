# tower — Getting Started (target-state walkthrough)

> **Status: this is the UX contract, not a working product.** The commands
> and output below describe what tower *will* look like when phase 1–2 of
> the [execution plans](../plans/) are complete. Nothing here runs yet.
> The CLI verbs shown are the target surface — implementation plans
> ([plans/README.md](../plans/README.md)) treat this document as the
> reference for CLI behavior and output shape.

Everything happens through one binary: `tower`.

```
tower <verb> [args]          # every verb supports --json
tower serve                  # the server (runs under systemd)
tower tui                    # the TUI (later phase)
```

One server, one database, one socket. Agents live in herdr panes; tower
never owns terminals directly — you can always drop into `herdr` or
`tmux attach` to see exactly what an agent sees.

---

## 1. Install and start an instance

```console
$ cargo install tower            # or: pacman -S tower, or download binary
$ tower --version
tower 0.1.0

$ tower serve                    # foreground, or:
$ systemctl --user enable --now tower
```

First start creates everything it needs and prints the important paths:

```
● tower server 0.1.0
  socket    /run/user/1000/tower.sock     (clients use this by default)
  tcp       127.0.0.1:8266                  (token required)
  database  ~/.local/share/tower/tower.db
  token     ~/.local/share/tower/token    (generated, 0600)
  config    ~/.config/tower/config.toml   (all keys optional)
```

No configuration file is required to start. The CLI finds the server via the
unix socket automatically.

## 2. Check your machine

```console
$ tower doctor
herdr socket      ok    ~/.config/herdr/herdr.sock (protocol 20)
herdr cli         ok    herdr 0.8.2
claude            ok    ~/.local/bin/claude
pi                ok    ~/.local/bin/pi (0.87.1)
database          ok    ~/.local/share/tower/tower.db (writable)
event log         ok    0 events
machine local     ok    role=coordinator

7 checks passed
```

If herdr isn't running yet, start it first (`herdr` in another terminal, or
its service unit). tower drives agents through herdr — that's where the
panes, detection, and persistence come from.

## 3. First look

```console
$ tower ps
NAME   KIND    MACHINE  STATE   TASK          NOTE
─      ─       ─        ─       ─             ─
(1 agent adopted from herdr, unowned)

$ tower ps --json
{
  "agents": [
    {"id": "01J9X0...", "name": "research", "kind": "pi",
     "machine": "local", "state": "idle", "adopted": true}
  ]
}
```

On first contact tower snapshots herdr and **adopts** agents it finds
already running — you don't have to relaunch anything to bring an instance
under management. `adopted: true` rows are unowned until you take them.

```console
$ tower spawn research --adopt     # take ownership of the existing agent
$ tower prompt research 'summarize what you are working on'
```

## 4. Spawn a new agent

Agents are rows, not config files. You create them with one command; flags
are the configuration:

```console
$ tower spawn writer --kind pi --workdir ~/Projects/example --worktree
spawned writer
  id       01J9X4Z...
  kind     pi              (detection: herdr manifest "pi")
  machine  local
  workdir  ~/Projects/example
  worktree ~/Projects/example/.worktrees/writer   (isolated branch)
  state    launching → idle (2.1s)

$ tower spawn backend --kind claude --workdir ~/Projects/example --worktree
spawned backend
  id       01J9X52...
  kind     claude
  ...
  state    launching → idle (4.4s)

$ tower ps
NAME     KIND     MACHINE  STATE  TASK  NOTE
research pi       local    idle   —     adopted
writer   pi       local    idle   —     worktree
backend  claude   local    idle   —     worktree
```

Useful spawn flags:

| Flag | Meaning |
|---|---|
| `--kind pi` / `--kind claude` | harness (any herdr-detected kind works) |
| `--workdir DIR` | where the agent runs |
| `--worktree` | isolated git worktree per agent (recommended for coders) |
| `--permissions yolo` | explicit opt-in to full-bypass mode (recorded in events) |
| `--machine NAME` | spawn on a remote machine (phase 5) |
| `--prompt '...'` | first prompt, sent as soon as the agent settles |

What you configured at spawn can be inspected any time:

```console
$ tower ps --json | jq '.agents[] | {name, kind, worktree, permissions}'
```

There is no agent YAML in v1. If you want repeatable fleets, that's a shell
script — or spawn from code over `/v1` / MCP. (Declarative fleet files are a
backlog item; see plans.)

## 5. Talk to agents

```console
$ tower prompt writer 'draft a short README for this project' --wait
prompt delivered; settled in working

$ tower stream writer          # live output, Ctrl-C to detach
● Reading the existing files first…
+ Read src/main.rs
  Thought 412ms
⠋ Drafting… (working · 1m 12s · esc to interrupt)
```

The stream is the same SSE feed every other client (TUI, web UI) consumes.

```console
$ tower read backend --source visible --format ansi | less -R
$ tower prompt backend 'stop what you are doing'   # also: interrupt
$ tower stop writer           # stop the session; the agent row survives
$ tower spawn writer --adopt  # a "seat": same name, same identity, new session
```

## 6. Questions and approvals find you

Agents block; tower routes the blocking to an inbox instead of making you
watch terminals:

```console
$ tower ps
NAME     KIND     MACHINE  STATE    TASK          NOTE
backend  claude   local    blocked  t_01J9X7Q     awaiting approval
research pi       local    working  t_01J9X6B

$ tower inbox
ID            FROM      KIND       AGE   SUMMARY
m_01J9X7R     backend   approval   40s   Allow `cargo publish` for this crate?
  └─ context: "Bash command: cargo publish --dry-run"

$ tower approve m_01J9X7R            # or: tower approve m_01J9X7R --deny
approved → delivered to backend

$ tower ps
NAME     KIND     MACHINE  STATE    TASK          NOTE
backend  claude   local    working  t_01J9X7Q
```

Questions work the same way: `tower ask <name> 'which approach?'`, reply
from the inbox. Unanswered items expire on their deadline (default 5 min) and
the agent is told to proceed with its fallback.

## 7. The shared task pool

Agents don't just wait for you — they pull work. Create tasks, agents claim
them exclusively:

```console
$ tower task create 'implement CSV error column names' \
    --tag backend --tag rust --priority 2
t_01J9X8A  queued

$ tower task create 'write launch blog post' --tag writing
t_01J9X8B  queued

$ tower task list
ID         STATE     OWNER     PRIORITY  TAGS          ATTEMPTS  TITLE
t_01J9X8A  working   backend         2    backend,rust  0/3       implement CSV error…
t_01J9X8B  queued    —               0    writing       0/3       write launch blog…

$ tower task show t_01J9X8A
task t_01J9X8A  implement CSV error column names
  state    working (owner: backend, lease renews in 18s)
  created  2026-09-26 10:12:03 by me
  trail
    10:12:04  claimed by backend
    10:12:09  status: working — "reading csv module"
    10:13:41  status: input-required — question m_01J9X7K to me
```

How does the agent know to pull work? Agents run the **work loop** — a
documented contract they follow via MCP tools (`tower_task_pull`,
heartbeat, status, complete — see `docs/agent-loop.md` in the phase-2
plan). Each agent's prompt tells it to keep pulling tasks matching its
tags. Ownership is exclusive: if two agents race for one task, exactly one
wins. If an owner dies, its lease expires and the task requeues itself —
attempts are counted (`0/3`) so stuck work fails loudly instead of looping
forever.

```console
$ tower task cancel t_01J9X8B
$ tower task release t_01J9X8A       # voluntary give-back to the pool
```

## 8. Watch the system

```console
$ tower events --follow --filter task
10:12:03 task.created    t_01J9X8A  by me
10:12:04 task.claimed    t_01J9X8A  backend (lease 60s)
10:12:09 task.status     t_01J9X8A  working
10:12:11 agent.state     backend    idle → working
...

$ tower schema            # every route and event type, introspectable
```

The web UI (phase 4) is the **agent cloud** — a single highly graphical page:
every agent a colored point in a drifting cloud (state = color, activity =
size, attention = pulsing halo, health = brightness), with a floating panel
for the agent you click. Read-only by design: it answers "who needs me right
now?" at a glance; configuration and control stay in the CLI and TUI.

## 9. Later: more machines

```console
$ tower machines add workbox
node token (shown once):
  tower node --coordinator wss://main:8266/nodes --token eyJ…

# on the other machine:
$ tower node --coordinator wss://main:8266/nodes --token eyJ…
registered workbox

$ tower spawn indexer --kind pi --machine workbox --workdir ~/data
$ tower ps
NAME      KIND     MACHINE   STATE  TASK  NOTE
research  pi       local     idle   —     adopted
backend   claude   local     idle   —     worktree
indexer   pi       workbox   idle   —
```

One database, one coordinator; tasks are claimable across machines. If
`workbox` goes dark, its agents keep running (herdr owns them), its task
leases expire, and the pool reabsorbs the work.

---

## Command summary

| Command | Purpose |
|---|---|
| `tower serve` / `tower node` | run the server / a remote-machine agent |
| `tower doctor` | preflight: herdr, harnesses, db, socket |
| `tower ps [--json]` | list agents with live states |
| `tower spawn NAME --kind K [--workdir D] [--worktree] [--adopt]` | create/adopt an agent |
| `tower prompt NAME 'text' [--wait]` | send work |
| `tower stream NAME` / `tower read NAME` | live output / snapshot read |
| `tower interrupt NAME` / `tower stop NAME` | ctrl+c / end session (seat survives) |
| `tower inbox` / `tower ask` / `tower approve ID` | human ↔ agent loop |
| `tower task create/list/show/cancel/release` | shared pool |
| `tower events --follow [--filter ...]` | system event stream |
| `tower machines add/remove/list` | node registry |
| `tower schema` | route + event-type registry |
| `tower tui` | the TUI (phase 3) |
| browser → `/ui` | the agent cloud (phase 4, read-only) |

Everything above also exists as `--json` for scripting, as REST under
`/v1` for tools, and as MCP tools for the agents themselves.