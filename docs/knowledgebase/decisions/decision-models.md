---
type: Decision
title: Decision models — an optional System One layer for dispatch, inbox triage, and question routing
description: Tower may call a decision model (Clef, Jev, or any model behind the shared System One API) to rank agents for a job, triage the inbox, and route agent questions. It is off by default, never required, runs inside tower serve as a service principal, keeps hard rules in code, starts in shadow mode, and records every call.
tags:
  - decision
  - decision-model
  - dispatch
  - router
  - orchestrator
  - messaging
status: proposed
sources:
  - resource: https://docs.ollama.com/api/systemone
    title: Ollama System One API (/v1/systemone)
  - resource: https://docs.ollama.com/capabilities/decision
    title: Ollama decision models guide (local and cloud)
  - resource: https://blog.cloudflare.com/clef-decision-models/
    title: Introducing Clef (Cloudflare, 2026-10-01)
  - resource: https://typesafe.ai/blog/introducing-system-one-models-and-jev
    title: Introducing System One models and Jev (TypeSafe AI)
  - resource: https://arxiv.org/abs/2610.02267
    title: "Fast Models, Slow Evidence: System-1 decision models for LLM agent harnesses"
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-18T16:32:35Z"
---

# Decision models

Date: 2026-10-18. Resolves [§17.8](../concepts/design/17-open-questions.md)
and the question-triage part of [§17.9](../concepts/design/17-open-questions.md).
Amends rule 6 of the [job-queue decision](job-queue.md). Plan:
[phase 7](../plans/phase-7-decision-models.md).

## Context

- **Tower calls no model today.** The operator makes every dispatch and
  inbox decision (CLI, MCP, skills), or fixed code makes it
  (`tasks::dispatch_all`, the lease sweeper, cron schedules). The server
  crate has no outbound HTTP client. The cost of a decision is operator
  attention.
- **The design already reserves the slot.** Job-queue rule 6 and
  [§5.2.1](../concepts/design/05-core-objects.md) defer an orchestrator
  with an "agent router" (jev or laya). §17.9 defers a supervisor that
  would triage questions. Task `tags` are named "the future router's input
  signal".
- **Decision models share one API.** A request has a `state` and named
  typed `questions` (`choice`, `noul` yes/no, `score`). The response has a
  probability for every option. Clef is Jev-API compatible. Ollama serves
  decision models at `/v1/systemone`: locally (Nimble, Tev1, Clef Flash)
  and in Ollama's cloud (`clef:cloud` through a signed-in local daemon, or
  `https://ollama.com` with an API key). Cloudflare Workers AI and TypeSafe
  host the same request shape. Laya ships open weights.
- **Evidence is mixed by decision type** (arXiv 2610.02267, Cloudflare's
  published benchmarks):
  - Choosing among described candidates is strong. Jev scored 98.9% on tool
    selection with a unique correct tool. Clef and Clef Flash scored above
    98% on BFCL.
  - Predicting difficulty is not. Neither Jev nor Laya beat chance at
    cheap-versus-strong model routing.
  - Deciding "should a human handle this" is middling. On When2Call, Jev
    scored 81% and Clef 72%.
  - Laya changes 30% of its answers when the option order is reversed. It
    falls to 31% with 50 similar candidates. Agents in one stack have similar
    briefs.
  - Thresholds picked and scored on the same data missed their target. A
    5% miss target reached up to 17% on held-out data.
  - Gate savings came in well below naive accounting. The durable gain was
    latency.
- **Not everyone has access.** Ollama cloud models need a paid plan or
  credits. Hosted Jev needs an account. Tower must work the same without
  any of them.

## Decision

1. **Optional, off by default.** With no `[decision]` section, or with an
   endpoint that fails (timeout, 401, 402, 404, 5xx), tower behaves exactly
   as today: the operator decides. No decision point blocks on a model.
2. **A protocol, not a model.** One client speaks the System One request
   shape to a configured endpoint URL with a model name and an optional API
   key read from a named environment variable. That covers Ollama local,
   Ollama cloud, Workers AI, and Jev-compatible hosts with one code path.
   Tower names no default model. The "jev vs laya" question in §17.8 closes
   as "whichever the endpoint serves". Laya is usable only behind a
   compatible HTTP endpoint; in-process inference is not adopted.
