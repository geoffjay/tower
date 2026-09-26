# openrig research

openrig is real: **`github.com/mvschwarz/openrig`** (Apache 2.0, TypeScript,
npm `@openrig/cli`, 471 stars) — "Multi-agent harness that runs Claude Code and
Codex together as one system." A harness wraps a model; a rig wraps your harnesses.
Define an agent team in YAML, boot it with one command. Not installed on this
machine; research from the README, docs, and site.

## Architecture

```
CLI / TUI / MCP
      |
Hono HTTP daemon
      |
  Domain services
      |
  SQLite + tmux + runtime adapters
```

- **Local daemon** (Hono HTTP) + CLI + TUI + MCP server, built on tmux
- **Single SQLite database** for instance state — same single-server shape agentos wants
- **Runtimes are adapters**: native Claude Code and Codex sessions, terminal nodes,
  and a Pi adapter using an RPC runner inside a terminal pane (note: pi support
  exists upstream, useful for ohmypi)
- **MCP server** exposes management tools (`rig_up`, `rig_ps`, `rig_send`,
  `rig_chatroom_send`, ...) so agents manage their own topology — the daemon
  serves both humans (CLI/TUI) and agents (MCP) from one place
- React web UI exists but is in maintenance mode — **the TUI is the primary
  operator surface**, which matches agentos's "web is monitor-only" stance

## How it launches Claude Code and Codex

- RigSpec (YAML): pods, members, edges, continuity policies, culture file
  (CULTURE.md sets coordination norms)
- `rig up <name>` boots tmux sessions, harnesses, startup files, readiness checks
- Claude Code: `--permission-mode acceptEdits`, classic renderer for scrollback;
  YOLO off by default (`permission_policy: builtin:yolo` →
  `--dangerously-skip-permissions` only when explicitly chosen)
- Codex: `-s workspace-write` unless a named profile governs the sandbox;
  `--add-dir` for workspace `.git` and shared queue-state
- Every agent runs in a **tmux session you can attach to directly** — tmux is
  the escape hatch and debugging surface
- **Discovery/adoption**: `rig discover` fingerprints existing tmux sessions,
  `rig adopt` brings them under management (nice pattern for agentos: adopt
  rather than require greenfield)
- **Snapshot/restore**: `rig down --snapshot` captures topology; `rig up <name>`
  restores with per-node outcomes (resumed/fresh/failed)
- Hooks: activity relays POST event type/subtype + seat/runtime identity to the
  daemon's `/api/activity/hooks` endpoint with an activity token (payload excludes
  prompt text and tool arguments)

## How openrig supports herdr as a provider

The key integration for agentos's TUI goal:

- `rig terminal open <rig> --provider herdr` — opens a rig's team terminals
  together in a herdr workspace; `--provider cmux` also available
- "With herdr installed and connected, open the starter's terminals together."
  The TUI shows **coordination state**; **herdr shows the actual agent terminals
  alongside it**. Division of labor: openrig = team brain, herdr = terminal muscle
- It is explicitly "terminal integration, not native plugin enrollment" —
  read the opened/absent/degraded result; a partial view is not a healthy team
- Underlying sessions remain accessible through tmux regardless of provider

So openrig does **not** embed herdr; it delegates terminal presentation to it
while keeping ownership of topology, messaging, and queue state. That is exactly
the layering agentos should copy: agentos TUI on top of herdr panes.

## Concepts worth stealing

| Concept | What it is | agentos analog |
|---|---|---|
| Seat | Stable role/address in a rig (`dev-owner@first-project`); the occupying conversation can change while identity/context remain | Agent identity decoupled from process |
| Pod | Group of seats with shared guidance; each agent keeps its own context window | Grouping/namespace |
| RigSpec | Declarative YAML topology, portable RigBundle with SHA-256 integrity | Config format |
| Culture file | CULTURE.md sets coordination norms per rig | Team conventions |
| Queue | Durable task records with transitions; `rig queue show <id> --full` | Task tracking |
| Chatroom | `rig chatroom` multi-party comms | Rooms |
| Send/broadcast | `rig send seat@rig 'msg'`, `rig broadcast` | Messaging |
| Kernel | A meta-rig whose seats operate the daemon itself (agent-operated software) | Self-management |
| Agent-operated upgrades | Migrations executed by an agent following a shipped skill, not by the tool | Upgrade story |

## Cautions

- openrig writes a lot of provider config (trust settings, hooks in
  `~/.codex/config.toml`, `~/.claude.json`, `.claude/settings.local.json`) — heavy
  machine footprint; agentos should be more conservative
- 3,026 commits and a very elaborate upgrade/migration story (telemetry state
  migrations with preimage receipts) — signs of accreted complexity; agentos
  should stay smaller
- Its docs emphasize epistemics ("a delivered message is not a reviewed result")
  — the queue/artifact model is opinionated; adopt the ideas, not the weight

## Sources

- https://github.com/mvschwarz/openrig (README)
- https://openrig.dev — docs, spec, blog ("Why I Built OpenRig")
- `docs/reference/getting-started.md` in the repo (terminal provider usage,
  permission model, kernel framing)