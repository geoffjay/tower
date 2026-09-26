# IPC research: inter-agent and human-agent communication

## Requirements

1. Human → agent: send prompts/instructions to a specific agent
2. Agent → human: ask questions, request approvals, report blocked state
3. Agent → agent: delegate, coordinate, share results
4. All messages durable (agents crash; humans are away for hours)
5. Multi-subscriber streams of agent output for the monitoring UI

## Options considered

### 1. Unix domain sockets (herdr's choice)

- Used by herdr for its client/server API (`herdr.sock`, `herdr-client.sock`)
- Pros: zero setup, no ports, filesystem permissions as auth, fast
- Cons: local-only — doesn't cross machines; the server needs more than this
- **Verdict: right for herdr, too narrow for tower's server API**

### 2. HTTP + JSON-RPC over a single port (agentd's mistake was N services, not HTTP)

- One HTTP server with namespaced routes; JSON-RPC 2.0 for request/response
- agentd's failure mode was nine of these, not the pattern itself
- **Verdict: correct core for the single server**

### 3. SSE for streaming (A2A-standard, fits the goal)

- Server-Sent Events: `Content-Type: text/event-stream`, one-directional
  server→client push over plain HTTP, auto-reconnect built into browsers and
  trivial in curl/CLI clients
- The A2A protocol's streaming mechanism is literally SSE with JSON-RPC
  payloads: `SendStreamingMessage` → stream of `TaskStatusUpdateEvent` /
  `TaskArtifactUpdateEvent`, `SubscribeToTask` for resubscription after drops
- agentd used WebSockets; openrig uses an HTTP daemon with webhooks
  (`/api/activity/hooks`)
- **Why SSE over WebSockets here**: simpler infra (no upgrade handshake, no
  sticky-session proxies), works through plain HTTP tooling, natural fit for
  the read-only web UI, and it's the emerging standard for agent streaming.
  Bidirectional needs (prompting an agent) go through a normal POST, so
  full-duplex isn't required.
- **Verdict: yes — SSE for agent output streams, event bus, and UI updates**

### 4. MCP (Model Context Protocol)

- Standard for agent↔tool access. openrig exposes its daemon as an MCP server
  so agents can call `rig_up`, `rig_send`, etc. — the same management surface
  as the human CLI
- **Verdict: not the backbone, but expose the server's control API over MCP too
  so agents can self-organize** (openrig proves this pattern works)

### 5. A2A (Agent2Agent protocol)

- Open Linux Foundation standard for agent-to-agent over HTTP(S)/JSON-RPC 2.0:
  Agent Cards, Tasks, Messages/Parts, Artifacts, SSE streaming, push notifications
- See [a2a.md](a2a.md) for details
- **Verdict: adopt as the inter-agent wire format** — see below

## Recommended layering

```
                  ┌──────────────────────────────┐
                  │        tower server        │
                  │  HTTP + JSON-RPC (1 port)    │
                  │  ├─ REST-ish control routes  │
                  │  ├─ SSE /events (bus)        │
                  │  ├─ SSE /agents/:id/stream   │
                  │  ├─ MCP endpoint             │
                  │  └─ herdr socket (agent I/O) │
                  │  SQLite: messages, tasks,    │
                  │         agents, events      │
                  └──────────────────────────────┘
```

- **Transport**: one HTTP server. Human comms and agent comms use the same
  message model (sender, recipient, parts, task_ref) — the lesson from agentd
  is that notify/ask/communicate should be one `messages` table with types, not
  three channels
- **Durability**: every message is a row before delivery (agentd's rooms, herdr
  kanban's "every handoff is a durable SQLite row", openrig's queue transitions
  all agree on this)
- **Streaming**: SSE, with a monotonic event log so clients can replay from a
  cursor after reconnect (A2A's `SubscribeToTask` resubscription semantics)
- **Agent I/O**: through herdr's socket API (prompt/send-keys/read/wait) —
  not scraping tmux directly
- **Human attention**: "blocked" state from herdr's detection → notification;
  approval requests as messages requiring a response (agentd's RequireApproval
  policy pattern)

## Concrete message kinds (unified table)

| kind | from | to | examples from prior art |
|---|---|---|---|
| prompt | human/agent | agent | herdr `agent prompt`, agentd room post |
| question | agent | human | agentd ask (UUID'd, response expected) |
| approval | agent | human | agentd RequireApproval (5-min timeout) |
| notice | agent | human | agentd notify (priority, lifetime) |
| delegation | agent | agent | openrig `rig send` seat-to-seat |
| broadcast | human/agent | pod/room | openrig `rig broadcast`, agentd rooms |

All one table, one API. The CLI, TUI, web UI, and MCP tools are thin views over
it.