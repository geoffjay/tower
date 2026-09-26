# Phase 1 — MVP core

Goal: one machine, one server. Spawn claude + pi agents through herdr, prompt
them, stream their output, see their states. Restart the server and lose
nothing.

Exit criteria (DESIGN.md §18): one machine: spawn claude+pi via herdr, prompt
both, stream output to terminal, states visible in `ps`; restart server,
agents rebind.

---

## Milestone 0 — Scaffold (D§2)

- **T0.1** Init cargo workspace, 6 crates per D§2 (`tower-core`,
  `tower-server`, `tower-client`, `tower-tui`, `tower-web`, plus
  `tower-driver` — extracted from server per M1 layout note below).
  Workspace-level lints: `unsafe` forbidden, `deny(warnings)` in CI config only.
  Verify: `cargo build` green on empty crates.
- **T0.2** CI (GitHub Actions or local justfile+script): fmt, clippy -D
  warnings, test, build. Verify: intentionally broken clippy line fails CI.
- **T0.3** `justfile` (or make) recipes: `dev`, `test`, `lint`, `run`, `e2e`
  (added phase-by-phase). Verify: `just lint` runs fmt+clippy.
- **T0.4** Repo docs: README install/run, CHANGELOG.md (keepachangelog
  format), AGENTS.md pointing agents at DESIGN.md + plans/.
  Verify: fresh clone builds with `just dev`.

## Milestone 1 — Spikes (D§17.1, D§17.2)

- **S1.A** **herdr CLI driver spike** (gates everything). For each verb,
  record exact command, JSON shape, failure modes, latency:
  - `herdr api snapshot --json` — schema of panes/agents; how pane ids and
    agent identity appear; how `--kind` + detection state map to what we need
    for reconcile (D§8.1, D§8.4)
  - `herdr agent start <name> --kind claude --pane <id>` in a scratch
    workspace — flag grammar, timeout behavior, error JSON
  - `herdr agent prompt` (+`--wait`) / `send-keys` / `read --source ...` /
    `wait --until` — settle semantics, what `--wait` actually waits on
  - Output: appended "S1.A findings" section in this file + decision on the
    driver's verb set; update DESIGN.md §8 if flag/shape corrections needed
- **S1.B** **pi harness spike**: spawn pi via herdr kind `pi`, prompt it,
  read output, confirm detection states fire (idle/working/blocked).
  Record whether pane prompt/read suffices or pi's programmatic/RPC mode is
  needed (D§17.2). Output: findings section + answer in DESIGN.md open
  questions.

## Milestone 2 — Core types + storage (D§5, D§6)

- **T2.1** `tower-core`: ULID wrapper, ms-epoch time helpers, error enum
  (codes per D§7: `not_found | conflict | timeout | driver | invalid`),
  serde-JSON DTOs for all §5 objects (Agent, Task, Message, Event, Machine),
  state enums with `#[non_exhaustive]` (states evolve; phase 2 adds pool
  fields). Verify: round-trip serde tests.
- **T2.2** DDL migrations (sqlx migrate): machines, agents, tasks, messages,
  artifacts, events per D§6 (full schema now — phase 2 needs the task-pool
  columns already present, avoiding a later migration churn on hot tables).
  XDG paths (D§4): config/db/artifact dir resolution, first-run token
  generation (0600). Verify: migration test against tmpdir; token perms
  asserted.
- **T2.3** Event log: `append_event(tx, type, subject, payload)` helper
  enforcing the envelope (seq, ts, type, subject_type, subject_id); write
  path goes through one tokio-mutex connection (single-writer, D§6).
  Verify: monotonic seq test, concurrent append under mutex serializes.

## Milestone 3 — Server shell (D§7, D§3)

- **T3.1** axum app skeleton: bind unix socket + TCP per config (D§4);
  `/healthz`, `/v1/schema` (route + event-type registry, D§7); bearer token
  middleware for TCP, socket-exempt path (D§13); JSON error envelope.
  Verify: curl both transports; 401 without token over TCP.
- **T3.2** Config loading (config.toml, defaults, D§4) + `tracing` JSON
  logs (D§15). Verify: debug level from env/config honored.
- **T3.3** `/v1/events` SSE endpoint (D§7 streaming): cursor from
  `Last-Event-ID` or `?cursor`, `?filter=`, `?subject=`, 15s heartbeat
  comments, disconnect-with-resume-hint on slow client (D§7 backpressure:
  bounded channel per subscriber). Verify: scripted event appends while
  `curl -N` streams; reconnect replays from cursor; filter works.
