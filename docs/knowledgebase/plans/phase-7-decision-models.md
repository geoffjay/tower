---
type: Plan
title: Phase 7 — Decision models
description: An optional System One decision layer inside tower serve — dispatch ranking, inbox triage, and question routing — off by default, shadow mode first, every call recorded, operator fallback when no model is reachable.
tags:
  - plan
  - phase-7
  - decision-model
  - dispatch
  - messaging
status: draft
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-18T16:32:35Z"
---

# Phase 7 — Decision models

Goal: when a decision model is available, tower uses it to cut operator
decisions; when none is, nothing changes. A configured System One endpoint
(Ollama local or cloud, Workers AI, or a Jev-compatible host) ranks agents
for queued jobs, triages pending questions, and routes agent questions to
another agent. Each decision point runs `off`, `shadow`, `suggest`, or
`act`. Every call is recorded beside the operator's own action, so value is
measured on this fleet, not taken from vendor benchmarks. Decision:
[decision models](../decisions/decision-models.md) — proposed, to be
accepted with this plan.

Exit criteria ([design §18](../concepts/design/18-phase-mapping.md)): with
no `[decision]` section, tower behaves as before and the existing suite
passes unchanged; with a configured endpoint, every decision point records
its probabilities in shadow mode beside the operator's action; in `act`
mode a queued job is assigned to the brief-matching idle agent, a stale
question is dismissed, and an agent's question is answered by another agent
with the asker receiving the answer — each with a decision record; an
unreachable or unpaid endpoint (timeout, 401, 402) falls back to operator
behavior with a logged warning; no approval is ever answered by the model.

Depends on: phase 2c (`agents.brief` is the dispatch and routing signal; the
question hold and inbox contract define what triage may close). Independent
of phases 5 and 6. Touches the CLI, TUI, and web surfaces from phases 3–4.

---

## Milestone 0 — Spikes

- **S7.1 Endpoint compatibility.** Send the same tower-shaped requests to
  Ollama local (`nimble`, `clef-flash`) and Ollama cloud (`clef:cloud`
  through a signed-in daemon, and `https://ollama.com` with a key). Add a
  hosted Jev-compatible endpoint if an account is available. Record:
  - request and response differences (the `confidence` field's scale, the
    `noul` shape, option limits per model);
  - the error codes for missing sign-in, no credits, and an unknown model;
  - p50/p95 latency from this machine.
  Output: the `[decision]` config shape and the error-to-"unavailable"
  mapping. Confidence is computed client-side (max probability) if the
  endpoints disagree on its scale.
- **S7.2 Question wording on real history.** Build a hand-labeled set from
  this repo's own tower database: past assignments (job → owner), inbox
  items and how they ended (answered, dismissed, expired), and agent
  questions another agent could have answered. Run two to four wordings per
  decision point against S7.1's endpoints. Record agreement, calibration,
  and order sensitivity (options reversed). Output:
  - question text and the state layout per point;
  - starting thresholds from a held-out half;
  - go/no-go for Milestone 4. On no-go, move Milestone 4 to the backlog and
    amend the §18 exit criteria first.

## Milestone 1 — Decision layer core
([D§4](../concepts/design/04-paths-and-configuration.md), [D§6](../concepts/design/06-data-model.md), [D§9](../concepts/design/09-server-modules.md), [D§13](../concepts/design/13-security.md))

- **T1.1 Config.** An optional `[decision]` section in `config.rs`
  (`serde(default)`, absent = disabled):
  - `endpoint` (full URL), `model`, `api_key_env` (the variable name, never
    the key), `timeout_ms`;
  - `[decision.points.<name>]` with `mode` and `threshold`.
  Unknown point names are rejected, naming the key. Verify: config tests
  for absent, minimal (all points `shadow`), full, and unknown-key cases.
