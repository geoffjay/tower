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
  at: "2026-09-26T22:59:25Z"
---

# 10. Client CLI

`tower <verb>` (all thin over `/v1`):

```
tower ps [-m]                       # agents table w/ state glyphs
tower spawn <name> --kind claude [--workdir .] [--worktree] [--prompt "..."]
tower prompt <name> 'text' [--wait] # --wait blocks until settled state
tower read <name> [--source visible] [--format ansi]
tower stream <name>                 # attach to SSE output (like tail -f)
tower stop <name> [--remove]
tower inbox                         # pending questions/approvals addressed to me
tower ask <name> ...                 # send question
tower approve <msg-id> [--deny]      # answer approval
tower send <to> --kind <kind> ...    # generic unified send
tower task list [--state queued] [--tag x] [--mine <name>]   # queue + owned views
tower task show <id>                          # detail incl. assignment/lease trail
tower task create 'title' [--tag x] [--assign name] [--priority N]
tower task assign <id> <name>                 # dispatch (operator; orchestrator later)
tower task cancel <id> / task release <id> [--as <agent>]
tower task start|heartbeat <id> [--as <agent>]            # agent work loop
tower task status <id> <working|input-required|completed|failed> [--result R] [--as <agent>]
tower task create 'title' --assign name --when-available | --at <time>   # reserve: deliver when free / after time
tower task assign <id> <name> --when-available                          # reserve instead of immediate
tower schedule create 'title' (--daily HH:MM | --cron '…') [--tz Zone] [--assign name] [--tag x] [--lease-s N]
tower schedule list | show <id> | pause <id> | resume <id> | run <id> | rm <id>
tower service install | uninstall | status          # run `tower serve` as a user service (launchd / systemd)
                                    # --as defaults to $TOWER_AGENT (set in spawned panes)
tower machines                      # machine inventory
tower machines add <name>           # issue a node token (prints once)
tower tui                           # launch TUI
tower serve / node / doctor / schema
```

Output: human tables by default, `--json` everywhere (agentd lesson: the CLI is
scriptable and agent-usable; MCP wraps the same surface).