- **T3.4** Systemd user unit (`tower.service`) + `tower serve` daemon
  flags. Verify: `systemctl --user start tower` healthy on a dev box.

## Milestone 4 — HerdrDriver (D§8)

- **T4.1** `tower-driver` crate: `Harness` trait per D§8.1 +
  `ReadSource`/`ReadResult`/`HarnessEvent`/`AgentSpec` types. Trait stays
  object-safe; events via `BoxStream`. Verify: trait compiles with a mock
  harness used in tests.
- **T4.2** HerdrDriver CLI transport: implement each verb using S1.A's
  recorded commands. Process spawn wrapper with timeout, JSON parsing,
  error mapping to `driver` errors. No socket/SemanticFrame work in phase 1
  (D§17.1: CLI first). Verify: unit tests with recorded fixture JSON per verb
  (golden files), plus live smoke test against real herdr (`just e2e-smoke`,
  gated on `TOWER_E2E=1` so CI without herdr still passes).
- **T4.3** Driver event pump: poll `snapshot` + `agent read` (rate per
  S1.A findings), diff, emit `agent.state` + `agent.output` events into the
  log; output chunks also written under `artifacts/` (D§7 replay note).
  Verify: fake pane fixture drives state transitions through the pump.

## Milestone 5 — Inventory + sessions (D§9.1, D§9.2)

- **T5.1** Inventory module: boot sequence — ensure `local` machine row →
  driver snapshot → reconcile (match pane id, then name) → emit
  `agent.state` events for drift. Unknown detected agents listed as
  `adopted: true` candidates (D§8.4) without taking ownership. Verify:
  integration test with FakeHarness — spawn/adop/reconcile paths; restart
  rebind test (T8 reuses this).
- **T5.2** Sessions module + control routes (D§7): `POST /v1/agents`
  (spawn), `/prompt`, `/interrupt`, `/send-keys`, `/stop`, `GET /v1/agents`,
  `/v1/agents/{id}`, `/read`. Serialized per-agent driver calls (D§9.2).
  Verify: integration tests over FakeHarness covering happy paths + driver
  error mapping to `driver` code.

## Milestone 6 — CLI client (D§10)

- **T6.1** `tower-client`: transport (socket preferred, TCP+token
  fallback), `--json` on every verb, table rendering (human output).
  Verbs: `ps`, `spawn`, `prompt --wait`, `read`, `stream`, `stop`, `doctor`,
  `schema`. Verify: `--json` output shape-tested; table snapshot tests.
- **T6.2** `tower doctor` (D§4): herdr reachable (socket + CLI), db
  writable, port free, claude/pi found on PATH. Exit non-zero on failures;
  `--json` mode. Verify: failure injection tests (unset PATH entry,
  unwritable XDG dir).

## Milestone 7 — End-to-end smoke (D§16)

- **T7.1** E2E script (`just e2e`, requires local herdr + pi; claude when
  authed): start server on temp HOME → spawn pi agent → prompt "print hello
  and exit" → assert `working` then `done` state events over `/v1/events`
  → read output via CLI → `ps` shows both. Optional claude leg when
  `CLAUDE_AUTHED=1`. **S1.B amendment**: until pi provider auth is sorted
  (S1.B), the pi leg asserts prompt delivery + output read-back (error
  text is still output); state-event assertions use a FakeHarness-driven
  leg. Verify: green run; recorded transcript in this file.
- **T7.2** Restart resilience: stop server mid-agent-run → restart → agents
  rebind (pane id match), states refresh, event log unbroken. Verify: part
  of `just e2e`.

## Backlog (phase 2 seeds, do not do now)

- herdr socket (SemanticFrame v20) transport behind the same trait
- Messages/inbox, MCP endpoint, task pool (phase 2 scope)

## Verification log

(filled during execution)

| Date | Check | Result |
|---|---|---|

## Spike findings

### S1.A — herdr CLI driver (executed 2026-09-26)

All commands return single-line JSON: success `{"id":"cli:<verb>","result":{...,"type":"<snake_type>"}}`,
error `{"error":{"code":"<code>","message":"..."},"id":"..."}` — one error shape
to parse. herdr server must be running (it was; PID check via `herdr status`).
Protocol 20, herdr 0.8.2.

**Verb grammar validated (live):**