- **T1.2 Client + fake.** A `decide` module in `tower-server`: a System One
  request builder (choice, noul, score) and a `reqwest` client. Timeouts,
  connect errors, and 401/402/404/5xx map to `Unavailable` with the
  provider's message; the key goes only into the auth header. A
  `FakeDecider` with scripted answers follows the `FakeHarness` pattern
  ([D§16](../concepts/design/16-testing.md)). Verify: request JSON
  matches S7.1's captured shapes; each error code → `Unavailable`; the key
  is absent from logs and errors.
- **T1.3 Decision records.** Migration (next free number): a `decisions`
  table with id, point, subject kind and id, model, endpoint host, state,
  probabilities, confidence, mode, action, `operator_action`, created_at,
  and resolved_at. Add a `decision.made` event kind, and prune with
  `event_retention_days`. The operator's later action on the same subject
  (assign, respond, dismiss) fills `operator_action`. Verify: migration
  applies on a phase-2c database; checksum test pins it; prune honors
  retention; an operator assign after a shadow record fills it.
- **T1.4 Doctor + startup.** `tower doctor` adds "decision endpoint
  reachable" when configured: a one-question probe that reports
  unavailable/unauthorized/no-credits plainly. On startup, an unreachable
  endpoint logs one warning, and the layer retries with backoff. Verify:
  doctor output for reachable, 401, and absent config.
- **T1.5 Read surfaces.** `GET /v1/decisions?point=&subject=` and
  `tower decisions [--point P] [--json]`. Add `tower decisions stats` with
  agreement rate, the confidence histogram, and calls per point over the
  retention window. Add the MCP tool `tower_decisions` (read-only). Verify:
  route and CLI `--json` shape tests; stats on a seeded table.

## Milestone 2 — Dispatch
([D§5.2.1](../concepts/design/05-core-objects.md), [job-queue decision](../decisions/job-queue.md))

- **T2.1 Service principal.** The decision service acts as
  `PartyKind::Service` and may call `tasks::assign`; agent callers still may
  not. Events carry `source: decision`. Verify: the service assign succeeds;
  an agent MCP assign is still refused; the CAS conflict path is unchanged.
