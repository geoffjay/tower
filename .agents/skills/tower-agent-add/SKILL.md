---
name: tower-agent-add
description: Add an agent to the tower fleet. Use when the user wants a new tower-managed agent (a coding harness such as omp, claude, or pi running in a herdr pane) with a role. Collects the name, harness kind, working directory, role, and whether to save a named definition, spawns the agent with a standing brief, and confirms that it started and can reach its model.
argument-hint: "[name] [kind] [role]"
---

# Add a tower agent

Spawn one agent with a clear role, then prove that it works. A spawned
agent that cannot reach its model looks idle but does nothing, so the
check in step 5 is required.

## Before you start

1. Work from the tower repo root. Use `tower` if `command -v tower` finds
   it. Otherwise use `./target/debug/tower`, and run `cargo build` if that
   file does not exist. The examples below say `tower`.
2. Run `tower doctor`. If the `tower server` or `herdr` check fails, stop
   and show the user the message.
   If a tower command fails with `Operation not permitted`, a sandbox
   blocks the local connection to the server or to herdr. It does not
   mean that tower is down. Ask the user to allow the command to run
   outside the sandbox.
3. Run `tower ps` to see the names that are already in use.

## 1. Collect the facts

Read what the user gave after the command. Ask for the missing facts in
one message.

| Fact | Rule | Default |
|---|---|---|
| Name | Lowercase letters, digits, and `-`. Must not be in `tower ps`. In a stack, use `<stack>-<role>` | Derive from the role |
| Kind | A herdr harness kind: `omp`, `claude`, `pi`, `codex`, … | `omp` |
| Working directory | Absolute path of the project the agent works in | The current project root |
| Worktree | `--worktree` gives the agent its own git branch and directory. Use it when two agents write in the same repo | Off |
| Role | What the agent is for, and what is out of its scope | — (required) |
| Definition | Whether to save the agent as a named definition. A definition makes the agent easy to create again after a restart. Ask the user. Recommend yes. | Ask |

`tower doctor` checks only `pi` and `claude` on `PATH`. For another kind,
run `command -v <kind>`. If the binary is missing, tell the user and ask
for another kind.

## 2. Write the brief

The brief is the first prompt the agent receives. Tower sends it when the
agent is ready. It must stand alone. Use this template:

```text
You are <name>, a tower-managed agent. Your role: <role>.
<If in a stack:> You are part of the "<stack>" stack. Its purpose: <purpose>.
Out of scope: <limits>.

You get work only as tower jobs. Never take work that was not assigned to you.
When a job arrives, follow its delivery message: declare start, heartbeat, and report the result.
The full contract: run `<absolute path of the tower binary> contract`.
The tower CLI is <absolute path of the tower binary>. Your identity is already set in $TOWER_AGENT.

Reply now with exactly one line: ready <name>
```

Always write the absolute path of the binary. `tower` is often not on the
`PATH` inside agent panes. Show the brief to the user before you spawn.

## 3. Save the definition (only if the user agreed)

Write the agent as a named definition. Then the user can create it again
with `tower spawn --name <name>` after a restart. The definition holds
the brief, so the agent does not lose its role.

```sh
mkdir -p ~/.config/tower/agents/<name>
```

Write the brief from step 2 to `~/.config/tower/agents/<name>/PROMPT.md`.
Write the facts from step 1 to
`~/.config/tower/agents/<name>/config.toml`:

```toml
kind = "<kind>"
workdir = "<working directory>"
worktree = true   # write this line only when worktree is on
```

If a file already exists, show it to the user and ask before you
overwrite it.

## 4. Spawn

```sh
# With a definition saved in step 3:
tower spawn --name <name>
# Without a definition:
tower spawn <name> --kind <kind> --workdir <dir> [--worktree] --prompt '<brief>'
```
Quote the brief with single quotes. If it contains a single quote, write
`'\''` in its place. Spawn creates a tab labelled with the agent name in
the `tower-agents` herdr workspace. It does not change the user's own
workspaces.

## 5. Confirm that it works

1. Wait about 15 seconds. Then run `tower ps`. The agent must be `idle` or
   `working`.
2. Run `tower read <name> | tail -20`. Look for the line `ready <name>`.
3. If the state is `blocked`, run `tower inbox`. A new harness often asks a
   question at startup (for example, a folder-trust dialog). Show the item
   to the user. Answer it with `tower approve <id>` only if the user agrees.
4. If the pane shows an authentication or provider error (for example
   `401`), the harness cannot reach its model. Tell the user the exact
   error. Offer to remove the agent (`tower stop <name> --remove`) and try
   another kind.

## 6. Report

Tell the user the name, kind, working directory, and the state from
`tower ps`. If you saved a definition, tell the user how to create the
agent again: `tower spawn --name <name>`. Tell the user how to give it
work: `/tower-queue-job`, or `tower task create '<title>' --assign <name>`.

## Rules

- Do not start harness processes yourself. Tower spawns agents through
  herdr only.
- Do not put secrets in the brief. Prompts are stored in the event log.
  The same rule applies to `PROMPT.md`.
- To take over an agent that already runs in herdr, use
  `tower spawn <name> --adopt` instead of a new spawn.