| tower op | herdr command | notes |
|---|---|---|
| snapshot/inventory | `herdr api snapshot` | full tree: workspaces/tabs/panes/agents/layouts; panes carry `pane_id`, `cwd`, `agent_status`, `terminal_title`, `revision`, `state_change_seq` |
| list panes | `herdr pane list` | same pane objects |
| split pane (for spawn) | `herdr pane split --pane <id> --direction right` → `result.pane.pane_id` | new pane at fresh interactive shell |
| start agent | `herdr agent start <NAME> --kind pi --pane <ID> --timeout 60000` → `result.agent` (has `interactive_ready: true`, `agent_status`, `name`, `pane_id`) | pane must be at shell prompt; timeout >3000, ≤300000 |
| prompt | `herdr agent prompt <NAME> <TEXT> --wait --until <STATES...> --timeout <MS>` | `--wait` requires observed state change within 5000ms else `agent_prompt_stalled`; blocked agent → `agent_blocked` rejection **before** input sent; without `--timeout` wait is indefinite |
| read | `herdr agent read <NAME> --source visible\|recent\|recent-unwrapped\|detection --lines N --format text\|ansi` | `--format ansi` preserves SGR |
| send-keys | `herdr agent send-keys <NAME> '<key>'` → `{"type":"ok"}` | logical keys |
| wait | `herdr agent wait <NAME> --until <STATES> --timeout <MS>` | default until: idle,done,blocked; timeout error code `timeout` |
| close pane | `herdr pane close <pane_id>` → `ok` | cleanup works |
| error shape | any bad target | `agent_not_found` observed; codes are stable-looking strings |

**Key facts for the driver:**
- `agent list` returns `agents: []` (empty when none started) — inventory
  reconcile source is `api snapshot` (which embeds `agents` array too)
- detection state lives on panes (`agent_status`), agents enrich with `name`;
  `herdr agent explain <name>` shows which manifest rule fired — useful for
  debugging blocked states later
- pane `revision`/`state_change_seq` give change-detection keys for polling
- `agent prompt --wait` semantics: it does NOT track turns; if agent already
  `working`, completion of that active turn may match. Driver treats `--wait`
  as "settled-state wait", not completion tracking
- **pi has no provider auth on this machine** (S1.B blocker, below) — agent
  started and detected `idle` fine, but any prompt errors inside pi with
  "No API key found" (pi 0.87.1, no auth.json, no ANTHROPIC_API_KEY etc.).
  Also noted: `agent_prompt_stalled` fired even for valid pi input because
  the error screen didn't change `state_change_seq` — detection sees the
  same idle state. Driver must not treat `agent_prompt_stalled` as fatal;
  re-read output and surface the text instead.
- `herdr pane run <PANE_ID> <CMD>...` exists (run command in pane) — could
  replace split+start for direct CLI launching, untested here
- Workspaces exist (`herdr workspace list`); tower should create/use a
  dedicated workspace for its agents rather than polluting user's —
  **new open question for DESIGN.md §17** (workspace management verb)

**Decision**: driver verbs = snapshot, pane split/close, agent
start/prompt/read/send-keys/wait/list. Output parsing: serde over the
single-line JSON; error mapping via `error.code`. No socket work needed for
phase 1 — CLI is sufficient and stable-looking. Latency: all verbs
sub-second except start (detection wait ~2–10s expected) and prompt --wait.

### S1.B — pi harness (executed 2026-09-26)

- pi 0.87.1 (mise-managed) starts in herdr pane and detects as `idle`
  via manifest `remote:/.../pi.toml 2026.09.14.1` — **pane prompt/read
  suffices for driving; no RPC runner needed for phase 1**
- Detection confirmed live: `agent explain spike-pi` → `state: idle`,
  `fallback_reason: default_known_agent_idle_fallback` (prompt box idle
  rule)
- **Blocker found: pi has no provider credentials on this machine**
  (`Error: No API key found for the selected model`; no auth.json, no
  provider env vars). Any prompt to pi fails inside the harness with an
  on-screen error; herdr detection stays `idle` (error screen not in
  manifest rules) and `agent prompt --wait` returns `agent_prompt_stalled`
- Resolution options (operator decision needed): (a) run `pi /login`
  interactively once and store auth.json, (b) export a provider API key
  in tower's agent env (pi reads ANTHROPIC_API_KEY etc.), (c) point pi at
  a local/gateway provider (radius/pi.dev catalog in providers.md)
- Until resolved, pi prompts in e2e smoke (T7.1) will show the error
  screen; the smoke test can still verify prompt delivery + read-back
  (text appears on screen) even without a working model

**DESIGN.md §17.2 answer**: pane prompt/read is enough. pi's programmatic
mode unnecessary for v1.