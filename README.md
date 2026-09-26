# agentos

Project research for a multi-agent orchestration system: a single server managing
coding agents (Claude Code, pi/ohmypi), with a thin client, a read-only monitoring
web UI, and a TUI layered on top of herdr.

## Goals

- Client/server model: **one** server, one client binary for talking to every agent
- Spawn and supervise agents
- Communicate with agents (human-to-agent and agent-to-agent)
- Agents stream data out, possibly via SSE
- Harnesses: **Claude Code** and **ohmypi** (the `pi` CLI, v0.87.1, installed via mise)
- Web UI: agent cloud — visual monitoring only (color/size/halo encodings),
  not for configuration or control
- TUI: adds on top of **herdr**, similar to how **openrig** does it

## Background

The prior project, agentd, fulfilled its purpose but split functionality across
nine services, which made it hard to manage. The ideal setup is a single server
with a single client. openrig has a method for launching Claude Code and Codex
agents that is worth studying, plus a herdr terminal-provider integration.

## Documents

| File | Contents |
|---|---|
| [research/agentd.md](research/agentd.md) | What agentd was, why it hurt, and what to keep |
| [research/herdr.md](research/herdr.md) | herdr's architecture — the substrate the TUI builds on |
| [research/openrig.md](research/openrig.md) | openrig's architecture, agent launching, and herdr-as-provider |
| [research/ipc.md](research/ipc.md) | IPC options for inter-agent and human-agent communication |
| [research/cross-server.md](research/cross-server.md) | Communication across servers / machines |
| [research/a2a.md](research/a2a.md) | A2A protocol findings and fit assessment |
| [RECOMMENDATIONS.md](RECOMMENDATIONS.md) | Synthesis: proposed architecture for agentos |
| [DESIGN.md](DESIGN.md) | Full design: stack, data model, API, modules, phases |
| [docs/getting-started.md](docs/getting-started.md) | Target-state CLI walkthrough — the UX contract |
| [plans/](plans/) | Execution plans per phase with milestones and exit criteria |

## Research questions

1. Can agentd be simplified and improved for this? — yes; see [research/agentd.md](research/agentd.md)
2. How does openrig support herdr as a provider? — see [research/openrig.md](research/openrig.md)
3. What is the best IPC for inter-agent and human-agent comms? — see [research/ipc.md](research/ipc.md)
4. What is the best communication method across servers? — see [research/cross-server.md](research/cross-server.md)
5. A2A protocol — see [research/a2a.md](research/a2a.md)