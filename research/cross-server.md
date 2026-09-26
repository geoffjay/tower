# Cross-server communication research

## Requirement

Agents will run on more than one machine (laptop + desktop + rented server, per
the herdr "machines" model and the factory notes' multi-host plans). Coordination
should stay simple; execution is distributed.

## Prior art on this machine

- **herdr `machine`**: join machines over SSH; their workspaces/agents sit
  alongside local ones in one client view. SSH transport, no central broker.
  Also `--remote <ssh-target>` attach. (Herdr Cloud "coming soon" = same
  machines, no SSH setup.)
- **Hermes factory notes** (`/home/cap/Projects/factory/FACTORY_NOTES.md` §2):
  kanban is single-host by design (local PIDs, local process spawn). Recorded
  multi-host options, in preference order:
  1. Canonical board on host 1; remote hosts get workers with **remote terminal
     backends** (ssh/docker) so execution goes multi-host while coordination
     stays single-host
  2. Shared kanban.db mount (rejected-ish: SQLite WAL over network FS,
     cross-host PID liveness issues)
  3. One board per host bridged by cron/gateway
- **agentd**: all services HTTP on localhost — never solved cross-server.
- **openrig**: local daemon only; remote daemon addresses mentioned in TUI docs
  ("the selected daemon address is remote") but multi-machine is not its focus.

## Options

### 1. Single coordinator + SSH remote execution (recommended)

One agentos server on the primary host owns the DB (messages, tasks, inventory).
Remote machines run a small **node agent** that:
- connects out to the coordinator (no inbound firewall holes on workers)
- executes agent operations locally (via herdr on that machine)
- streams events/SSE back to the coordinator

This is herdr's machine model plus the factory notes' option 1, hardened by
avoiding network filesystems entirely. It matches the "single server" goal:
coordination state lives in exactly one place.

### 2. Mesh of peers (A2A-native)

Every machine runs a full agentos server; servers find each other via Agent
Cards and talk A2A. Maximal fidelity to the protocol, but N databases means
distributed-state problems (the exact pain agentd's rewrite is escaping).

### 3. Message bus (NATS/Redis) between servers

Solid ops story, but adds infrastructure to manage — against the
one-binary spirit.

## Recommendation

**Option 1**, with option 2 as the protocol between *independent* parties:

- Within an agentos deployment: coordinator + SSH-connected node agents
  (SSH already keys machines; herdr proves the UX)
- Between agentos and the outside world (foreign agents, other people's
  servers): speak **A2A** — HTTP(S) + JSON-RPC + SSE works across any network
  boundary and is the emerging standard
- Web UI and TUI connect to the coordinator only; it fans out to nodes

## Practical details

- Node agents dial home with a long-lived connection (websocket/SSE upstream
  or SSH reverse tunnel); coordinator pushes work down that channel
- Events flow node → coordinator → subscribers, tagged with origin machine
- Machine inventory is a table (host, address, status, last_seen) — herdr's
  `machine` concept, made durable
- Failure mode: node offline = its agents show as unreachable, work for it
  queues or reroutes. Coordinator down = nodes keep running (herdr keeps
  agents alive independently), reconnect on coordinator return. Local agent
  state survives because herdr owns the PTYs, not agentos.