---
type: Concept
title: Design §1 — Purpose and scope
description: What tower is, target harnesses, goals, non-goals, and the naming note.
tags:
  - design
  - design-s1
  - scope
  - goals
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §1 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 1. Purpose and scope

Status: draft v0.1 (2026-09-26). Part of the [tower design](./).
Inputs: [README](../../../../README.md), [architecture recommendations](../../decisions/architecture-recommendations.md), research ([agentd](../../references/agentd.md), [herdr](../../references/herdr.md), [openrig](../../references/openrig.md), [IPC](../../decisions/ipc.md), [cross-server](../../decisions/cross-server.md), [A2A](../../references/a2a.md)).

tower is a single-server system for running and supervising multiple coding
agents. It provides one coordination point (server + database + event log), a
thin client, a read-only web UI, a TUI layered on top of herdr, and standard
agent-facing interfaces (MCP, A2A).

Target harnesses: **Claude Code** and **pi (ohmypi)**, executed in
**herdr**-managed panes.

## Goals

1. One server binary, one port, one database — agentd's module lesson
2. Spawn, supervise, and communicate with agents across machines
3. Stream agent output and system events over SSE
4. Human attention routed by blocked-state detection (questions/approvals)
5. Interfaces: CLI (humans), MCP (agents), SSE (UIs), A2A (foreign agents)
6. Everything durable: messages, tasks, and events are rows before delivery
7. Agents survive server loss (herdr owns the PTYs, not tower)
8. Job queue: agents receive assigned work, own it exclusively (atomic
   assignment + lease + heartbeat), and report status themselves — no
   agent self-serves from the queue

## Non-goals

- No terminal multiplexer (herdr owns PTYs, detection, layout persistence)
- No web configuration or control (web UI is read-only monitoring)
- No distributed database (single coordinator; nodes execute, not vote)
- No model/provider proxying or cost accounting in v1
- No multi-user auth in v1 (single operator; token auth only)

## Naming note

> **Naming note**: the metaphor is air traffic control — tower is the
> control-and-visibility layer over the herd of agents (radar = agent cloud,
> clearances = prompts, holding = blocked, squawks = inbox). `tower` is taken
> on crates.io (the Tower middleware library); if this project is ever
> published, the crates would need a qualifier (e.g. `tower-atc` or similar),
> or the ecosystem conflict is accepted for private use. HTTP middleware in
> this project is axum, unrelated to that crate.
