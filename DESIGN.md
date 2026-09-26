# agentos — Design Document

Status: draft v0.1 (2026-09-26)
Inputs: [README.md](README.md), [RECOMMENDATIONS.md](RECOMMENDATIONS.md), research/

---

## 1. Purpose and scope

agentos is a single-server system for running and supervising multiple coding
agents. It provides one coordination point (server + database + event log), a
thin client, a read-only web UI, a TUI layered on top of herdr, and standard
agent-facing interfaces (MCP, A2A).

Target harnesses: **Claude Code** and **pi (ohmypi)**, executed in
**herdr**-managed panes.

### Goals

1. One server binary, one port, one database — agentd's module lesson
2. Spawn, supervise, and communicate with agents across machines
3. Stream agent output and system events over SSE
4. Human attention routed by blocked-state detection (questions/approvals)
5. Interfaces: CLI (humans), MCP (agents), SSE (UIs), A2A (foreign agents)
6. Everything durable: messages, tasks, and events are rows before delivery
7. Agents survive server loss (herdr owns the PTYs, not agentos)
8. Shared task pool: agents pick up unowned work, own it exclusively
   (atomic claim + lease + heartbeat), and report status themselves

### Non-goals

- No terminal multiplexer (herdr owns PTYs, detection, layout persistence)
- No web configuration or control (web UI is read-only monitoring)
- No distributed database (single coordinator; nodes execute, not vote)
- No model/provider proxying or cost accounting in v1
- No multi-user auth in v1 (single operator; token auth only)

---

## 2. Stack

Rust, single binary. Rationale: agentd lineage, static deployable binary, a2a-rs
wire types, mature async stack.

| Concern | Choice |
|---|---|
| Runtime | tokio |
| HTTP | axum (REST control + SSE + MCP + A2A all on one router) |
| Database | SQLite via sqlx, WAL mode, busy_timeout=5s |
| IDs | ULID (sortable, time-ordered) |
| Time | unix epoch milliseconds (INTEGER columns) |
| JSON | serde / serde_json |
| CLI parsing | clap |
| TUI | ratatui + crossterm |
| MCP | rmcp (or hand-rolled streamable HTTP) |
| A2A | a2a-rs types, custom axum routes |
| Logs | tracing + tracing-subscriber (JSON) |
| Static web assets | rust-embed for icons/fonts; UI itself is Topcoat server-rendered (§12.3) |

### Binary layout

