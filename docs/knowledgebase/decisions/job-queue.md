---
type: Decision
title: Job queue — assignment-only dispatch, no self-serve claims
description: Tasks are a job queue agents are assigned to; claim/pull self-serve endpoints are removed; lease+heartbeat remain the liveness mechanism; an orchestrator role with a decision-model router is deferred.
tags:
  - decision
  - task-pool
  - job-queue
  - dispatch
  - orchestrator
  - router
status: accepted
---

# Job queue — assignment-only dispatch, no self-serve claims

Date: 2026-09-26. Supersedes the shared task pool semantics in [design §5.2.1](../concepts/design/05-core-objects.md) (as originally ingested). Amends [§1 goals](../concepts/design/01-purpose-and-scope.md), [§6](../concepts/design/06-data-model.md), [§7](../concepts/design/07-server-api.md), [§9.4](../concepts/design/09-server-modules.md), [§10](../concepts/design/10-client-cli.md), [§14](../concepts/design/14-reliability.md), [§16](../concepts/design/16-testing.md), [§18](../concepts/design/18-phase-mapping.md).

## Context

The original design let any agent pull work from a shared pool atomically
(`POST /v1/tasks/pull`, `POST /v1/tasks/{id}/claim`). Experience with
multi-agent orchestration shows self-serve pools produce duplicated effort:
agents pick overlapping work, race, and waste tokens before the CAS rejects
the loser. Worse, "who should do this?" was nobody's decision — the pool
answered only "who got there first."

## Decision

1. **Tasks are a job queue, not a self-serve pool.** No agent can claim or
   pull a job. `claim` and `pull` endpoints/tools are removed from the
   design. A job enters `queued` and waits for assignment.
2. **Jobs are assigned, not claimed.** Only a principal with the
   **assign capability** (phase 2: the operator via CLI/MCP; later: an
   orchestrator agent) can set a job's owner. Assignment is the single
   dispatch decision point.
3. **Assignment is exclusive (CAS).** Assigning a job that already has a
   live owner returns `conflict` (409). The exclusive-ownership CAS from
   the old §5.2.1 survives — it guards assignments instead of claims.
4. **An explicit `assigned` state exists between `queued` and `working`.**
   Assignment sets owner + starts the lease; the agent then **declares**
   it is working on the job (`start` transition → `working`). This makes
   "assigned but never started" visible — the exact stall the next rule
   targets.
5. **Lease + heartbeat remain the liveness mechanism.** The owner
   heartbeats while working; lease expiry requeues the job
   (`attempt_count`/`max_attempts` unchanged). The lease is also the
   machine-checked form of "still working on it?" — the orchestrator does
   not need to ping agents to know they are alive.
6. **Orchestrator role, deferred.** A `role: orchestrator` agent may read
   the queue and assign jobs. It uses an **agent router** — a decision
   model that picks the best-suited agent from the roster — with
   `still-working` liveness checks (rate-limited; lease data makes these
   mostly unnecessary). Phase 2 ships the queue primitive with the
   operator as the assigning principal; the orchestrator agent + router
   are a later milestone. Router candidates: **jev** or **laya** — both
   have Rust libraries; laya can run locally via a GGUF build (llama.cpp /
   ollama) using `laya-rs`/`laya-rust` candle-based native inference, jev
   via hosted decision endpoints / `fuzzy-jev`. Not a phase-2 decision.
7. **Agents report on their own jobs.** Agents see jobs assigned to them
   (`--mine`), and `heartbeat`, `status`, `release` remain owner-only.
   Agent work loop: find my assigned job → declare start → heartbeat →
   report status → complete.

## Consequences

- Phase 2 exit criteria change: race tests move from "two agents pull,
  one wins" to "assign is exclusive; second assign conflicts; killed
  owner's lease expiry requeues; reassign; survivor completes."
- `tower task assign <id> <agent>` becomes the dispatch verb; `task
  create --assign` pre-assigns at creation. `claim`/`pull` disappear from
  CLI, REST, MCP, docs, and work-loop contract.
- MCP tools change: `tower_task_assign` (operator + future orchestrator)
  in; `tower_task_claim`/`tower_task_pull` out. Agent-facing set gains
  `tower_task_start`.
- New event `task.assigned` replaces `task.claimed`. `task.leased_out`
   stays (lease expiry is unchanged).
- Tags/priority keep their filtering/ranking role for the **assigner**,
  not for pull queries.
- TUI/web keep queue views; the "pool banner" becomes a queue banner.

## Why lease+heartbeat survives the reframe

The failure story was never about who may pick up work — it was about
knowing an owner is alive without pinging every agent. A lease is
machine-agnostic (agentd/hermes PID-liveness fails across hosts), expiry
is a timestamp comparison, and heartbeats piggyback on the agent's MCP
loop. Self-serve claiming added duplicate-work risk without adding
liveness; assignment-only keeps the good part and drops the waste.

## Rejected alternatives

- **Keep claim/pull behind a capability flag** — two dispatch modes means
  two sets of race semantics to test and explain; the flag would be off
  by default and the claim path would rot untested.
- **Availability signaling (agents mark themselves free)** — extra
  surface the router doesn't need yet; queue depth + agent state from
  detection (`idle`) is sufficient signal for assignment decisions.
- **Orchestrator agent in phase 2** — the queue primitive (assignment,
  exclusivity, lease, states) must be proven with the operator assigning
  before automating the assigning principal. Deferring keeps phase 2
  testable without a model in the loop.
