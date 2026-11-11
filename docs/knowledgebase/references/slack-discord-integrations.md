---
type: Reference
title: Slack and Discord integration findings
description: Evidence-based comparison of Slack Socket Mode and Discord Gateway for a self-hosted tower control surface.
tags:
  - slack
  - discord
  - integrations
status: draft
generated:
  by: claude-code/1.0
  at: "2026-11-11T00:00:00Z"
sources:
  - resource: https://docs.slack.dev/apis/events-api/using-socket-mode/
    title: Socket Mode setup and distribution constraints
  - resource: https://docs.slack.dev/interactivity/implementing-slash-commands/
    title: Slack command text and thread restrictions
  - resource: https://docs.slack.dev/interactivity/handling-user-interaction/
    title: Slack deadlines and temporary response URLs
  - resource: https://docs.slack.dev/reference/scopes/connections.write/
    title: App-level connection permission
  - resource: https://docs.slack.dev/reference/methods/chat.postMessage/
    title: Bot messages, thread replies, and accessibility
  - resource: https://docs.slack.dev/apis/web-api/rate-limits/
    title: Slack message limits and retry policy
  - resource: https://slack.com/help/articles/222386767-Manage-app-approval-for-your-workspace
    title: Workspace app approval
  - resource: https://slack.com/help/articles/115002422943-Usage-limits-for-free-workspaces
    title: Slack free workspace limits
  - resource: https://docs.discord.com/developers/interactions/application-commands
    title: Discord typed commands and autocomplete
  - resource: https://docs.discord.com/developers/interactions/receiving-and-responding
    title: Discord interaction transport and response deadlines
---

# Slack and Discord integration findings

## Scope and recommendation

Research date: 2026-11-11. This comparison covers one self-hosted tower instance and a private Slack workspace or Discord server. It does not claim live integration testing or measured development time.

Both platforms can support agent prompts, jobs, workflow runs, progress notifications, and inbox answers. Neither requires exposing tower publicly when using its outbound socket transport.

[INFERENCE] Prefer Discord for a personal installation when server control and native command discovery matter most. Prefer Slack when the intended operators already coordinate work there and can approve a custom app. Slack offers no clear backend implementation shortcut for tower's Rust server.

Choose one adapter first. Access policy and result correlation matter more than the initial WebSocket connection. The plans are alternatives, not a requirement to implement both:

- [Discord integration plan](../plans/discord-integration.md).
- [Slack integration plan](../plans/slack-integration.md).

## Platform comparison

| Concern | Slack app | Discord bot | Consequence for tower |
|---|---|---|---|
| Local hosting | Socket Mode receives commands and components over an outbound WebSocket | Gateway receives interactions over an outbound WebSocket | Neither needs a tunnel or public tower port |
| Credentials | App-level connection token plus workspace bot token | Bot token for the Gateway and bot API operations | Slack has two runtime token types to configure |
| Command input | Text after `/tower` is one string | Named, typed options and optional argument autocomplete | Discord provides more command validation and discovery in the client |
| Commands inside threads | Custom slash commands cannot run there | No equivalent Slack-style ban; command availability still depends on context and permissions | Slack needs buttons or shortcuts for work-thread actions |
| Forms and controls | Block Kit messages, buttons, select menus, and modals | Messages, components, and modals | Both support job forms and approval actions |
| Immediate acknowledgement | Three seconds | Three seconds | Do not wait for tower work to finish |
| Temporary replies | `response_url`: up to five responses within thirty minutes | Interaction tokens: fifteen minutes | Use ordinary bot messages for long-running work on either platform |
| Thread identity | Parent message timestamp plus channel ID | Thread/channel ID, with associated message IDs | Persist platform-specific destination mappings |
| Actor policy | Workspace/channel/user IDs; user groups need extra design | Guild/channel/user/role IDs | Do not assume equivalent role systems |
| Installation control | Workspace owners can require app approval | Installer needs permission to authorize the app in the server | Access to either platform does not guarantee installation authority |
| Public distribution | Socket Mode apps cannot currently appear in the public Slack Marketplace | Guild installation is a normal application context | Slack Marketplace distribution needs a different transport plan |

