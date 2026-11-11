---
type: Plan
title: Discord integration
description: Proposed Gateway bot for agent prompts, jobs, workflow runs, notifications, and inbox responses, with permissions and restart recovery.
tags:
  - discord
  - integrations
  - messaging
  - workflows
status: draft
generated:
  by: claude-code/1.0
  at: "2026-11-11T00:00:00Z"
sources:
  - resource: docs/knowledgebase/concepts/design/07-server-api.md
    title: Tower API and event contracts
  - resource: https://docs.discord.com/developers/interactions/receiving-and-responding
    title: Discord interactions and response limits
  - resource: https://docs.discord.com/developers/interactions/application-commands
    title: Discord application commands
---

# Discord integration

## Status and goal

Proposed; implementation has not started. All commands and configuration in this plan describe new behavior, not current features.

Add Discord as a remote control surface for one tower instance. Users can prompt agents, create jobs, start workflows, and answer questions. Tower sends progress and results to the correct Discord thread. Agents continue to run through herdr.

Dependencies: the existing messaging, job queue, workflow, and event services. This work does not require multi-machine support or the A2A edge.

## Existing contracts

- [D§3](../concepts/design/03-process-and-deployment.md): `tower serve` owns server state; herdr runs agents.
- [D§5](../concepts/design/05-core-objects.md): jobs, messages, and workflow runs retain their existing lifecycle rules.
- [D§7](../concepts/design/07-server-api.md): REST operations and the event stream already expose the required control surfaces.
- [D§9](../concepts/design/09-server-modules.md): reuse service operations; do not duplicate job dispatch or workflow execution.
- [D§13](../concepts/design/13-security.md): the server bearer token has broad access. It does not provide permissions for individual Discord users.
- [Agent availability](../decisions/agent-availability.md): reserve jobs for busy agents. Do not bypass run claims or interrupt work through automatic prompt delivery.

The current agent prompt endpoint returns state, not a structured assistant reply. Terminal reads are snapshots, not reliable final answers. Jobs, explicit messages, and artifacts provide stronger result contracts.

## Proposed architecture

Run an optional Discord adapter under `tower serve`. Isolate Discord protocol code from the service modules. Use an outbound Discord Gateway connection, not a public tower endpoint. Use one Discord application and bot per tower instance initially.

The adapter calls existing service operations directly. It does not execute CLI commands, spawn agent processes, or expose the server bearer token to Discord. A Discord outage must not stop tower or cancel accepted work.

Persist integration state in tower's database. Record Discord interaction IDs, actor IDs, destination IDs, tower object IDs, and event cursors. Use a migration number that is free when implementation starts.

### Alternatives

| Method | Capability | Decision |
|---|---|---|
| Channel webhook | Notifications only; no command handling by itself | Not sufficient for this plan |
| Gateway bot | Commands and buttons over an outbound connection | Preferred for local tower instances |
| HTTP interactions endpoint | Signed requests to a public HTTPS endpoint | Defer; needs public ingress and signature checks |
| External REST/SSE adapter | Separate process uses the existing API | Viable alternative; adds a service and credential boundary |

A shared bot for multiple instances needs explicit routing and ownership rules. It is outside the initial scope.

## User experience

Register a `/tower` command with these operations:

| Proposed command | Behavior |
|---|---|
| `/tower agents` | Show permitted agents and their current states |
| `/tower prompt agent:... message:...` | Send an explicit prompt; report acceptance separately from completion |
| `/tower job create title:... agent:...` | Create a job; optionally assign it and report any waiting reason |
| `/tower job show id:...` | Show state, result, and permitted artifacts |
| `/tower workflow run name:...` | Collect declared inputs and start an existing workflow |
| `/tower run show id:...` | Show run state and step progress |
| `/tower inbox` | Show permitted pending questions and approvals |

Autocomplete agent and workflow names within the user's permitted scope. Use forms for long prompts and workflow inputs. Validate input types and required fields before submission.

Create a thread for each job or run in a configured channel. Show the requester, tower object ID, state, and available actions. Publish waiting reasons, step transitions, failures, and results. Update existing status messages where possible; do not stream terminal frames into the channel.

Questions have an **Answer** button that opens a form. Approvals have **Approve** and **Deny** buttons. These actions respond to existing tower inbox items. Cancel actions use the existing job or run cancellation operation.

Use normal bot messages for work that outlives the original interaction. Discord requires an initial response within three seconds. Interaction tokens expire after fifteen minutes. Acknowledge or defer promptly; do not wait for agent completion before acknowledging.

## Security and output policy

