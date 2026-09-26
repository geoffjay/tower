# herdr research

herdr 0.8.2 is installed on this machine (`/usr/bin/herdr`, pacman package,
https://herdr.dev, Apache 2.0, GitHub `herdrdev/herdr`). It is a tmux-like
terminal workspace manager purpose-built for coding agents — the substrate the
agentos TUI should build on.

## Architecture

- **Client/server over Unix domain sockets**: server at
  `~/.config/herdr/herdr.sock`, client at `herdr-client.sock`. Handshake reports
  `version=20 encoding=SemanticFrame` — a semantic frame protocol, not raw
  terminal bytes.
- **Paned hierarchy**: workspace → tab → pane, mirroring tmux session/window/pane
  (config at `~/.config/herdr/config.toml` maps them explicitly). Layout state
  persists in `~/.config/herdr/session.json`; agents keep running when clients
  detach.
- **The TUI runs per client**; panes persist server-side. Also supports
  `--remote <ssh-target>` attach and `herdr machine` for multi-machine management
  (machines joined over SSH, their workspaces shown alongside local ones).

## JSON-RPC-style socket API

`herdr api schema` exposes schemas (`error_response, event, request,
subscription_event, success_response`) and typed methods (`AgentStartParams`,
`AgentPromptParams`, `AgentSendKeysParams`, `PaneAgentState`, `AgentViewFilter/Sort`).
`herdr api snapshot` returns the full workspace/tab/pane layout with geometry.
Responses look like `{"id":"cli:agent:list","result":{"agents":[],"type":"agent_list"}}`.

This is a first-class automation surface — herdr's own docs call it "the CLI and
the socket API are the same surface agents drive." **agentos should drive agents
through this API rather than owning terminals itself.**

## How it launches agents (22 kinds)

It does **not** spawn agent processes itself. It provisions a PTY pane running a
shell, types the agent CLI's canonical executable into it, then waits for
detection (`herdr agent start <NAME> --kind <KIND> --pane <ID> [-- <AGENT_ARGS>]`,
30s default timeout, pane must be at a shell prompt).

Kinds detected out of the box: `pi, claude, codex, gemini, cursor, devin, agy,
cline, omp, mastracode, opencode, copilot, kimi, kiro, droid, amp, grok, hermes,
kilo, qodercli, qwen, maki`. **Both target harnesses are covered: `claude` and
`pi` (ohmypi).** Per-kind detection manifests live at
`~/.local/state/herdr/agent-detection/remote/*.toml`, auto-updated from a remote
manifest service.

## Detection manifests (state machine)

Each manifest declares prioritized `[[rules]]` that classify the pane's lifecycle
state — `idle / working / blocked / done / unknown` — by scraping the emulated
terminal screen: regions like `osc_title`, `osc_progress`, `bottom_non_empty_lines(12)`,
`prompt_box_body`, `after_last_horizontal_rule`, matched with `contains`/`regex`/
`line_regex`/`all`/`any`/`not` combinators. Example: claude treats the `❯`
prompt-box as idle, `esc to interrupt` as working, "do you want to proceed?" as
blocked; codex uses "Action Required" OSC title as blocked.

This gives agentos a **free agent-state model** (working/blocked/idle) for the
monitoring UI, with zero instrumentation of the agents themselves.

## Driving agents

- `agent prompt` — sends text (honors bracketed-paste, encoded Enter), `--wait`
  for settled state
- `agent send-keys` — logical keys (`esc`, `ctrl+c`)
- `agent wait --until <state>` — wait for a lifecycle state transition
- `agent read` — read pane output; sources: `visible`, `recent`,
  `recent-unwrapped`, `detection` (plain-text bottom buffer), `--format ansi`
  preserves colors
- OSC 52/title/progress, SGR mouse, Kitty graphics, OSC 8 links all handled
- Lifecycle events are pushed to subscribers over the socket (subscription events)

## Implications for agentos

1. **Don't build a terminal multiplexer.** herdr already owns PTYs, persistence,
   reattach, state detection, and remote machines. agentos's TUI should be a
   herdr *client/overlay* (like openrig's `--provider herdr`), not a competitor.
2. **Consume the socket API**: snapshot for inventory, subscriptions for live
   state, `agent read` for streaming output, `agent prompt`/`send-keys` for input.
3. **Reuse the detection state machine** as the canonical agent status in the
   web UI (working/blocked/idle), and treat "blocked" as the human-attention
   signal.
4. **The Claude Code + pi support is already solved** — kinds `claude` and `pi`
   with maintained detection manifests.

## Reference material on this machine

- `/usr/bin/herdr`, `herdr api schema --json`, `herdr api snapshot`
- `~/.config/herdr/config.toml`, `~/.config/herdr/release-notes.json`
- `~/.local/state/herdr/agent-detection/remote/*.toml`
- `~/.local/bin/pi` (ohmypi wrapper: `exec mise x pi -- pi`; v0.87.1)
- `~/.local/bin/claude`, `codex`, `opencode`, `gemini` — all mise wrappers