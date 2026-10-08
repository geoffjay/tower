# tower — Getting Started (target-state walkthrough)

> **Status: this is the UX contract, not a working product.** The commands
> and output below describe what tower *will* look like when phase 1–2 of
> the [execution plans](knowledgebase/plans/index.md) are complete. Nothing here runs yet.
> The CLI verbs shown are the target surface — implementation plans
> ([plans overview](knowledgebase/plans/overview.md)) treat this document as the
> reference for CLI behavior and output shape.

Everything happens through one binary: `tower`.

```
tower <verb> [args]          # every verb supports --json
tower serve                  # the server (runs under systemd)
tower tui                    # the TUI: fleet, inbox, jobs, events, machines
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
| `--name foo` | spawn from the definition `~/.config/tower/agents/foo/` |

What you configured at spawn can be inspected any time:

```console
$ tower ps --json | jq '.agents[] | {name, kind, worktree, permissions}'
```

There is no agent YAML in v1 — except named definitions. Put a folder under
`~/.config/tower/agents/<name>/` with a `PROMPT.md` (first prompt) and a
`config.toml` (`kind = "pi"`, `workdir = "..."`, `worktree = true`); then:

```console
$ tower spawn --name foo   # definition expands: prompt sent, kind applied
```

Explicit flags override the definition, so a definition plus `--workdir`
still works. For anything beyond that, repeatable fleets are a shell
script, spawn from code over `/v1` / MCP, or let an agent build one with
`/tower-deploy` (§10). (Declarative fleet files are a backlog item; see plans.)

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

## 7. The job queue

Agents don't pick their own work — you assign it. Jobs wait in a queue
until dispatched to an agent, which then owns them exclusively:

```console
$ tower task create 'implement CSV error column names' \
    --tag backend --tag rust --priority 2
t_01J9X8A  queued

$ tower task create 'write launch blog post' --tag writing
t_01J9X8B  queued

$ tower task assign t_01J9X8A backend
t_01J9X8A  assigned to backend (lease 60s)

$ tower task list
ID         STATE     OWNER     PRIORITY  TAGS          ATTEMPTS  TITLE
t_01J9X8A  working   backend         2    backend,rust  0/3       implement CSV error…
t_01J9X8B  queued    —               0    writing       0/3       write launch blog…

$ tower task show t_01J9X8A
task t_01J9X8A  implement CSV error column names
  state    working (owner: backend, lease expires in 42s)
  attempts 0/3
  created  2026-09-26 10:12:03
  trail
    10:12:03  created
    10:12:04  assigned to backend by me
    10:12:09  working (backend)
    10:13:41  status: input-required
```

How does the agent know it has work? Assignment sends it a **delegation**
message (a prompt naming the job), the job appears in its MCP view
(`tower_task_list --mine`), and the agent runs the **work loop** — a
documented contract (`tower contract` prints it anywhere — it's
`docs/agent-loop.md`, embedded in the binary): declare start → heartbeat →
report status → complete. Agents never pull or claim: if two dispatchers
race to assign one job, exactly one wins and the loser gets a clean
conflict naming the owner. If an owner dies, its lease expires and the job
requeues itself — attempts are counted (`0/3`) so stuck work fails loudly
instead of looping forever. An owner blocked on your answer
(`input-required`) never loses its job to lease expiry.

```console
$ tower task cancel t_01J9X8B        # also interrupts the owner
$ tower task release t_01J9X8A       # give it back to the queue (acts as the owner)
```

Every command above takes `--json` to print the raw API response.

### Scheduled and recurring jobs

An agent owns **one job at a time**; assigning a second while it's busy is
a `conflict`. To line work up for a busy agent, **reserve** it — tower
delivers it the moment the agent is free (the lease starts then, not now):

```console
$ tower task assign t_01J9X8C backend --when-available
t_01J9X8C  reserved for backend (delivered when available)

$ tower task create 'nightly cleanup' --assign backend --at 22:00
t_01J9X8D  reserved for backend (not before 2026-09-28 22:00:00)
```

Recurring work is a **schedule**: a job template that fires on a cron
cadence and creates an ordinary job each time.

```console
$ tower schedule create 'dependency audit' --daily 09:00 --assign backend
s_01J9X9A  daily 09:00 America/Los_Angeles → backend  next: 2026-09-28 09:00:00

$ tower schedule list
ID         CADENCE      ZONE                 TARGET   NEXT                 LAST  TITLE
s_01J9X9A  daily 09:00  America/Los_Angeles  backend  2026-09-28 09:00:00  —     dependency audit

$ tower schedule show s_01J9X9A        # cadence, next/last run, recent jobs

$ tower schedule run s_01J9X9A        # one extra run now
$ tower schedule pause s_01J9X9A      # resume picks up from now; rm deletes
```

What happens when things don't go to plan is fixed policy
([decision](knowledgebase/decisions/scheduled-jobs.md)): if yesterday's run
is still being worked, today's is **skipped**; if it was never picked up,
it's **replaced** (at most one pending run per schedule); if tower was down
across several firings, it fires **once**; removing the target agent
**pauses** the schedule. Schedules only fire while `tower serve` runs —
`tower service install` keeps it running as a user service.

## 8. Watch the system

```console
$ tower events --follow --filter task
10:12:03 task.created    t_01J9X8A  by me
10:12:04 task.assigned   t_01J9X8A  backend (lease 60s, by me)
10:12:09 task.status     t_01J9X8A  working
10:12:11 agent.state     backend    idle → working
...