3. **Built in means a server-side service, not an agent.** The decision
   layer is a background task inside `tower serve`, like the sweeper and the
   pump. It makes HTTP calls and spawns no process, so herdr stays the only
   execution substrate. It acts as a `PartyKind::Service` principal through
   the existing code paths (`tasks::assign`, `messaging::dismiss`), and its
   actions carry `source: decision` in their event payloads.
4. **The service holds the assign capability.** Job-queue rule 2 becomes:
   the operator or the decision service may set a job's owner. The
   exclusive-ownership CAS is unchanged, so a stale decision loses the race
   cleanly.
5. **Hard rules stay in code; the model only ranks.** Eligibility filters
   run first: the agent is idle, the job is not held, tags match, and the
   agent has no open job. The model never sees an ineligible option. Every
   `choice` question includes a `none` option, which leaves the item to the
   operator.
6. **Each decision point has a mode:** `off`, `shadow`, `suggest`, or `act`.
   - `shadow` records the answer and takes no action.
   - `suggest` shows the answer on operator surfaces.
   - `act` applies the answer when confidence meets that point's threshold;
     below the threshold it falls back to `suggest`.
   - Enabling the section without per-point modes means `shadow`.
   - Thresholds are set from recorded shadow data, never from the data they
     are scored on.
7. **Every call is recorded.** Each record stores the decision point, the
   subject, the model, the endpoint host, the state as sent, the
   probabilities, the confidence, the mode, the action taken, and the
   operator's later action on the same subject. Agreement between the model
   and the operator is the measure of value. Records are pruned with the
   event retention. API keys never enter records, logs, or events.
8. **Three decision points, in order of evidence:**
   - **Dispatch** — which eligible agent gets a queued job. State: the
     job's title, description, and tags, plus each candidate's stored brief
     and state. Ask "whose brief matches this job", not "who will succeed".
     Requires `agents.brief` (phase 2c).
   - **Inbox triage** — for pending agent questions: needs an answer, stale,
     or answered elsewhere, plus an urgency score. Mechanical staleness is
     handled in code first: the asker's job was cancelled or failed, or the
     asker was removed. The model handles the rest from the asker's later
     output and messages. Dismissal is silent to the asker, so `act` applies
     only above a high threshold.
   - **Question routing** — an agent's question goes to the operator or to
     another idle agent whose brief covers it. A forwarded question keeps a
     copy in the operator inbox, cannot be forwarded again, and returns its
     answer to the asker through `respond`. This point ships only if the
     phase-7 wording spike shows enough agreement on real tower history.
9. **Approvals are never decided by a model.** Approval items (permission
   prompts) are excluded from triage and routing in every mode.

### Not adopted

- **An LLM orchestrator agent in herdr for dispatch.** It spends a full
  generation per decision on a choice among a few options. §17.9 stays open
  for the parts a decision model cannot do (compaction and reset control).
- **In-process Laya inference** (`laya-rs`, candle): a heavy dependency for
  the weakest and most order-sensitive model in the published evaluations.
- **An LLM adapter as a fallback** (TypeSafe's System One wrapper over a
  chat model): it brings the LLM cost back, which is what this decision
  avoids. The fallback is the operator.
- **Model-chosen difficulty or model routing:** at chance in the published
  evaluations.
- **Still-working pings:** the lease already answers liveness (job-queue
  rule 5). Phase 7 adds none.

## Consequences

- The server crate gains an outbound HTTP client (workspace `reqwest`,
  already used by `tower-client`).
- A new `decisions` table (next free migration number) and a
  `decision.made` event kind.
- Design amendments at execution:
  - [§4](../concepts/design/04-paths-and-configuration.md): the
    `[decision]` config section and a `tower doctor` reachability check.
  - [§5.2.1](../concepts/design/05-core-objects.md): the assign capability
    includes the decision service.
  - [§6](../concepts/design/06-data-model.md): the `decisions` table.
  - [§7](../concepts/design/07-server-api.md): decision read routes and
    the MCP tool.
  - [§9](../concepts/design/09-server-modules.md): a decision module, and
    question forwarding in messaging.
  - [§10](../concepts/design/10-client-cli.md): `tower decisions`.
  - [§13](../concepts/design/13-security.md): API key by environment
    variable name only.
  - [§17.8](../concepts/design/17-open-questions.md) closed;
    [§17.9](../concepts/design/17-open-questions.md) narrowed.
  - [Job-queue](job-queue.md) rules 2 and 6.
- `docs/getting-started.md` gains a decision-model section. The
  `tower-status` skill shows suggestions. `docs/agent-loop.md` notes that
  another agent may answer a question.