- Require explicit allowed guild, channel, user, and role IDs. Deny access by default; disable direct messages initially.
- Separate permissions for viewing, prompting, creating work, cancelling, answering, and approving.
- Check permissions on every command, form submission, and button click. Hiding a command is not authorization.
- Bind each action to the original tower object and configured Discord destination. Reject altered, expired, or cross-instance action IDs.
- Recheck inbox state before answering. Report already answered or expired items without sending a second response.
- Keep bot credentials in a protected secret source. Never put credentials in prompts, events, logs, or committed configuration.
- Allow output only in configured destinations. Document that prompts, results, and artifacts sent to Discord leave the machine.
- Disable unintended mentions in generated output. Bound message sizes and attachment sizes; respect Discord rate limits.
- Do not expose arbitrary shell commands, raw keystrokes, agent spawning, workflow editing, or server configuration through this integration.
- Record the Discord actor, action, destination, and resulting tower object for audit. Do not claim that a Discord actor is a separate tower security principal.

Use slash commands and components without reading ordinary channel messages. Do not request the privileged Message Content intent for this scope.

## Recovery and delivery rules

Persist thread bindings and notification progress across server restarts. Resume events from the saved cursor. Reconcile current object state when retained events cannot cover a gap.

Deduplicate commands by Discord interaction ID. Creation and its interaction mapping need an atomic boundary or an equivalent durable recovery rule. Do not blindly retry a mutation after an ambiguous result.

Persist notification work before advancing the event cursor. Retain unsent work during Discord outages. Respect rate limits and reconnect through the supported Discord client behavior. Reconcile ambiguous message sends; do not promise exactly-once delivery across Discord and SQLite.

Report inaccessible, deleted, or archived destinations to the operator. Do not redirect private output to another channel automatically. Tower work continues even if Discord cannot receive its result.

## Ordered implementation milestones

### M0 — Contracts and design

- **D0.1** Amend D§3, D§4, D§7, D§9, D§13, and D§15 before implementation. Define startup, configuration, authorization, audit, and event ownership.
- **D0.2** Record the Discord client library choice and Gateway intents in this plan. Confirm command registration, forms, threads, and required channel permissions.
- **D0.3** Define persistent mappings, deduplication, notification recovery, and prompt admission rules. Reject busy-agent prompts unless an existing safe delivery contract applies; offer job creation instead.
- **D0.4** Define result rendering from explicit messages, job results, and artifacts. Label terminal snapshots as snapshots, not assistant replies.

Verify: review the design changes against existing service contracts. Record decisions and unresolved API gaps here before code starts.

### M1 — Connection and access control

- **D1.1** Add opt-in configuration and secret loading. Keep existing startup behavior when Discord is disabled.
- **D1.2** Add the supervised Gateway connection and command registration. Disconnects must not stop tower services.
- **D1.3** Implement authorization and persistent interaction records. Add the read-only agent list command first.

Verify: install the bot in a private test guild. Exercise authorized and unauthorized commands. Disconnect Discord and confirm that local tower operations still work.

### M2 — Prompts, jobs, and workflow runs

- **D2.1** Add explicit prompts with safe admission checks and accurate acknowledgement text.
- **D2.2** Add job creation, assignment, inspection, and permission-controlled cancellation. Preserve waiting reasons and agent claims.
- **D2.3** Add workflow selection, typed input forms, run inspection, and cancellation.
- **D2.4** Add thread bindings and state notifications. Render results and permitted artifacts without terminal scraping.

Verify: create a job for a busy agent and observe reserved delivery. Start a workflow with required inputs. Observe progress and completion in the bound threads. Repeat an interaction and confirm that it creates no duplicate work.

### M3 — Inbox and recovery

- **D3.1** Add inbox questions, answer forms, and approval buttons. Recheck permissions and pending state on each action.
- **D3.2** Complete durable notification recovery, event replay, and destination error reporting.
- **D3.3** Document application creation, bot installation, permissions, secret rotation, configuration, and removal. Update the changelog and operator guidance.

Verify: answer a real agent question and approval. Repeat the action after another client answers it. Restart tower during active work and observe recovery. Exercise expired interactions, revoked access, unavailable channels, and rate limits.

Each milestone requires `cargo make lint` and `cargo make test`. Keep regression tests for authorization, state transitions, duplicate interactions, and recovery. Also run the actual bot in a private guild; mocks alone do not prove Discord behavior.

## Exit criteria

- An operator can install and configure one bot without exposing tower publicly.
- Authorized users can prompt agents, create jobs, start workflows, inspect work, and cancel permitted work.
- Busy-agent behavior and workflow claims match local tower behavior.
- Job and run threads receive the correct state changes and structured results.
- Authorized users can answer questions and approve or deny pending requests.
- Unauthorized or stale interactions cannot mutate tower state or reveal private output.
- Repeated commands and server restarts do not create duplicate jobs or runs.
- Discord outages do not stop accepted work; notifications recover without losing their destinations.
- Live Discord evidence and repository checks appear in the Verification log.

## Deferred capabilities

Free-form thread messages, direct messages, and mention-based commands require a separate conversation contract. Define response correlation, concurrent turns, and structured agent replies before adding them.

Also defer shared bots across instances, public HTTP interactions, workflow authoring, and administrative host controls. These are not requirements for completing this plan.

## Verification log

No implementation milestones have passed. This proposal does not claim a live Discord smoke run.

## Spike findings

Record dated findings, decisions, and design changes here when implementation research starts.