- **T2.2 Candidates and question.** Triggers: an unassigned queued job
  appears, an agent goes idle, or a sweeper tick finds queued work.
  Eligible agents are idle, have no open job, match the job's tags, and the
  job is not held. With zero eligible agents, nothing is called; with one,
  the model is still asked so `none` stays possible. The question is
  `choice` over the eligible agents plus `none`, with the state from S7.2's
  layout (job title, description, tags; each candidate's brief). Verify:
  filter unit tests (held, busy, tag mismatch, owned); no call with zero
  candidates.
- **T2.3 Modes.**
  - `shadow`: records only.
  - `suggest`: `task show`, `task list`, the TUI jobs view, and MCP
    `tower_task_show` show the suggested owner and its confidence.
  - `act`: assigns via `tasks::assign` at or above the threshold. The
    delegation notice is unchanged, and a lost CAS is recorded and not
    retried.
  - `none`, or below threshold: the job waits for the operator.
  Verify: `FakeDecider` tests per mode; act below threshold → suggestion
  only; act racing an operator assign → one owner, a recorded conflict.

## Milestone 3 — Inbox triage
([D§5.3](../concepts/design/05-core-objects.md), [D§9.3](../concepts/design/09-server-modules.md))

- **T3.1 Mechanical staleness, no model.** Close a pending question when
  the asker's job was cancelled or failed, or the asker was removed. Use the
  `close_blocked_inbox` pattern, with `source: job_state` in the event. This
  runs whether or not `[decision]` is configured. Verify: clock-injected
  tests for each rule; questions on live jobs untouched; approvals
  untouched.
- **T3.2 Model triage.** For each pending agent question, ask a `choice`
  question (needs-answer, stale, answered-elsewhere) and a `score` question
  for urgency. The state is the question text, the asker's job state, and
  the asker's output and messages since the question (bounded tail). Run
  when new output or messages arrive for the asker, rate-limited per item.
  Approvals are excluded.
  - `suggest`: `tower inbox` gains urgency and stale columns and sorts by
    urgency; the TUI inbox shows a marker; the web `needs_you` count is
    unchanged.
  - `act`: dismisses at or above the threshold, with `source: decision`.
  Verify: `FakeDecider` tests per mode; approvals never sent to the model;
  the rate limit holds under output bursts.

## Milestone 4 — Question routing (gated by S7.2)
([D§5.3](../concepts/design/05-core-objects.md), [D§7](../concepts/design/07-server-api.md), [D§9.3](../concepts/design/09-server-modules.md))

- **T4.1 Forward semantics.** A pending question addressed to `me` can be
  forwarded to an agent. The forwarded agent receives it as a prompt naming
  the question id. It answers with `tower_approve`/`respond` on that id,
  which delivers to the asker exactly as an operator answer does. The
  operator copy stays in the inbox, marked as forwarded, and the operator
  can still answer first; the first answer wins (CAS on `pending`). A
  forwarded question cannot be forwarded again. Verify: route and MCP tests
  — forward, agent answers, asker receives; operator answers first → the
  agent's answer gets 409; forwarding a second time is refused.
- **T4.2 Router.** On a new agent question, ask a `choice` question over
  the operator plus idle agents with briefs, excluding the asker. In `act`
  at or above the threshold, forward; otherwise the question stays with the
  operator. Never interrupt a working agent. Verify: `FakeDecider` tests;
  the asker is never a candidate; a busy agent is never a candidate.
- **T4.3 Surfaces.** The inbox, `task show` trail, and TUI show who a
  question was forwarded to and who answered. `docs/agent-loop.md`: a
  forwarded question arrives as a prompt; answer it with `tower_approve` on
  the named id. Verify: CLI/TUI smoke; agent-loop example matches real
  output.

## Milestone 5 — Docs, design amendments, exit verification

- **T5.1 Contract and UX docs.**
  - `docs/getting-started.md`: a decision-model section covering config for
    Ollama local and cloud, modes, `tower decisions`, and what changes in
    the inbox and job views.
  - The `tower-status` skill shows suggestions and recent model actions.
  - The `tower-queue-job` skill notes that the decision service may assign
    unowned jobs in `act` mode.
  Verify: examples match real output.
- **T5.2 Design amendments.** Apply the deltas listed in the
  [decision](../decisions/decision-models.md) to D§4, D§5.2.1, D§5.3, D§6,
  D§7, D§9, D§10, and D§13. Close
  [D§17.8](../concepts/design/17-open-questions.md), narrow D§17.9, and
  amend job-queue rules 2 and 6. Flip the decision to `accepted` once the
  operator approves this plan.
- **T5.3 Live exit verification.** Real herdr, two or more agents with
  briefs. Each step goes in the log below:
  1. No config: the queue and inbox behave as before.
  2. Ollama local, all points `shadow`: records appear beside operator
     actions; `tower decisions stats` shows agreement.
  3. `clef:cloud` in `act`: a queued job is assigned to the matching idle
     agent; a stale question is dismissed; a forwarded question is answered
     by another agent and delivered to the asker.
  4. Signed out or no credits: a warning, then operator fallback.
  5. A claude permission prompt stays an operator approval.
  Update the plans overview status and add the CHANGELOG entry.

## Backlog (seeds)

- Fine-tuning from decision records (Clef RL service, or local) once enough
  operator-labeled records exist.
- A model cascade (local Clef Flash first, escalate to cloud Clef below
  threshold): measure on records before building. The published cascade
  traded six accuracy points for small savings.
- Pane-text state detection (idle / blocked / done) as a decision point,
  beside the herdr detection.
- Supervisor duties from [D§17.9](../concepts/design/17-open-questions.md)
  that need generation, not choice (compaction, reset).

## Verification log

| Date | Check | Result |
|---|---|---|