$ tower schema            # every route and event type, introspectable
```

Or stay in one place — the TUI follows the same event stream and wraps the
verbs above:

```console
$ tower tui
 tower  1 Fleet  2 Inbox (1)  3 Tasks  4 Events  5 Machines          ● live
1 queued · 1 working · 1 blocked

   NAME      MACHINE  KIND    STATE    TASK                       NOTE
●  api       local    claude  working  implement CSV error colu…
◉  backend   local    claude  blocked  migrate schema             awaiting approval
○  docs      local    pi      idle     —                          adopted · docs
enter open · i prompt · x interrupt · o herdr · f state · m machine · s sort
```

| View | What you do there |
|---|---|
| `1` Fleet | agents with state, job and why they wait; the line under the table shows the selected agent's full name and task; `f`/`m`/`s` filter by state or machine, sort; `enter` opens an agent |
| Agent | its live screen (colors kept), message history; `i` prompt, `x` interrupt, `o` jump to the pane in herdr (focus when the TUI runs inside herdr, else attach; herdr's detach key brings you back) |
| `2` Inbox | questions and approvals with deadlines and the agent's screen; `y`/`n` approve/deny (or yes/no), `r` answer in words |
| `3` Tasks | owned jobs with live lease countdowns, the queue (with reservations), recently closed, schedules; `n` new job, `a` assign (`<name> later` reserves), `x x` cancel, `enter` trail; on a schedule `p` pause/resume, `R` run now |
| `4` Events | the feed; `f` type filter, `s` subject (an agent name works), `O` include output chunks; scrolling pauses, `G` follows |
| `5` Machines | inventory and node status |

`:` opens the command palette (`spawn`, `stop`, `prompt`, `ask`, `task`,
`assign`, `reserve`, `cancel`, `agent`, `filter`, `sort`), `tab` cycles
views, `j`/`k`/`g`/`G` move, `?` lists every key, `q` quits. If the server
restarts, the TUI shows `reconnecting` and resumes from where the stream
left off.

The web UI is the **agent cloud** — a single highly graphical page:
every agent a colored point in a slowly drifting cloud (state = color,
activity = size, attention = pulsing halo, health = brightness). Each
machine is a small central node its agents bind to with spoke lines, so
you can see the grouping at a glance; agent names sit under their points,
truncated if long (the full name is in the hover tooltip and the floating
panel for the agent you click). Read-only by design: it answers "who
needs me right now?" at a glance; configuration and control stay in the
CLI and TUI.

```console
$ tower ui
http://127.0.0.1:8266/ui/login?token=3f9c…
```

Open the link once; the browser keeps a read-only cookie for `/ui` (it
cannot call the API), so later visits go straight to
`http://127.0.0.1:8266/ui`. The page reconnects by itself when the server
restarts.

`Cmd+K` (`Ctrl+K` on Linux) or the `⌘K` button opens a command palette:
type to search, arrows to move, `Enter` to go. Search `settings` for the
settings page, where the theme dropdown switches between Tower Dark (the
default), Tokyo Night Storm and Tokyo Night Light. The choice is kept by
that browser and applies on every page.

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

One database, one coordinator; jobs are assignable across machines. If
`workbox` goes dark, its agents keep running (herdr owns them), their job
leases expire, and the queue reabsorbs the work.

---

## 10. Let an agent run tower

Tower ships agent skills for its own operation, in `.agents/skills/`
(linked from `.claude/skills/`). Run your agent in the tower repo and call
them by name. Each one drives the CLI, shows you its plan, and waits for
your approval before it changes anything.

| Skill | What it does |
|---|---|
| `/tower-agent-add [name] [kind] [role]` | Spawns an agent with a standing brief and checks that it can reach its model; asks whether to save a named definition for easy re-creation |
| `/tower-agent-remove <name>` | Shows the jobs and schedules the removal affects, removes, and offers to reassign |
| `/tower-status [stack]` | Read-only: what needs you first (inbox, blocked, stalled, failed), then the fleet |
| `/tower-deploy [purpose \| update S \| teardown S]` | Interviews you, proposes a stack of agents, schedules, and starting jobs, deploys it, and verifies it |

```console
$ claude
> /tower-deploy a docs stack: one writer, a nightly link check at 02:00
```

A stack is a naming convention, not a tower object: agents are
`<stack>-<role>`, schedules are titled `<stack>: <title>`, and both jobs
and schedules carry the tag `stack:<stack>`. So `tower ps`, `tower schedule
list`, and `tower task list --tag stack:docs` show a stack's state, and
`/tower-deploy update docs` or `teardown docs` can find it again. Keep the
names when you change a stack by hand.

Claude Code's sandbox blocks connections to `127.0.0.1`. The first tower
command in a session fails with `Operation not permitted` until you allow
it to run outside the sandbox.

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
| `tower task create/list/assign/show/cancel/release` | job queue (you assign, agents own) |
| `tower schedule create/list/show/pause/resume/run/rm` | recurring jobs |
| `tower service install/uninstall/status` | keep `tower serve` running |
| `tower events --follow [--filter ...]` | system event stream |
| `tower machines add/remove/list` | node registry |
| `tower schema` | route + event-type registry |
| `tower contract` | print the agent work-loop contract (`docs/agent-loop.md`, embedded in the binary — no repo needed) |
| `tower ui` | print a login link for the agent cloud (read-only, `/ui`) |

Everything above also exists as `--json` for scripting, as REST under
`/v1` for tools, and as MCP tools for the agents themselves —
[wiring an MCP client](./mcp.md) covers Claude Code, omp, and pi.