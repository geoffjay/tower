---
type: Plan
title: Slack integration
description: Proposed Socket Mode app for agent prompts, jobs, workflow runs, and inbox responses, with scoped access and durable notifications.
tags:
  - slack
  - integrations
  - messaging
  - workflows
status: draft
generated:
  by: claude-code/1.0
  at: "2026-11-11T00:00:00Z"
sources:
  - resource: docs/knowledgebase/concepts/design/07-server-api.md
    title: Tower operations and event contracts
  - resource: https://docs.slack.dev/apis/events-api/using-socket-mode/
    title: Slack Socket Mode
  - resource: https://docs.slack.dev/interactivity/implementing-slash-commands/
    title: Slack slash commands
  - resource: https://docs.slack.dev/interactivity/handling-user-interaction/
    title: Slack interactions and response limits
---

# Slack integration

## Status and goal

Proposed; implementation has not started. Commands and configuration below describe new behavior, not existing features.

Add Slack as a remote control surface for one tower instance. Users can prompt agents, create jobs, start workflows, and answer inbox items. Tower publishes progress and results in work threads. Agents continue to run through herdr.

This is an alternative to the [Discord plan](discord-integration.md), not a dependency on its implementation. See [Slack and Discord findings](../references/slack-discord-integrations.md) for platform differences and the selection recommendation.

## Existing contracts and boundaries

- [D§3](../concepts/design/03-process-and-deployment.md): keep server state in `tower serve`; run agents through herdr.
- [D§5](../concepts/design/05-core-objects.md): preserve job, message, and run lifecycle rules.
- [D§7](../concepts/design/07-server-api.md) and [D§9](../concepts/design/09-server-modules.md): reuse existing service operations and events.
- [D§13](../concepts/design/13-security.md): do not expose the server bearer token. It does not represent individual Slack users.
- [Agent availability](../decisions/agent-availability.md): preserve reserved delivery and workflow claims.

The current prompt endpoint reports agent state, not a structured final answer. Use explicit messages, job results, and artifacts for result delivery. Label terminal snapshots as snapshots. Do not infer completion or answers from terminal text.

## Proposed architecture

Run an optional Slack adapter under `tower serve`. Receive commands and interactive payloads through outbound Socket Mode. Use the Slack Web API to post and update messages. No public inbound endpoint is required for this single-workspace installation.

Use one Slack app for one tower instance and one workspace initially. Install it through Slack's app settings. Keep the app-level token and bot token in protected secret sources. Do not build a public OAuth callback for this scope.

Keep Slack protocol code separate from tower service logic. Call service operations directly, not shell commands or a second job engine. An external REST/SSE adapter remains possible, but adds a service and credential boundary.

Slack refreshes Socket Mode connections. Reconnect without interrupting tower work. Multiple connections for one app can receive different payloads; they do not identify different tower instances. Do not reuse one app across independent instances without a routing design.

If both platforms ship, reuse existing tower operations and consistent authorization rules. Share durable delivery code only when their contracts match. Do not require a generic plugin framework or a second adapter before shipping this one.

## Installation and permissions

Provide an app manifest with Socket Mode, interactivity, a bot user, and the `/tower` slash command. Make the command name configurable to avoid collisions with other apps.

Start with these permissions:

| Credential | Scope | Purpose |
|---|---|---|
| App-level token (`xapp-`) | `connections:write` | Open Socket Mode connections |
| Bot token (`xoxb-`) | `commands` | Install the slash command and shortcuts |
| Bot token (`xoxb-`) | `chat:write` | Post and update the bot's messages in joined channels |

Invite the bot to each configured destination, including private channels. Do not request user tokens, channel history, broad public posting, or ordinary message subscriptions for the initial command interface.

Make file delivery optional. Request `files:write` only if artifact uploads are enabled. Choose its supported upload API during implementation; do not use the retired `files.upload` method. Never publish localhost links as remotely accessible artifacts.

Check workspace approval policy before installation. Workspace access alone does not establish permission to install the app or send source code to Slack. Keep direct messages, external shared channels, and organization-wide installations outside the initial scope. Require dedicated, non-shared destination channels.

## User experience

