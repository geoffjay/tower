---
type: Concept
title: Design §8 — Harness layer
description: Harness adapter trait, HerdrDriver and TmuxDriver, Claude Code and pi specifics, harness discovery and adoption.
tags:
  - design
  - design-s8
  - harness
  - herdr
  - driver
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §8 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-09-26T22:59:25Z"
---

# 8. Harness layer

## 8.1 Adapter trait

```rust
#[async_trait]
pub trait Harness: Send + Sync {
    async fn start(&self, spec: &AgentSpec) -> Result<Handle>;
    async fn prompt(&self, h: &Handle, text: &str, wait: bool) -> Result<()>;
    async fn interrupt(&self, h: &Handle) -> Result<()>;
    async fn send_keys(&self, h: &Handle, keys: &[String]) -> Result<()>; // approvals 1/2
    async fn read(&self, h: &Handle, source: ReadSource) -> Result<ReadResult>;
    async fn snapshot(&self) -> Result<Vec<PaneAgentState>>;
    fn events(&self) -> BoxStream<HarnessEvent>;   // state changes, output
}
```

Two implementations:

1. **HerdrDriver** (primary): wraps herdr for one machine. Preferred transport:
   herdr socket (`SemanticFrame` v20). Risk: the socket protocol is not a
   published, stability-guaranteed API. Mitigation: v1 of the driver shells
   out to the `herdr` CLI (`herdr agent start/prompt/read/wait`, `herdr api
   snapshot --json`) with JSON parsing — stable flags, documented, slower.
   Socket mode behind the same trait once the protocol is validated.
   **Placement**: spawned agents get one tab each (labelled with the agent
   name, rooted at its workdir) in a dedicated `tower-agents` herdr
   workspace, created on first spawn — tower never splits panes inside the
   operator's own workspaces. `agent start` retries `agent_pane_busy`
   (≤10s) while the fresh tab's shell comes up.
2. **TmuxDriver** (fallback, phase 2+): for machines without herdr. Minimal
   PTY + `tail`-based reads, no detection (state = `unknown` unless manual).
   Exists only for escape; not a goal.

## 8.2 Claude Code specifics

- Launch in herdr pane with `claude --permission-mode acceptEdits`
  (config-overridable); `--dangerously-skip-permissions` only when the agent row
  has `permissions = "yolo"` (explicit, per-agent, recorded in events)
- Classic renderer preferred (scrollback; openrig's finding)
- tower never writes to `~/.claude.json` or hooks by default — trust and
  permission prompts are surfaced via `blocked` state, answered through the
  inbox (send-keys `1`/`2` on approval messages). Claude hook integrations
  (activity relay) are opt-in later.

## 8.3 pi (ohmypi) specifics

- Launch with kind `pi`; detection manifest maintained upstream by herdr
- Adapter drives it via prompt/read first-class; if pi's RPC runner mode is
  needed (openrig's Pi adapter pattern), add a `pi` extension to the trait
  behind config — investigate pi 0.87 RPC surface during phase 1 spike

## 8.4 Harness discovery

`herdr api snapshot` on boot → reconcile with `agents` table:
match by pane id, then by name. Unknown agents appear in inventory as
`adopted: true` candidates (openrig's discover/adopt pattern);
`POST /v1/agents` with `adopt: <name>` takes ownership without relaunching.
Unnamed herdr agents (no `name` in the snapshot) are skipped — they aren't
addressable by name, so they can be neither owned nor adopted.
