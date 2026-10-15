---
type: Concept
title: Design §9 — Server modules
description: Responsibilities of the inventory, sessions, messaging, tasks, machine hub, and event bus modules.
tags:
  - design
  - design-s9
  - server
  - modules
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §9 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 9. Server modules

## 9.1 inventory

Owns `agents` + `machines` rows. Boot sequence: ensure `local` machine row →
driver snapshot → reconcile → emit `agent.state` events for drift.

## 9.2 sessions

Owns spawn/prompt/interrupt/stop flow, translating API calls to driver calls
and emitting events. One task per in-flight driver call per agent (serialized
per agent; herdr prompts wait on detection anyway).

## 9.3 messaging

Unified message store + delivery. "Delivery" to a human = the message appears
in inbox queries + TUI + web UI (they poll/SSE; no push channel in v1).
"Delivery" to an agent = driver `prompt`; approval responses answer the
dialog on screen ([§8.2](08-harness-layer.md): deny = `esc`, approve = the first plain
"Yes"). A failed driver delivery marks the
message `failed`. Blocked-agent inbox items include the captured pane context
and use `[inbox].blocked_deadline_s` (default 24 hours); `tower inbox` shows
that context, and the TUI shows it in the selected item's detail. When an
agent leaves `blocked`, its tagged pending inbox item is closed as `answered`
without a prompt. The state transition and deadline notification share the
per-agent driver lock, so the sweeper rechecks the current state before acting.
For a due blocked-inbox item, only a still-blocked agent is notified; other
questions/approvals keep the general expiry behavior. The sweeper marks due
`pending` questions/approvals `expired` at `deadline_at` and emits an
`approval.expired` event: an expired question prompts the agent
("proceed with defaults or stop"); an expired approval is **denied** (keys
`esc`) — an unattended permission is never granted, and text typed into a
permission menu is undefined.

## 9.4 tasks

Task rows + job-queue semantics ([§5.2.1](05-core-objects.md)): assign
(atomic CAS, assign-capable principals only), start/heartbeat/status
(owner-only), lease sweeper (10s tick: expiry → requeue +
`task.leased_out`, max-attempts → `failed`). Emits `task.*` events.
Task completion is owner-reported (prompt response or detection `done`) —
never inferred from a delivered message alone (openrig's epistemics rule).
The sweeper also pauses lease expiry during `input-required` (see [§5.2](05-core-objects.md)
state machine note). The **dispatcher** delivers reserved jobs to available
targets (on the agent's `idle`/`done` transition, when its job closes, and
each sweep); the **scheduler** fires due schedules each sweep
([§5.2.2](05-core-objects.md)).

## 9.5 machine hub

Node connection manager: accepts outbound websocket upgrades at
`/nodes` (node → coordinator), authenticates per-machine tokens, multiplexes
driver calls to node agents, relays their events into the local log with
`machine_id` tagging. Node protocol: length-prefixed JSON frames over one WS
connection (upstream events; downstream RPC). Coordinator loss = nodes idle;
agents keep running (herdr); node reconnects and resyncs from snapshot.

## 9.6 event bus

Fan-out of the events table to SSE subscribers; pruning per retention config;
owns cursor semantics and heartbeats.