One binary, `agentos`, dispatched by subcommand (herdr's shape):

```
agentos serve          # the server (foreground; systemd unit provided)
agentos node           # remote-machine agent (dials coordinator)
agentos tui            # interactive TUI (client)
agentos <verbs>...     # CLI client verbs (see §13)
```

Client and server share one crate workspace:

```
agentos/
  crates/
    agentos-core      # types: Agent, Task, Message, Event, states, errors
    agentos-server    # axum app, modules, herdr driver, node hub
    agentos-client    # CLI verbs (thin over core + reqwest)
    agentos-tui       # ratatui client
    agentos-web       # Topcoat UI crate (agent cloud + widgets, §12)
```

---

## 3. Process and deployment model

```
                 ┌──────────────────────── agentos serve ────────────────────────┐
                 │ axum router, single port                                       │
                 │  /v1/* control+query   /v1/events SSE   /mcp   /a2a   /ui     │
                 │ ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐ │
                 │ │inventory│ │sessions │ │messaging │ │ tasks  │ │ machine  │ │
                 │ │ module  │ │ module  │ │ module   │ │ module │ │ hub      │ │
                 │ └─────────┘ └─────────┘ └──────────┘ └────────┘ └──────────┘ │
                 │ ┌──────────────────┐  ┌──────────────────────────────────┐     │
                 │ │ herdr driver(s)  │  │ SQLite (WAL): agentos.db        │     │
                 │ │ (local machine)  │  │ + monotonic event log           │     │
                 │ └──────────────────┘  └──────────────────────────────────┘     │
                 └─────────────────────────────────────────────────────────────────┘
                    ▲            ▲              ▲                ▲
          agentos CLI│     agentos tui│    browser (UI)    agentos node ── herdr ── agents
                (HTTP)          (HTTP)       (HTTP+SSE)     on remote machines
```

- `agentos serve` runs as a systemd user unit on the primary machine
  (`agentos.service` ships with the project). It is the only stateful process.
- Agents run inside herdr on the same machine (local herdr driver) or on remote
  machines (node agents relay through the hub).
- Clients are stateless: CLI, TUI, browsers, MCP clients, A2A clients.
- `agentos node` is a stateless relay: it holds no database, executes driver
  calls against its local herdr, and forwards events to the coordinator.

---

## 4. Paths and configuration

XDG layout, zero-config start:

| Item | Path |
|---|---|
| Config | `~/.config/agentos/config.toml` |
| Database | `~/.local/share/agentos/agentos.db` |
| Artifacts | `~/.local/share/agentos/artifacts/` |
| Auth token | `~/.local/share/agentos/token` (0600, generated on first run) |
| Unix socket | `$XDG_RUNTIME_DIR/agentos.sock` (default bind) |
| TCP listen | `127.0.0.1:8266` (default; node deployments use `0.0.0.0` + token) |

Config file (all keys optional; defaults in parentheses):

```toml
[server]
bind_socket   = true          # unix socket
bind_tcp      = "127.0.0.1:8266"
event_retention_days = 14     # event log pruning

[herdr]
socket_path   = "~/.config/herdr/herdr.sock"  # driver target
cli_fallback  = true          # wrap `herdr` CLI if socket protocol fails

[harness.claude]
permission_mode = "acceptEdits"  # "acceptEdits" | "default"; yolo only per-agent opt-in

[harness.pi]
args          = []             # extra args appended to spawn

[node]                        # only used by `agentos node`
coordinator   = "wss://host:8266/nodes"
token        = "<per-machine token>"

[a2a]
enabled      = true
external_url = "https://agents.example.com"   # used in agent card
```

`agentos doctor` validates: herdr reachable, socket/CLI, database writable,
port free, claude/pi executables found.

---

## 5. Core objects and state machines

### 5.1 Agent

An agent is a named, durable identity bound to a harness instance in a herdr
pane. The process may come and go; the agent row persists (openrig's "seat").

```
AgentState:
  unknown ──► launching ──► idle ──► working ──► done
                  │           │  ▲       │
                  │           ▼  │       ▼
                  └────────► blocked ──► (idle, after response)
                  │
                  └─────────────────────► dead   (pane/process gone)
```

States come from herdr detection manifests (`idle/working/blocked/done`,
plus our own `launching/dead/unknown`). `blocked` is the human-attention
signal: the messaging module turns it into a question/approval inbox item.

### 5.2 Task

A task is a unit of delegated work with an owner agent. Task states follow
A2A exactly (so the edge needs no translation):

```
queued ──claim──► working ──► input-required ──► completed
   ▲                │              └──────────► failed
   │                │◄── (answer arrives)         │
   │                └──lease expired──────────────┘
   └── cancel / reject (from any non-terminal state)
```

`queued` = unowned, pick-uppable. `working` = owned, lease active.
`input-required` = owned, blocked on a human answer (lease keeps ticking;
the sweeper pauses expiry while `input-required` so a human's slow reply
doesn't requeue work mid-question — the question message's own
`deadline_at` governs that path instead). Terminal: `completed / failed /
canceled / rejected`. Lease-expired tasks return to `queued` with an
`attempt_count` bumped (see §5.2.1); a max-attempts guard (default 3)
routes exhausted tasks to `failed` instead of infinite requeue.

Mapping: agent state changes emit `task-status` events
(`working` ↔ `working`, `blocked` ↔ `input-required`, terminal states map
directly). A task may also be created without an agent (queued work).

### 5.2.1 Shared task pool and ownership

Tasks are pick-uppable from a shared list. Ownership is exclusive and
time-bound:

- **`owner` (nullable)**: the agent that currently owns the task. `NULL` means
  unowned and pick-uppable.
- **`lease_expires_at`**: ownership expires if not renewed. Owner must
  heartbeat via `POST /v1/tasks/{id}/heartbeat` (default window: 60s lease,
  heartbeat at 20-30s; configurable per task and globally).
- **Claim protocol**: `POST /v1/tasks/{id}/claim` — a single transaction:
  `UPDATE tasks SET owner = :agent, state='working', lease_expires_at = now
  + lease WHERE id = :id AND (owner IS NULL OR lease_expires_at < now)`.
  Affected-rows == 1 → claimed; 0 → `conflict` error returned to the losing
  agent. No coordinator arbitration needed — SQLite row state is the lock
  (the single-writer SQLite design from §6 makes the check-and-set atomic).
- **Lease expiry sweeper** (part of the tasks module, runs every ~10s):
  expired leases → `owner = NULL`, `state = 'queued'`, `attempt_count =
  attempt_count + 1`, emit `task.leased_out` event. The work returns to the
  pool; another agent (or the same one after its crash) can re-claim it.
  When `attempt_count` reaches `max_attempts` (default 3), the sweeper moves
  the task to `failed` with a `lease_exhausted` result instead of requeueing
  (no infinite crash-loop churn).
- **Completion**: only the owner may move the task to a terminal state;
  non-owner terminal writes return `conflict`.
- **Priority/queueing**: `priority INTEGER` (default 0, higher = sooner);
  pool queries (`GET /v1/tasks?state=queued`) order by `priority DESC,
  created_at ASC`. Agents may pull the next matching task atomically:
  `POST /v1/tasks/pull {tags?, capacity: 1}` claims the highest-priority
  unowned task matching a filter — same CAS semantics as claim.
- **Tags**: `tags TEXT` (JSON array) on tasks for routing/filtering
  (`["backend", "rust"]`); pull filters on tags. Pods/teams from openrig map
  to tag filters, not new tables.
- **Reclaim, not orphan**: lease expiry is the crash story — an agent that
  dies mid-task has its task auto-requeued within one lease window. There is
  no "stuck forever" state; `agent.state=dead` + expired lease = clean pickup
  by a survivor.
- **Status reporting**: owners report progress with
  `POST /v1/tasks/{id}/status` (state transitions + `result` updates); every
  transition emits `task.status` events so the UI and other agents watch
  without polling the row.

Why lease + heartbeat instead of session-bound ownership: agentd/hermes
kanban's PID-liveness checks fail across machines (cross-host PIDs aren't
comparable, and herdr-panes outlive client sessions). A lease is machine-
agnostic: heartbeat renewal is just another event write, and expiry is a
simple timestamp comparison. Heartbeats piggyback on the agent loop (MCP tool
call between prompt turns) — see the MCP tools in §7.

### 5.3 Message

One row per communication act, any direction:

- `from`: human (operator id, `me`), agent (agent id), service, external (A2A)
- `to`: human, agent, room (named group), external
- `kind`: `prompt | question | answer | approval | approval-response |
  notice | delegation | broadcast`
- `parts`: A2A Part array — `[{text:...} | {data:...} | {raw:...} | {url:...},
  media_type?, filename?, metadata?]`
- `status`: `pending | delivered | answered | expired | failed`
- `deadline_at`: questions/approvals expire (default 5 min, configurable)

### 5.4 Event

Append-only log row. Every state change anywhere is one event. Event types:

| type | payload (summary) |
|---|---|
| `server.started` | version, pid |
| `agent.created` / `agent.removed` | agent summary |
| `agent.state` | agent_id, from, to, detection detail |
| `agent.output` | agent_id, chunk (cursor into artifacts) |
| `task.created` / `task.status` | task_id, state, result ref |
| `task.claimed` | task_id, owner_id, lease_expires_at |
| `task.leased_out` | task_id, prior owner, state → queued |
| `task.completed` / `task.failed` | task_id, owner_id, result ref |
| `message.created` / `message.status` | message summary, status transitions |
| `approval.expired` | message_id, agent_id |
| `machine.state` | machine_id, status |
| `node.registered` / `node.disconnected` | machine summary |

Envelope: `{seq, ts, type, subject_type, subject_id, payload}` where `seq` is
the global monotonic cursor.

### 5.5 Machine

A registered host. `role: coordinator | node`, `status: online | offline |
degraded`, `last_seen_at`. Local machine is always present (`local`).

---

## 6. Data model (DDL)

```sql
CREATE TABLE machines (
  id          TEXT PRIMARY KEY,          -- ULID
  name        TEXT NOT NULL UNIQUE,
  role        TEXT NOT NULL DEFAULT 'node',
  address     TEXT,                      -- host:port or ssh alias
  status      TEXT NOT NULL DEFAULT 'offline',
  last_seen_at INTEGER,
  created_at  INTEGER NOT NULL
);

CREATE TABLE agents (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  kind          TEXT NOT NULL,           -- claude | pi | (herdr kind)
  machine_id    TEXT NOT NULL REFERENCES machines(id),
  pane_id       TEXT,                    -- herdr pane identifier
  workdir       TEXT,
  worktree      TEXT,                    -- isolated worktree, or NULL
  state         TEXT NOT NULL DEFAULT 'unknown',
  desired_state TEXT NOT NULL DEFAULT 'running',   -- running | stopped
  permissions   TEXT NOT NULL DEFAULT 'default',   -- default | accept-edits | yolo
  config        TEXT NOT NULL DEFAULT '{}',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE tasks (
  id                TEXT PRIMARY KEY,
  agent_id          TEXT REFERENCES agents(id),   -- creator/responsible agent (optional)
  owner_id          TEXT REFERENCES agents(id),  -- current exclusive owner (NULL = in pool)
  origin            TEXT NOT NULL DEFAULT 'local',  -- local | a2a | node:<machine>
  external_ref      TEXT,                          -- A2A task id when origin=a2a
  context_id        TEXT,                          -- A2A context grouping
  title             TEXT NOT NULL,
  description       TEXT,
  state             TEXT NOT NULL DEFAULT 'queued',
  priority          INTEGER NOT NULL DEFAULT 0,
  tags              TEXT NOT NULL DEFAULT '[]',   -- JSON array
  attempt_count     INTEGER NOT NULL DEFAULT 0,
  max_attempts      INTEGER NOT NULL DEFAULT 3,
  lease_expires_at  INTEGER,                      -- ownership deadline; NULL when unowned
  result            TEXT,                          -- JSON summary
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX idx_tasks_agent ON tasks(agent_id);
CREATE INDEX idx_tasks_state ON tasks(state);
CREATE INDEX idx_tasks_pool ON tasks(state, priority DESC, created_at);

CREATE TABLE messages (
  id          TEXT PRIMARY KEY,
  task_id     TEXT REFERENCES tasks(id),
  from_kind   TEXT NOT NULL,   -- human | agent | service | external
  from_id     TEXT NOT NULL,
  to_kind     TEXT NOT NULL,   -- human | agent | room | external
  to_id       TEXT NOT NULL,
  kind        TEXT NOT NULL,
  parts       TEXT NOT NULL,    -- JSON array of parts
  status      TEXT NOT NULL DEFAULT 'delivered',
  deadline_at INTEGER,
  responded_at INTEGER,
  created_at  INTEGER NOT NULL
);
CREATE INDEX idx_messages_to ON messages(to_kind, to_id, status);
CREATE INDEX idx_messages_task ON messages(task_id);

CREATE TABLE artifacts (
  id           TEXT PRIMARY KEY,
  task_id      TEXT REFERENCES tasks(id),
  name         TEXT NOT NULL,
  media_type   TEXT,
  content_path TEXT NOT NULL,  -- file under artifacts/
  size         INTEGER NOT NULL,
  created_at   INTEGER NOT NULL
);

CREATE TABLE events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts           INTEGER NOT NULL,
  type         TEXT NOT NULL,
  subject_type TEXT,
  subject_id   TEXT,
  payload      TEXT NOT NULL
);
CREATE INDEX idx_events_subject ON events(subject_type, subject_id);
CREATE INDEX idx_events_type ON events(type);

PRAGMA journal_mode=WAL;
```

Single-writer discipline: one write connection behind a tokio mutex; readers on
the pool. Write volume is low (state changes, message posts), so this is not a
bottleneck.

---

## 7. Server API

REST-style JSON over one port. JSON-RPC appears only at the MCP and A2A
boundaries (decision: REST internally for debuggability with curl; the
research protocols where they are standards).

Conventions:

- Errors: `{"error": {"code": "not_found|conflict|timeout|driver|invalid",
  "message": "...", "detail": {...}}}` with proper HTTP status
- All timestamps ms epoch; all ids ULIDs
- `GET /v1/schema` returns the route + event-type registry (herdr's
  `api schema` idea — the contract is introspectable)

### Control (POST)

| Route | Body | Effect |
|---|---|---|
| `POST /v1/agents` | `{name, kind, workdir?, worktree?, permissions?, machine?, prompt?, task_title?}` | Spawn agent in a herdr pane on `machine` (default local); returns agent |
| `POST /v1/agents/{id}/prompt` | `{text, wait?}` | `herdr agent prompt --wait`; creates `prompt` message + task if `task_title` |
| `POST /v1/agents/{id}/interrupt` | — | `ctrl+c` via send-keys |
| `POST /v1/agents/{id}/send-keys` | `{keys}` | escape hatch (power users) |
| `POST /v1/agents/{id}/stop` | `{remove?}` | stop session; keep agent row (seat) |
| `POST /v1/messages` | `{to, kind, parts, task_id?, deadline_s?}` | unified send (any direction) |
| `POST /v1/messages/{id}/respond` | `{parts}` | answer/question or approval-response; sets `responded_at` |
| `POST /v1/tasks` | `{title, description?, priority?, tags?, assign?}` | create task; `assign: <agent>` pre-assigns (claim), else it enters the shared pool |
| `POST /v1/tasks/{id}/claim` | `{as: <agent_id>, lease_s?}` | atomic CAS claim (conflict on 0 rows); state → `working` |
| `POST /v1/tasks/pull` | `{as: <agent_id>, tags?, capacity: 1, lease_s?}` | atomically claim next-highest-priority unowned task matching tags; empty result when pool dry |
| `POST /v1/tasks/{id}/heartbeat` | `{as: <agent_id>}` | renew lease (owner-only, 409 otherwise) |
| `POST /v1/tasks/{id}/status` | `{as: <agent_id>, state?, result?}` | owner status report; emits `task.status`; terminal states close the task |
| `POST /v1/tasks/{id}/release` | `{as: <agent_id>, reason?}` | voluntary release → back to pool (`queued`) |
| `POST /v1/tasks/{id}/cancel` | — | cancel + interrupt owning agent |

### Query (GET)

| Route | Notes |
|---|---|
| `GET /v1/agents` | list with state, machine, task summary |
| `GET /v1/agents/{id}` | full detail incl. recent output ref |
| `GET /v1/agents/{id}/read` | `?source=visible\|recent\|detection&format=text\|ansi` — proxied herdr read |
| `GET /v1/tasks` | `?state=queued&tags=&owner=&since=` pool/inventory queries |
| `GET /v1/tasks/{id}` | task detail + message trail + claim/lease history |
| `GET /v1/messages` | `?to=&status=&since=` inbox queries |
| `GET /v1/machines` | inventory |
| `GET /healthz`, `GET /v1/schema` | health, contract |

### Streaming (GET, SSE)

| Route | Semantics |
|---|---|
| `GET /v1/events` | global event bus. `?cursor=<seq>&filter=type:...&subject=agent:<id>`. `Last-Event-ID` honored; replay from cursor |
| `GET /v1/agents/{id}/stream` | output chunks as `agent.output` events (15s heartbeat comment) |
| `GET /v1/tasks/{id}/stream` | task-scoped events (A2A `SubscribeToTask` semantics) |

Backpressure: slow SSE consumers get disconnected (with a `resume` hint carrying
their cursor); clients re-request from the log. Output chunks are also appended
to artifacts storage so replay is lossless for subscribed tasks.

### MCP (for agents)

`POST /mcp` — streamable HTTP MCP server exposing the same operations:

tools: `agentos_ps`, `agentos_spawn`, `agentos_prompt`, `agentos_send`,
`agentos_ask`, `agentos_approve`, `agentos_task_list`, `agentos_task_show`,
`agentos_task_create`, `agentos_task_claim`, `agentos_task_pull`,
`agentos_task_heartbeat`, `agentos_task_status`, `agentos_task_release`,
`agentos_machine_list`. One management surface for humans and agents
(openrig-proven pattern). The claim/pull/heartbeat/status tools are what
agents use in their work loop: pull work → do it → report status →
complete. Server-side, MCP `agentos_task_claim` is the same code path as
the REST route (one transaction, same CAS) — there is no second
implementation to drift.

### A2A (edge)

- `GET /.well-known/agent-card.json` — card: skills derived from agent roster
  (one skill per named agent), `capabilities: {streaming: true}`.
- `POST /a2a` — JSON-RPC 2.0: `message/send` and `message/stream`.
  Inbound message → `prompt` message + task (origin `a2a`); outbound replies and
  task events use A2A shapes (already native, per §5).
  Push notifications (`pushNotifications`) deferred (phase 6+).

Auth: all `/a2a` and `/v1` TCP access requires `Authorization: Bearer <token>`
(unix socket access is exempt — filesystem permissions are the auth).
Read-only UI routes accept the same token.

---

## 8. Harness layer

### 8.1 Adapter trait

```rust
#[async_trait]
pub trait Harness: Send + Sync {
    async fn start(&self, spec: &AgentSpec) -> Result<Handle>;
    async fn prompt(&self, h: &Handle, text: &str, wait: bool) -> Result<()>;
    async fn interrupt(&self, h: &Handle) -> Result<()>;
    async fn read(&self, h: &Handle, source: ReadSource) -> Result<ReadResult>;
    async fn snapshot(&self) -> Result<Vec<PaneAgentState>>;
    fn events(&self) -> BoxStream<HarnessEvent>;   // state changes, output
}
```

Two implementations:

1. **HerdrDriver** (primary): wraps herdr for one machine. Preferred transport:
   herdr socket (`SemanticFrame` v20). Risk: the socket protocol is not a
   published, stability-guaranteed API. Mitigation: v1 of the driver shells
   out to the `herdr` CLI (`herdr agent start/prompt/read/wait`, `herdr api
   snapshot --json`) with JSON parsing — stable flags, documented, slower.
   Socket mode behind the same trait once the protocol is validated.
2. **TmuxDriver** (fallback, phase 2+): for machines without herdr. Minimal
   PTY + `tail`-based reads, no detection (state = `unknown` unless manual).
   Exists only for escape; not a goal.

### 8.2 Claude Code specifics

- Launch in herdr pane with `claude --permission-mode acceptEdits`
  (config-overridable); `--dangerously-skip-permissions` only when the agent row
  has `permissions = "yolo"` (explicit, per-agent, recorded in events)
- Classic renderer preferred (scrollback; openrig's finding)
- agentos never writes to `~/.claude.json` or hooks by default — trust and
  permission prompts are surfaced via `blocked` state, answered through the
  inbox (send-keys `1`/`2` on approval messages). Claude hook integrations
  (activity relay) are opt-in later.

### 8.3 pi (ohmypi) specifics

- Launch with kind `pi`; detection manifest maintained upstream by herdr
- Adapter drives it via prompt/read first-class; if pi's RPC runner mode is
  needed (openrig's Pi adapter pattern), add a `pi` extension to the trait
  behind config — investigate pi 0.87 RPC surface during phase 1 spike

### 8.4 Harness discovery

`herdr api snapshot` on boot → reconcile with `agents` table:
match by pane id, then by name. Unknown agents appear in inventory as
`adopted: true` candidates (openrig's discover/adopt pattern);
`POST /v1/agents` with `adopt: <name>` takes ownership without relaunching.

---

## 9. Server modules

### 9.1 inventory

Owns `agents` + `machines` rows. Boot sequence: ensure `local` machine row →
driver snapshot → reconcile → emit `agent.state` events for drift.

### 9.2 sessions

Owns spawn/prompt/interrupt/stop flow, translating API calls to driver calls
and emitting events. One task per in-flight driver call per agent (serialized
per agent; herdr prompts wait on detection anyway).

### 9.3 messaging

Unified message store + delivery. "Delivery" to a human = the message appears
in inbox queries + TUI + web UI (they poll/SSE; no push channel in v1).
"Delivery" to an agent = driver `prompt`. Sweeper marks `pending`
questions/approvals `expired` at `deadline_at` and notifies the agent with an
`approval.expired` prompt ("proceed with defaults or stop").

### 9.4 tasks

Task rows + pool semantics (§5.2.1): claim/pull (atomic CAS), lease sweeper
(10s tick: expiry → requeue + `task.leased_out`, max-attempts → `failed`),
heartbeat handling, owner-only terminal writes. Emits `task.*` events.
Task completion is owner-reported (prompt response or detection `done`) —
never inferred from a delivered message alone (openrig's epistemics rule).
The sweeper also pauses lease expiry during `input-required` (see §5.2
state machine note).

### 9.5 machine hub

Node connection manager: accepts outbound websocket upgrades at
`/nodes` (node → coordinator), authenticates per-machine tokens, multiplexes
driver calls to node agents, relays their events into the local log with
`machine_id` tagging. Node protocol: length-prefixed JSON frames over one WS
connection (upstream events; downstream RPC). Coordinator loss = nodes idle;
agents keep running (herdr); node reconnects and resyncs from snapshot.

### 9.6 event bus

Fan-out of the events table to SSE subscribers; pruning per retention config;
owns cursor semantics and heartbeats.

---

## 10. Client CLI

`agentos <verb>` (all thin over `/v1`):

```
agentos ps [-m]                       # agents table w/ state glyphs
agentos spawn <name> --kind claude [--workdir .] [--worktree] [--prompt "..."]
agentos prompt <name> 'text' [--wait] # --wait blocks until settled state
agentos read <name> [--source visible] [--format ansi]
agentos stream <name>                 # attach to SSE output (like tail -f)
agentos stop <name> [--remove]
agentos inbox                         # pending questions/approvals addressed to me
agentos ask <name> ...                 # send question
agentos approve <msg-id> [--deny]      # answer approval
agentos send <to> --kind <kind> ...    # generic unified send
agentos task list [--state queued] [--tag x]   # pool + owned views
agentos task show <id>                          # detail incl. claim/lease trail
agentos task create 'title' [--tag x] [--assign name] [--priority N]
agentos task cancel <id> / task release <id>
agentos machines                      # machine inventory
agentos machines add <name>           # issue a node token (prints once)
agentos tui                           # launch TUI
agentos serve / node / doctor / schema
```

Output: human tables by default, `--json` everywhere (agentd lesson: the CLI is
scriptable and agent-usable; MCP wraps the same surface).

---

## 11. TUI

ratatui client of `/v1` + local herdr shell-outs. Division of labor
(openrig's split): agentos TUI shows **coordination state**; herdr shows
**terminals**.

Views:

- **Fleet**: agents table — name, machine, kind, state glyph
  (`●` working, `○` idle, `◉` blocked, `✓` done, `✗` dead), current task,
  context note
- **Agent detail**: live output tail (`/v1/agents/{id}/stream`), prompt input
  line, message history; `o` opens the actual pane in herdr (shell out to
  `herdr` client attach)
- **Inbox**: pending questions/approvals; reply inline (`y`/`n`/text)
- **Tasks**: task list + state (pool/owned split, lease countdowns on owned
  tasks); detail shows trail + claim history
- **Events**: filtered feed of `/v1/events`
- **Machines**: inventory + node status

Chrome: command palette (`:`), vim-style navigation, state filter/sort.
Keybinding: `i` focuses prompt input on the agent detail view. A pool banner
(`N queued · M working · K blocked`) sits above the fleet table.

---

## 12. Web UI (read-only monitoring)

The web UI is a **health-at-a-glance visualization**, not a management console.
Glance first: the whole system's health readable in seconds without reading
text. Drill-down is deliberately minimal and optional.

### 12.1 The agent cloud (primary view)

Agents rendered as **points in a 2D cloud** (force-directed layout; agents
 drift toward their machine cluster, away from crowded neighbors, settle
 under a light repulsion simulation):

| Visual channel | Encodes |
|---|---|
| Point **color** (hue) | agent state: working=blue, blocked=amber, idle=gray, done=green, dead=red, launching=teal, unknown=violet |
| Point **size** | activity volume — event/message/output rate over a rolling window (bigger = busier) |
| Point **halo/pulse** | attention needed: blocked or expired items glow/pulse |
| Point **brightness** | health/quality score (recency of heartbeats/stale detection = dim) |
| Cluster position | machine grouping (local vs node machines, phase 5) |
| Edge lines (optional) | message volume between agents in the last N minutes (thicker = more traffic); toggleable |

A **floating side panel** appears when a point is selected: agent name, kind,
machine, state, current task + lease countdown, message rate, recent event
sparkline, last output snippet. That is the extent of drill-down in v1 —
full detail lives in the TUI, by design.

### 12.2 Supporting widgets

- Pool bar: queued / working / blocked counts, live
- Machine strip: one chip per machine with status dot (local + nodes)
- Event ribbon: last ~10 events, fading ticker
- No output tails in v1 cloud view (kept on the agent panel snippet only);
  the old per-agent tail page is backlog (§12.4)

### 12.3 Technology

- **Topcoat** (tokio-rs/topcoat, v0.9): full-stack Rust, server-rendered
  with client-side reactivity, no WASM/JS bundle, keeps the whole server +
  UI in Rust and the single-binary story intact
- Rendering: cloud points as absolutely-positioned DOM/SVG nodes updated
  via Topcoat reactive expressions; layout simulation computed server-side
  or client-side in Rust-compiled reactivity (Spike S4.B decides: SVG vs
  DOM points, and where the force layout runs)
- Data: initial render server-side from the DB; live updates by consuming
  the same SSE `/v1/events` stream every client uses (cursor resume on
  reconnect, D§7)
- Risk accepted: Topcoat is explicitly early-stage ("expect breaking
  changes") — pin the version; isolate UI code in `agentos-web` so framework
  churn is one crate's problem (D§2). Spike S4.B verifies canvas-scale
  reactivity (~50–100 points) before committing
- Read-only enforced as before: UI data routes are GET-only; no mutation
  routes in the UI bundle; token per D§13/§17.5

### 12.4 Backlog (drill-down, later if ever)

- Full agent detail page with output tail + message history
- Task pool board and task trails
- Historical charts (event rate, throughput) from the event log
- Config: points vs table view toggle

---

## 13. Security

- Unix socket: filesystem permissions (0600 dir) are the auth; no token needed
- TCP: bearer token (generated first run, 0600); bind 127.0.0.1 by default
- Nodes: per-machine tokens issued by the operator
  (`agentos machines add <name>` prints a token); WS over TLS or SSH tunnel
- A2A endpoint: bearer token required (declared in agent card auth)
- Never log/store secrets in events or message payloads; prompt text is stored
  (it is the work record) but not replicated to third parties
- SQLite, token, artifacts: 0600/0700 permissions, owner-only

---

## 14. Reliability and failure modes

| Failure | Behavior |
|---|---|
| Server crash | Agents unaffected (herdr owns PTYs). On restart: snapshot reconcile, event log intact, agents re-bound by pane id. Lease sweeper resumes; surviving owners renew and keep work, dead owners' tasks requeue on expiry |
| herdr crash | herdr restores layout/sessions (its own persistence). agentos reconciles on next snapshot/poll; agents marked `unknown` until then |
| Node offline | Its agents → `dead/unreachable` view state; queued prompts to it fail fast with `machine_offline` |
| Coordinator offline (node view) | Node keeps agents alive via herdr; reconnects, resyncs snapshot |
| Slow SSE client | Disconnect with resume cursor; lossless replay from event log + artifacts |
| Approval timeout | Sweeper expires it; agent notified; event emitted |
| Task lease expiry | Owner crashed/quiet → requeue within one lease window; next `task.leased_out` event; max-attempts → `failed` |
| Claim race (two agents, one task) | SQLite CAS loses exactly one bidder → clean `conflict`; no double-ownership window |
| DB write failure | Server degrades read-only + logs loudly; driver calls paused (never silently drop) |

Backups: the whole state is `~/.local/share/agentos/` — copy the directory.

---

## 15. Observability

- `tracing` JSON logs to stderr (journald via systemd)
- Event log is the primary audit surface (`agentos` CLI queries it;
  `events?filter=` in UI)
- `/healthz` returns: db ok, driver ok (last snapshot age), node statuses
- Prometheus metrics endpoint: optional phase 5 (`/metrics`, default off)

---

## 16. Testing

- **Unit**: state machines, claim/pull CAS + lease sweeper + max-attempts,
  message/timeout sweeper, event cursor math
- **Integration**: `FakeHarness` implementing the trait — scripted state
  transitions; full API + SSE flow against in-memory SQLite; concurrent-claim
  race tests (N clients claim same task, exactly one wins; loser gets 409)
- **E2E smoke**: temp `HOME`, real herdr, spawn `pi` (and `claude` when
  authed): prompt → working → done → events observed over SSE; recorded as
  `agentos doctor --e2e`
- **Contract**: `GET /v1/schema` output diffed in CI (route/event registry
  changes are deliberate)

---

## 17. Open questions

1. herdr socket protocol stability (SemanticFrame v20) — resolve in phase 1
   spike: validate CLI-driver first, socket later. (Owning risk.)
2. pi RPC runner surface (0.87) — is pane prompt/read enough, or does the
   adapter need pi's programmatic mode? Phase 1 spike.
3. Web UI: Topcoat is chosen (§12.3); spike S4.B (phase 4) validates
   cloud-scale reactivity + layout approach (SVG vs DOM, where the force
   sim runs) before full build-out.
4. Node transport security: TLS + token vs requiring SSH tunnel — phase 5.
5. Scoped read-only UI token vs full token — phase 4.
6. Event/artifact retention defaults and pruning UX — phase 3 tune.
7. Cloud metrics semantics: exact formulas for "activity volume" (size),
   "quality/health" (brightness), and message-volume edges — defined in
   phase 2 as event-log queries; documented in the UI as tooltips.

---

## 18. Phase mapping (→ execution plans)

| Phase | Scope | Exit criteria |
|---|---|---|
| 1. MVP core | core types, server shell (axum + SQLite + event log), HerdrDriver via CLI, agents spawn/prompt/read/wait, CLI verbs `ps/spawn/prompt/read/stream`, SSE `/v1/events` | one machine: spawn claude+pi via herdr, prompt both, stream output to terminal, states visible in `ps`; restart server, agents rebind |
| 2. Messaging + task pool | messages table + kinds, inbox, questions/approvals + sweeper, MCP endpoint, `blocked`→inbox flow, task pool (claim/pull/heartbeat/release + lease sweeper + priorities/tags) | agent blocks on a question; it appears in inbox; answered via CLI or MCP; agent resumes; expiry path tested. Two agents pulling the same pool: exactly one claims each task (race tested), a killed owner's task requeues within one lease window, another agent picks it up and completes it |
| 3. TUI | Fleet/Agent/Inbox/Tasks/Events views, herdr attach action, pool banner | daily monitoring driven entirely from TUI |
| 4. Web UI (agent cloud) | Topcoat UI, agent-cloud view (color/size/halo/brightness channels), floating detail panel, pool bar, machine strip, event ribbon; SSE-fed | cloud shows all agents as colored points with live state changes for an hour soak: zero polling errors, blocked agents visibly pulse, selecting a point opens the side panel with live detail |
| 5. Multi-machine | node agent, machine hub, `/nodes` WS, machine registry, remote spawn, cross-machine pool pickup | agent runs on second machine, appears in local ps/TUI; node disconnect handles gracefully; a task queued on the coordinator is claimed by an agent on the remote machine; node loss mid-task requeues the lease |
| 6. A2A edge | agent card, `/a2a` send/stream, task mapping, foreign delegation in | external A2A client delegates a task to a named agent and streams it to completion |

Each phase gets a detailed execution plan (task breakdown, spike resolutions,
test plan) derived from this document's relevant sections.