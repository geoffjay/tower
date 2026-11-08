---
type: Concept
title: Design §10 — Client CLI
description: tower CLI verbs, all thin over /v1, with --json everywhere.
tags:
  - design
  - design-s10
  - cli
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §10 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
---

# 10. Client CLI

`tower <verb>` (all thin over `/v1`):

```
tower ps [-m]                       # agents table w/ state glyphs
tower spawn <name> --kind claude [--workdir .] [--worktree] [--prompt "..."] [--model <id>] [--arg <flag>]...
tower spawn --name foo              # from ~/.config/tower/agents/foo/ (PROMPT.md + config.toml)
tower read <name> [--source visible] [--format ansi]
tower stream <name>                 # attach to SSE output (like tail -f)
tower stop <name> [--remove]
tower inbox                         # pending questions/approvals addressed to me
tower ask <name> ...                 # send question
tower approve <msg-id> [--deny]      # answer approval
tower dismiss <msg-id>               # close as stale, no answer delivered
tower send <to> --kind <kind> ...    # generic unified send
tower task list [--state queued] [--tag x] [--mine <name>]   # queue + owned views
tower task show <id>                          # detail incl. assignment/lease trail
tower task create ['title'] [--tag x] [--assign name] [--priority N] [--pre-run P] [--post-run P]   # no title in a terminal: asks
tower task edit <id> [--title T] [--description D] [--pre-run P] [--post-run P] [--at TIME]   # '' clears; D§5.2.3
tower task assign <id> <name>                 # dispatch (operator; orchestrator later)
tower task cancel <id> / task release <id> [--as <agent>]
tower task start|heartbeat <id> [--as <agent>]            # agent work loop
tower task status <id> <working|input-required|completed|failed> [--result R] [--as <agent>]
tower task create 'title' --assign name --when-available | --at <time>   # reserve: deliver when free / after time
tower task assign <id> <name> --when-available                          # reserve instead of immediate
tower schedule create 'title' (--daily HH:MM | --cron '…') [--tz Zone] [--assign name] [--tag x] [--lease-s N] [--pre-run P] [--post-run P]
tower schedule edit <id> [--title T] [--description D] [--daily HH:MM | --cron '…'] [--tz Zone] [--pre-run P] [--post-run P]
tower schedule list | show <id> | pause <id> | resume <id> | run <id> | rm <id>
tower service install | uninstall | status          # run `tower serve` as a user service (launchd / systemd)
tower workflow apply <file.toml>    # validate + store a new version when changed (operator; D§5.6)
tower workflow list | show <name> | rm <name> | enable <name> | disable <name>
tower workflow run [<name>] [--input k=v]... [--wait]   # start a run; --wait follows it to the end; no name in a terminal: asks
tower run list [--workflow <name>] [--state running] | show <run-id> | cancel <run-id>
tower artifact put <task-id> <name> [--file <path>] [--media-type T] [--as <agent>]   # stdin when no --file
tower artifact get <artifact-id>    # print the content
                                    # --as defaults to $TOWER_AGENT (set in spawned panes)
tower machines                      # machine inventory
tower machines add <name>           # issue a node token (prints once)
tower tui                           # launch TUI
tower ui                            # print a login link for the web UI (scoped read-only token, D§13)
tower contract                      # print docs/agent-loop.md (embedded at build; the agent
                                    # contract travels with the binary, not the repo)
tower serve / node / doctor / schema
```

Output: human tables by default, `--json` everywhere (agentd lesson: the CLI is
scriptable and agent-usable; MCP wraps the same surface).

**Asked, not remembered.** `tower task create` and `tower workflow run` take
their positional argument as optional. Run one without it in a terminal and
the CLI asks for each missing value with the control that fits it: a list of
agents, workflows, or known tags to pick from; a date picker and a time for
`--at`; a number for priority and lease; yes/no for `--wait`; the workflow's
declared `[inputs]` one by one, with their description, default, and
required mark. Flags given on the command line are not asked again. Before
it acts, the CLI prints the equivalent one-line command and asks to go
ahead, so the flags are learned by use. Without a terminal (stdin or stderr
not a TTY, as in scripts and agents) a missing argument is an error, never
a hang. Questions go to stderr, so `--json` output stays clean.

For blocked-agent inbox messages, the human `tower inbox` summary also prints
the captured pane context, so the question is visible without opening the TUI.
