---
type: Concept
title: Design §4 — Paths and configuration
description: XDG paths, config.toml keys and defaults, and tower doctor checks.
tags:
  - design
  - design-s4
  - config
  - paths
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §4 (original, removed from repo root after ingest)
generated:
  by: claude-code/1.0
  at: "2026-10-23T23:30:00Z"
---

# 4. Paths and configuration

XDG layout, zero-config start:

| Item | Path |
|---|---|
| Config | `~/.config/tower/config.toml` |
| Database | `~/.local/share/tower/tower.db` |
| Agent definitions | `~/.config/tower/agents/<name>/` (D§10 `spawn --name`) |
| Auth token | `~/.local/share/tower/token` (0600, generated on first run) |
| Unix socket | `$XDG_RUNTIME_DIR/tower.sock` (default bind) |
| TCP listen | `127.0.0.1:8266` (default; node deployments use `0.0.0.0` + token) |

Config file (all keys optional; defaults in parentheses):

```toml
[server]
bind_socket   = true          # unix socket
bind_tcp      = "127.0.0.1:8266"
event_retention_days = 14     # sweeper deletes older events hourly; 0 keeps all

[herdr]
socket_path   = "~/.config/herdr/herdr.sock"  # driver target
cli_fallback  = true          # wrap `herdr` CLI if socket protocol fails

[harness.<kind>]             # one section per harness kind (claude, pi, omp, opencode)
permission_mode = "acceptEdits"  # claude only: "acceptEdits" | "default"; yolo only per-agent opt-in
args          = []             # extra args appended to every spawn of that kind

[harness.pi]
args          = []             # extra args appended to spawn

[node]                        # only used by `tower node`
coordinator   = "wss://host:8266/nodes"
token        = "<per-machine token>"

[a2a]
enabled      = true
external_url = "https://agents.example.com"   # used in agent card

[inbox]
blocked_deadline_s = 86400   # blocked-agent questions/approvals (24h)
```

The blocked-agent inbox deadline defaults to 24 hours so operators can
answer from the harness pane without a five-minute expiry; it remains
configurable. Other questions and approvals retain the general five-minute
default unless their sender supplies `deadline_s`.

Agent definitions (one folder per named agent under `agents/`):
`PROMPT.md` is the first prompt, `config.toml` holds spawn arguments
(`kind`, `workdir`, `worktree`, `model`, `args`); explicit `tower spawn`
flags override both. `model` becomes `--model <id>` for the harness
(opencode: `OPENCODE_CONFIG_CONTENT` in the pane env, D§8.5); `args` are
extra harness flags (e.g. omp's `--smol`, `--plan`). Unknown keys are
rejected with an error naming the key.

Harness argument order at spawn: the kind's own launch flags (opencode:
`--standalone`, D§8.5) first, then `[harness.<kind>] args` from
`config.toml` (per-kind defaults), then the agent's own `args` (per-agent
overrides and extras), then `--model <id>`.

`tower doctor` validates: herdr reachable, socket/CLI, database writable,
port free, claude/pi executables found.
