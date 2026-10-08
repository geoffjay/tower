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
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
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

[harness.claude]
permission_mode = "acceptEdits"  # "acceptEdits" | "default"; yolo only per-agent opt-in

[harness.pi]
args          = []             # extra args appended to spawn

[node]                        # only used by `tower node`
coordinator   = "wss://host:8266/nodes"
token        = "<per-machine token>"

[a2a]
enabled      = true
external_url = "https://agents.example.com"   # used in agent card
```

Agent definitions (one folder per named agent under `agents/`):
`PROMPT.md` is the first prompt, `config.toml` holds spawn arguments
(`kind`, `workdir`, `worktree`); explicit `tower spawn` flags override both.

`tower doctor` validates: herdr reachable, socket/CLI, database writable,
port free, claude/pi executables found.