Sources: [Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/), [Slack commands](https://docs.slack.dev/interactivity/implementing-slash-commands/), [Slack interactions](https://docs.slack.dev/interactivity/handling-user-interaction/), [Discord commands](https://docs.discord.com/developers/interactions/application-commands), and [Discord interactions](https://docs.discord.com/developers/interactions/receiving-and-responding).

## What a Slack app would look like

Run `/tower job create` in a channel. The app opens a form for the title, prompt, and target agent. It acknowledges submission privately, then posts a job card in a configured channel.

The card becomes the parent of the work thread. It shows state and waiting reasons, then receives progress and the result. **Status**, **Cancel**, and **Follow up** buttons work inside the thread. A follow-up opens an explicit prompt form; ordinary replies do not become instructions.

An inbox question offers **Answer**. An approval offers **Approve** and **Deny**. The app checks the actor and current tower state before applying either action.

Slack's [message API](https://docs.slack.dev/reference/methods/chat.postMessage/) supports `thread_ts`, Block Kit, and accessible fallback text. This interface does not require channel-history access or an AI assistant feature from Slack.

## Ease of implementation and operation

### Setup

Slack can use an app manifest for repeatable setup. The operator still installs the app, obtains both tokens, and invites its bot to destination channels. Scope changes can require installation changes. See [Socket Mode setup](https://docs.slack.dev/apis/events-api/using-socket-mode/) and [app approval](https://slack.com/help/articles/222386767-Manage-app-approval-for-your-workspace).

[INFERENCE] Discord is likely simpler when the operator owns the server. Slack approval is an external dependency in a managed workspace. The actual workspace policy was not inspected during this research.

### Client libraries

Slack recommends Bolt and SDKs for JavaScript, Python, and Java in its Socket Mode guide. These examples do not establish an official Rust framework. A Rust adapter needs a client-library review and a live protocol smoke run.

The community [slack-morphism client](https://docs.rs/slack-morphism/latest/slack_morphism/) documents Socket Mode, Web API, and typed Block Kit support. A Rust implementation does not need to start with raw WebSockets. Treat this as a candidate, not a selected dependency; review compatibility, maintenance, and recovery behavior during M0.

[INFERENCE] A standalone Bolt bridge could shorten an initial Slack experiment. It also adds a process, packaging, and operator credentials. The proposed plan keeps the adapter under `tower serve` to preserve the current deployment model.

No client-library benchmark or implementation spike supports a claim that one complete Rust adapter is faster to build.

### Team use and operating limits

[INFERENCE] Slack has an adoption advantage when code review, incidents, and approval requests already happen in that workspace. Discord has an adoption advantage when agents serve a personal or community server. Neither platform provides tower's per-user authorization automatically.

Slack generally permits one new message per second per channel. Honor method-specific limits and `Retry-After`; coalesce progress rather than forwarding terminal output. See [rate limits](https://docs.slack.dev/apis/web-api/rate-limits/).

Slack free workspaces allow up to ten third-party or custom apps. Visible/searchable history covers the last ninety days; older content follows the documented retention limits. See [free workspace limits](https://slack.com/help/articles/115002422943-Usage-limits-for-free-workspaces). Tower must remain the work record, not Slack history. Recheck platform plans and retention policies before deployment.

Slack Socket Mode's Marketplace restriction does not prevent a private internal installation. Public distribution is a different requirement; do not design a hosted installer for the single-workspace case.

## Shared tower work

Both adapters need the same service contracts, not a second execution engine:

- Authorization at every command, button, and form submission.
- Safe prompt admission, queued jobs for busy agents, and existing workflow claims.
- Structured messages, results, and artifacts instead of guessed terminal answers.
- Durable command receipts, object mappings, and pending notifications.
- Event replay and state reconciliation after disconnection or restart.
- Explicit destinations, secret protection, audit records, and output limits.

These requirements dominate the safe implementation. Neither chat platform fixes the current prompt endpoint's lack of a structured final response.

Keep provider identities, thread mappings, and interaction deadlines separate. Reuse tower services first; introduce shared adapter code only for demonstrated common behavior.

## Selection gate before implementation

Confirm installation authority, permitted data destinations, and the operators' preferred platform. Check the chosen Rust client and test its socket lifecycle. Record the selected platform in its plan before M1.

No app was installed, no credentials were used, and no workspace or server policy was inspected. The findings establish documented capabilities and design tradeoffs, not deployment readiness.
