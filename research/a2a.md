# A2A protocol research

**Agent2Agent (A2A)** — open standard (originally Google, donated to Linux
Foundation, now under the Agentic AI Foundation with a TSC from AWS, Cisco,
Google, IBM, Microsoft, Salesforce, SAP, ServiceNow). Apache 2.0. Official SDKs:
Python, JS, Java, Go, .NET, Rust.

- Spec: https://a2a-protocol.org/latest/specification/
- Rust SDK: https://github.com/a2aproject/a2a-rs

## What it is

A common language for **opaque agents to collaborate over HTTP(S)**: discovery,
task delegation, streaming results. Explicitly complementary to MCP:

- **MCP** = agent → tools
- **A2A** = agent → agent (and human → agent)

What it is *not*: not an agent dev kit, not a sub-agent/tool-call protocol, not
a messaging app. It's the wire between independent parties who don't share
memory or internals.

## Core concepts

| Element | Description |
|---|---|
| **Agent Card** | JSON "business card" at a well-known URL: identity, skills, endpoint, capabilities (streaming, pushNotifications), auth requirements |
| **Task** | Stateful unit of work with unique ID and lifecycle: `submitted → working → input-required → completed / failed / canceled / rejected` |
| **Message** | One turn of conversation; role (`user`/`agent`), unique messageId |
| **Part** | Content container: exactly one of `text`, `raw` (inline bytes), `url`, or `data` (structured JSON); plus optional mediaType, filename, metadata |
| **Artifact** | Tangible deliverable generated during a task; chunked with `append`/`lastChunk` for streaming reassembly |
| **contextId** | Groups related tasks; carries context across interactions |

Transport: HTTP(S), JSON-RPC 2.0 payload format. Auth via standard web security
(OAuth, API keys) in headers, declared in the Agent Card.

## Interaction mechanisms

1. **Request/response (polling)** — `SendMessage`, then `GetTask` polls for updates
2. **Streaming (SSE)** — `SendStreamingMessage`: HTTP 200 with
   `text/event-stream`; events carry `TaskStatusUpdateEvent` and
   `TaskArtifactUpdateEvent`; stream closes on terminal state
3. **Push notifications** — server POSTs to a client-registered webhook
   (`TaskPushNotificationConfig`) for disconnected clients; payload is a
   `StreamResponse` (task, message, statusUpdate, or artifactUpdate); JWT+JWKS
   signing recommended; client then calls `GetTask` for the full state

**This validates the SSE goal**: A2A's streaming mechanism is SSE with
JSON-RPC envelopes. Designing agentos's event streams as A2A-shaped
(`status-update` / `artifact-update` events with task IDs) makes the internal
and external protocols one design.

## Task lifecycle fit for agentos

A2A's task states map cleanly onto herdr's detection states:

| A2A task state | agentos/herdr agent state |
|---|---|
| `submitted` | launching (pane provisioned, CLI typed) |
| `working` | working (detected) |
| `input-required` | **blocked** (detection rules: "do you want to proceed?") |
| `completed` / `failed` / `canceled` | done / dead / killed |

So an agent session **is** an A2A task; herdr's pane scraping is the
ground truth feeding task-status updates.

## Where it fits agentos

**Adopt the model, expose the protocol:**

1. **Internal message schema**: model messages/parts/artifacts after A2A's
   (they're well-designed and generic). One `messages` table with typed parts.
2. **Task tracking**: agent sessions as tasks with the lifecycle above; the
   `input-required` ↔ blocked mapping is the human-attention signal.
3. **External boundary**: the server exposes an A2A endpoint (Agent Card at
   `/.well-known/agent-card.json`) so foreign agents — or another agentos
   instance — can delegate work in. This *is* the cross-server protocol
   for inter-deployment comms (see [cross-server.md](cross-server.md)).
4. **Don't over-rotate**: intra-deployment comms (agentos server ↔ its own
   agents, node agents, TUI) doesn't need JSON-RPC ceremony — plain routes and
   SSE with A2A-shaped event types are enough. A2A is the **edge** protocol
   for parties that don't share a database.

## Rust SDK status

a2a-rs exists with the core types (AgentCard, Task, Message, Part, streaming).
If agentos is Rust (like agentd), the SDK gives the wire types for free.