Slack passes everything after `/tower` as text. Parse a small explicit grammar; do not promise Discord-style typed slash options or argument autocomplete. Use modal select menus to choose agents and workflows.

| Proposed command | Behavior |
|---|---|
| `/tower help` | Explain available commands and access rules |
| `/tower agents` | Show permitted agents and their states |
| `/tower prompt` | Open an agent selector and prompt form |
| `/tower job create` | Open a job form with optional assignment |
| `/tower job show <id>` | Show state, waiting reason, and result |
| `/tower workflow run` | Select an existing workflow and collect declared inputs |
| `/tower run show <id>` | Show run state and step progress |
| `/tower inbox` | Show permitted pending questions and approvals |

Acknowledge commands and actions within three seconds. Open modals while the single-use `trigger_id` remains valid; it expires after three seconds too. Open a lightweight form first if loading data could exceed that deadline. Treat modal errors as input errors, not accepted work.

Use an ephemeral acknowledgement for the requester. Publish accepted work to its configured channel with explicit visibility. Ephemeral responses are not the durable job record.

Post one parent message per job or run. Store its channel ID and timestamp. Reply using `thread_ts`; update the parent with current state. Include requester, tower object ID, state, and permitted actions. Provide accessible fallback text for Block Kit messages.

Slack app slash commands cannot run inside message threads. Put **Status**, **Cancel**, and **Follow up** buttons on work messages instead. **Follow up** opens a new explicit prompt form with the target context. It does not forward ordinary thread replies automatically.

Questions use an **Answer** button and text modal. Approvals use **Approve** and **Deny** buttons. Route these to existing inbox items. Recheck pending state and authorization before responding. Disable stale controls after another client answers.

Use bot-authenticated messages for long work. A `response_url` permits up to five responses within thirty minutes. It is not a durable notification address. Never log that URL or bypass destination policy with it.

## Access and output policy

- Require allowed app, workspace, channel, and user IDs. Deny unknown contexts by default.
- Define separate capabilities for viewing, prompting, creating work, answering, approving, and cancelling.
- Use explicit user grants initially. Slack user groups are not Discord roles; defer group lookup and its extra scopes.
- Reauthorize commands, option requests, button clicks, and modal submissions. Do not treat channel membership or installation approval as authority to run work.
- Bind action state to the instance, requester or permitted actors, destination, and tower object. Do not trust object IDs from editable modal metadata.
- Apply safe prompt admission under the same concurrency boundary as other deliveries. Reject unsafe busy-agent prompts and offer queued jobs instead.
- Keep secrets, `response_url` values, and temporary connection URLs out of events, logs, prompts, and committed configuration.
- Record actor IDs, action, destination, and resulting tower IDs for audit. Do not claim a separate tower security principal for each Slack user.
- Send only permitted results and artifacts to configured channels. Explain that exported content leaves the machine and follows Slack retention policy.
- Escape untrusted output, suppress unintended mentions, and disable automatic link and media unfurls. Bound message and file sizes.
- Exclude shell execution, raw keystrokes, agent spawning, workflow authoring, and host administration.

## Durable delivery and recovery

Store app/workspace IDs, actor IDs, command receipts, tower object mappings, parent message timestamps, event cursors, and pending notifications. Preserve Slack timestamps as opaque strings.

Acknowledge Socket Mode envelopes, but do not confuse transport acknowledgement with durable acceptance of work. Define a stable deduplication key for each payload type. Use `event_id` for Events API events where present. Commands and actions need explicit receipt and submission identities; record these contracts before implementation.

Persist an authorized command before claiming that tower accepted it. Couple object creation to its receipt mapping atomically, or define equivalent durable recovery. Repeated envelopes and repeated modal submissions must not create duplicate jobs or runs. Do not blindly retry mutations after ambiguous outcomes.

Persist pending notifications before advancing tower's event cursor. Replay retained events after restart; reconcile object state when retention leaves a gap. Tower remains authoritative even when Slack history expires.

Respect per-method limits and `Retry-After`. Coalesce progress updates rather than posting terminal frames. Slack generally permits one new message per second per channel, with other limits also applying.

