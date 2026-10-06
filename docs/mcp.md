# Wiring MCP clients to tower

How to point Claude Code, oh-my-pi (omp), and pi at tower's MCP endpoint.
The work-loop contract the tools serve is [`docs/agent-loop.md`](./agent-loop.md);
the server surface is design [§7](knowledgebase/concepts/design/07-server-api.md).

Every client needs the same three things:

| Piece | Value |
|---|---|
| URL | `http://127.0.0.1:8266/mcp` — the server's `[server] bind_tcp` (default `127.0.0.1:8266`) plus `/mcp`, streamable HTTP |
| Auth | `Authorization: Bearer <token>` — the token file `tower serve` prints on startup: `$TOWER_HOME/token` if set, else the XDG data dir (`~/Library/Application Support/tower/token` on macOS, `~/.local/share/tower/token` on Linux), 0600 |
| Identity | `X-Tower-Agent: <agent name>` — with it, the client **is** that agent (sees its own jobs/inbox, cannot `tower_task_assign`, `tower_stop`, `tower_task_cancel`, or mutate schedules). Without it, the client acts as the **operator** (full access including dispatch) |

The agent name must exist in tower (`tower ps`); an unknown name fails with
`not_found`. Per-call `as` arguments override the header on owner tools.

## Claude Code

```console
$ claude mcp add --transport http tower http://127.0.0.1:8266/mcp \
    --header "Authorization: Bearer $(cat "$HOME/Library/Application Support/tower/token")" \
    --header "X-Tower-Agent: backend"
```

- `-s local|user|project` sets scope (default `local` — this project, your
  machine only). Use `user` for every Claude session, `project` to commit a
  `.mcp.json` in the repo.
- Claude Code's sandbox blocks `127.0.0.1`: the first tower call fails with
  `Operation not permitted` until you approve running it outside the sandbox.
- Config lands in `~/.claude.json` (`mcpServers.<name>` = `{ "type": "http",
  "url": …, "headers": {…} }`) — hand-editing that file works too.

## oh-my-pi (omp)

User scope `~/.omp/agent/mcp.json`, project scope `.omp/mcp.json`:

```json
{
  "mcpServers": {
    "tower": {
      "type": "http",
      "url": "http://127.0.0.1:8266/mcp",
      "headers": {
        "Authorization": "Bearer !cat \"$HOME/Library/Application Support/tower/token\"",
        "X-Tower-Agent": "backend"
      }
    }
  }
}
```

- **`"type": "http"` is required** — omit it and omp assumes stdio and errors
  ("requires command field").
- Header values resolve at connect time: a leading `!` runs the rest as a
  shell command (10 s timeout, trimmed stdout); a bare environment-variable
  name copies that variable; `${VAR}` expands at discovery. So
  `"Authorization": "Bearer ${TOWER_TOKEN}"` works if `TOWER_TOKEN` is
  exported in the launching shell.
- `/mcp add tower --scope user --url http://127.0.0.1:8266/mcp --transport http`
  is the wizard form; it has no flag for extra headers, so add
  `X-Tower-Agent` by editing the JSON.
- Verify with `/mcp list` (shows which file a server came from) and
  `/mcp test tower`; `/mcp reload` after edits.
- omp also imports Claude Code configs (`~/.claude.json`, `.claude/mcp.json`),
  so a server added for Claude Code may already appear in omp under the same
  name — check `/mcp list` before duplicating it.

## pi

Global `~/.pi/agent/mcp.json`, project `.pi/mcp.json` (trusted projects only —
`mcp.json` is a trust-requiring project resource; approve the project with
`--approve`/the trust prompt). Same `mcpServers` shape; `url` implies HTTP:

```json
{
  "mcpServers": {
    "tower": {
      "url": "http://127.0.0.1:8266/mcp",
      "headers": {
        "Authorization": "Bearer !cat \"$HOME/Library/Application Support/tower/token\"",
        "X-Tower-Agent": "backend"
      }
    }
  }
}
```

- CLI form: `pi mcp add tower --url http://127.0.0.1:8266/mcp
  --header 'Authorization=Bearer …' --header 'X-Tower-Agent=backend'`
  (`-l` writes the project file instead).
- `--bearer-token-env-var TOWER_TOKEN` writes
  `"Authorization": "Bearer ${TOWER_TOKEN}"` for you — the cleanest secret
  form when the token is exported in the pane's shell.
- Header value resolution matches omp: `!command`, `${VAR}`/`$VAR`, else
  literal. An `auth` block is allowed only in the global file.
- Verify with `pi mcp list` (exits 1 on failure — agents can self-check their
  config with it).
- Legacy `sse` transport is rejected — use the streamable HTTP url.

## Tower-spawned agents

`tower spawn` exports `TOWER_AGENT=<name>` into the pane (and `TOWER_HOME`
when non-default) so the CLI knows who is calling. MCP config cannot read
that env var into a *header* portably, so per-agent MCP config hardcodes the
agent's name:

- **claude agents**: put the server (with that agent's `X-Tower-Agent`) in
  the workdir's `.mcp.json` or the project `.claude/mcp.json` before/at spawn.
- **omp/pi agents**: `.omp/mcp.json` / `.pi/mcp.json` in the agent's workdir,
  same headers.

Agents without MCP configured still run the full work loop through the
`tower` CLI (`tower task list --mine` etc. — it picks up `$TOWER_AGENT`);
MCP is a convenience surface, not a requirement. Either way, point the
agent's prompt at [`docs/agent-loop.md`](./agent-loop.md).

## Rotating the token

The token file is generated on first run; deleting it regenerates on next
`tower serve`. Baked-in header values (`!cat …` forms) pick up the new token
automatically; literal `Bearer <token>` strings in configs go stale — re-add
the server.