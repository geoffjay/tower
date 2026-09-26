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
"Delivery" to an agent = driver `prompt`. Sweeper marks `pending`
questions/approvals `expired` at `deadline_at` and notifies the agent with an
`approval.expired` prompt ("proceed with defaults or stop").

## 9.4 tasks

Task rows + pool semantics ([§5.2.1](05-core-objects.md)): claim/pull (atomic CAS), lease sweeper
(10s tick: expiry → requeue + `task.leased_out`, max-attempts → `failed`),
heartbeat handling, owner-only terminal writes. Emits `task.*` events.
Task completion is owner-reported (prompt response or detection `done`) —
never inferred from a delivered message alone (openrig's epistemics rule).
The sweeper also pauses lease expiry during `input-required` (see [§5.2](05-core-objects.md)
state machine note).

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