Report revoked tokens, missing membership, archived destinations, and deleted messages to the operator. Do not move private output to another channel automatically. Reconcile ambiguous Slack sends without claiming exactly-once delivery across Slack and SQLite.

## Ordered implementation milestones

### M0 — Contracts, installation, and client choice

- **S0.1** Amend D§3, D§4, D§7, D§9, D§13, and D§15 before code changes. Define configuration, supervision, authorization, audit, and event ownership.
- **S0.2** Review the community `slack-morphism` client as a candidate for Rust Socket Mode and Block Kit support. Record compatibility, maintenance, and recovery findings before selection. Slack's official Bolt examples use JavaScript, Python, and Java.
- **S0.3** Validate the minimal manifest, scopes, installation policy, private-channel membership, and Socket Mode setup in a test workspace.
- **S0.4** Specify receipt identities, mutation boundaries, notification recovery, and prompt admission. Record any shared contracts with the Discord plan.

Verify: record the approved design and a setup walkthrough. Do not require a public tower port or channel history to receive commands.

### M1 — Connection and authorized queries

- **S1.1** Add opt-in configuration and protected token loading. Preserve startup behavior when Slack is disabled.
- **S1.2** Add supervised Socket Mode reception, envelope acknowledgement, and connection refresh.
- **S1.3** Add authorization, durable receipts, help, and agent listing. Return private errors for denied requests.

Verify: install the app in a test workspace. Run allowed and denied commands. Refresh the socket and revoke credentials. Confirm that local tower operations continue.

### M2 — Prompts, jobs, workflows, and work threads

- **S2.1** Add prompt and job forms. Preserve reservations, run claims, and accurate waiting reasons.
- **S2.2** Add workflow selection and typed input validation. Add job/run inspection and permission-controlled cancellation.
- **S2.3** Add parent messages, thread mappings, state notifications, and structured results. Add button-driven follow-up forms.
- **S2.4** Add optional artifact delivery with explicit scopes and output limits, if enabled by configuration.

Verify: reserve a job for a busy agent. Start a workflow with required inputs and observe its result. Use thread buttons without slash commands. Repeat submissions and confirm one mutation. Complete work after the original response URL expires.

### M3 — Inbox, recovery, and operating guide

- **S3.1** Add answers, approval decisions, and stale-control handling through existing inbox operations.
- **S3.2** Complete replay, durable notification delivery, rate-limit handling, and destination error reporting.
- **S3.3** Document manifest installation, token rotation, access rules, channel invitations, output policy, and removal. Update the changelog.

Verify: answer a real agent question and approval. Repeat an action after another client answers it. Restart tower during work. Exercise delayed modals, duplicate payloads, rate limits, missing destinations, and revoked access.

Each milestone requires `cargo make lint` and `cargo make test`. Keep regression tests for authorization, duplicate mutations, state changes, and recovery. Also exercise the actual app in Slack; mocks alone do not prove the interface.

## Exit criteria

- An operator installs the app without exposing tower publicly or granting broad message access.
- Authorized users can prompt agents, create jobs, run workflows, inspect work, and cancel permitted work.
- Busy-agent delivery and workflow claims match local tower behavior.
- Buttons and modals work from job/run threads despite the slash-command restriction.
- Thread destinations and notification progress survive restarts and expired response URLs.
- Authorized users answer questions and approve or deny pending requests exactly through tower's existing state rules.
- Unauthorized, stale, cross-workspace, and repeated interactions cannot create unintended work or disclose private output.
- Slack failures do not stop accepted tower work. Recovery does not create duplicate jobs or runs.
- The operating guide and Verification log include real Slack evidence and repository checks.

## Deferred capabilities

Defer ordinary message forwarding, mentions, direct messages, shared external channels, user-group grants, App Home dashboards, and Workflow Builder steps. Slack Workflow Builder is not tower's workflow engine.

Also defer public distribution, a hosted OAuth installer, multiple workspaces, and shared apps across tower instances. Socket Mode currently cannot appear in the public Slack Marketplace.

## Verification log

No implementation milestone has passed. Research used official documentation on 2026-11-11. No live app installation, token use, or Slack smoke run is claimed.

## Spike findings

Record dated implementation findings and design changes here. Platform selection findings belong in the linked comparison reference.
