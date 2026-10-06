# tower-switcher

A herdr plugin (Concept A, [multi-domain decision
§8](../../docs/knowledgebase/decisions/multi-domain-operation.md)): the
operator's cross-session surface for tower.

## Two surfaces

- **Switch** (`herdr plugin action invoke dev.tower.switcher.switch`, or
  bind a key — see below): a fuzzy picker over herdr sessions →
  workspaces/agents; selecting focuses it in the invoking herdr client.
- **Board** (`herdr plugin pane open --plugin dev.tower.switcher
  --entrypoint board`): a popup overview of every running session — its
  workspaces, its named agents with lifecycle states — plus the tower
  fleet (`tower ps`).

## Install

```console
$ herdr plugin link /path/to/tower/plugins/tower-switcher
$ herdr plugin action list --plugin dev.tower.switcher
```

Bind a key (in `~/.config/herdr/config.toml`):

```toml
[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "dev.tower.switcher.switch"
description = "tower switcher"
```

## Requirements

- herdr ≥ 0.9.3 (both CLI **and running server** — a 0.8.x server answers
  `protocol_mismatch`; `herdr session list` works but per-session queries
  and plugin registration need the restarted server. The board and
  switch scripts fall back to the raw socket protocol for *reads*, which
  works against either generation; focus commands and plugin registration
  do not.)
- `jq`, `fzf`, `python3` (stdlib only) on `PATH`

## How it targets sessions

Reads go over each session's Unix socket directly (`tower-herdr-query`,
JSON protocol v20+) — session-scoped, no `HERDR_*` env dependence. Focus
commands run `herdr --session <name> …` through `HERDR_BIN_PATH`, so they
act in the *invoking* client's session space. Tower state comes from the
`tower` CLI (token/home resolution unchanged).

## Limitations (by design)

- Navigator only: focus/attach. Driving tower verbs (assign, cancel)
  stays in the TUI/CLI.
- One tower server assumed (`tower ps` addresses the default home). The
  per-domain multi-server layout ([decision §7.2](../../docs/knowledgebase/decisions/multi-domain-operation.md))
  would pass `TOWER_HOME` per stack — future work.