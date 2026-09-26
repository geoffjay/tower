---
type: Concept
title: Design §13 — Security
description: Socket, TCP, node, and A2A authentication; secret handling; file permissions.
tags:
  - design
  - design-s13
  - security
  - auth
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §13 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 13. Security

- Unix socket: filesystem permissions (0600 dir) are the auth; no token needed
- TCP: bearer token (generated first run, 0600); bind 127.0.0.1 by default
- Nodes: per-machine tokens issued by the operator
  (`tower machines add <name>` prints a token); WS over TLS or SSH tunnel
- A2A endpoint: bearer token required (declared in agent card auth)
- Never log/store secrets in events or message payloads; prompt text is stored
  (it is the work record) but not replicated to third parties
- SQLite, token, artifacts: 0600/0700 permissions, owner-only
