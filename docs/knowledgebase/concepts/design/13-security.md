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
- Web UI (phase 4, [§12.3](12-web-ui.md)): the browser never sees the bearer token. It
  holds a scoped read-only UI token, `hex(HMAC-SHA256(bearer token,
  "tower-ui-read-v1"))` — derived, so it rotates with the bearer token.
  It opens only `/ui` and `/ui/*` with `GET`/`HEAD` (incl. the runtime's
  WebSocket upgrade) and Topcoat's page re-render `POST`
  (`X-Topcoat-Runtime: true`, rewritten to `GET`); anything else is `401`.
  `tower ui` fetches it (`GET /v1/ui/token`, bearer) and prints a
  `/ui/login?token=…` link; login sets an HttpOnly, SameSite=Strict cookie
  scoped to `/ui` and redirects the token out of the address bar.
  Topcoat's origin policy rejects cross-origin WebSocket handshakes
- Nodes: per-machine tokens issued by the operator
  (`tower machines add <name>` prints a token); WS over TLS or SSH tunnel
- A2A endpoint: bearer token required (declared in agent card auth)
- Never log/store secrets in events or message payloads; prompt text is stored
  (it is the work record) but not replicated to third parties
- SQLite, token, artifacts: 0600/0700 permissions, owner-only
