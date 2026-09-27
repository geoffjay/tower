---
name: tower-agent-remove
description: Stop or remove a tower agent safely. Use when the user wants to retire, stop, or delete a tower-managed agent. Shows what the removal affects first — the job the agent owns, jobs reserved for it, and schedules that target it — gets confirmation, removes it, and offers to reassign the work that goes back to the queue.
argument-hint: "<agent name>"
---

# Remove a tower agent

Removal has effects on work. Show them to the user before you act.

## Before you start

1. Work from the tower repo root. Use `tower` if `command -v tower` finds
   it. Otherwise use `./target/debug/tower`. The examples below say
   `tower`.
2. Run `tower doctor`. If the `tower server` check fails, stop and show
   the user the message.
   If a tower command fails with `Operation not permitted`, a sandbox
   blocks the local connection to the server or to herdr. It does not
   mean that tower is down. Ask the user to allow the command to run
   outside the sandbox.

## 1. Identify the agent

Take the name from the text after the command. If there is no name, show
`tower ps` and ask which agent. If the user names a stack, not one agent,
use `/tower-deploy` for teardown instead.

Get the agent id. Jobs and schedules refer to agents by id, not by name:

```sh
tower --json ps | python3 -c "import sys,json; print([a['id'] for a in json.load(sys.stdin)['agents'] if a['name']=='<name>'][0])"
```

## 2. Find what the removal affects

```sh
tower task list --mine <name>        # the open job it owns (at most one)
tower --json task list               # reserved jobs: target_agent_id == <id>, state queued
tower --json schedule list           # schedules: target_agent_id == <id>
```

Tell the user each effect that applies:

| What | Effect of `stop --remove` |
|---|---|
| The job it owns | Goes back to the queue with no owner. The attempt count does not change. Work in progress in the pane is lost. |
| Jobs reserved for it | Go to the general queue with no target. Nobody gets them until the operator assigns them. |
| Schedules that target it | Pause, and their target is cleared. They fire again only after `tower schedule resume <id>`, and then into the general queue. |
| Finished jobs | Stay in the history. Their owner field is cleared. The trail in `tower task show` still records the work. |

If the agent owns a job in state `working`, say so first. Suggest that the
user wait for it, or let the agent finish with `tower task show <id>` to
watch it.

## 3. Choose stop or remove

| Command | Result |
|---|---|
| `tower stop <name>` | Closes the pane. The agent row stays with state `dead`. Its jobs stay as they are until their leases expire. |
| `tower stop <name> --remove` | Closes the pane, releases its jobs as in the table above, and deletes the agent row. |

Default to `--remove` when the user says "remove" or "delete". Ask for
confirmation before you run either command.

## 4. Remove and follow up

1. Run the command.
2. Run `tower ps` and confirm that the agent is gone (or `dead`).
3. For each job that went back to the queue, ask the user for a new owner.
   Assign with `tower task assign <id> <agent>`, or `--when-available` if
   that agent is busy.
4. For each paused schedule, ask the user for a new target. There is no
   command to change a schedule target. Remove it with
   `tower schedule rm <id>` and create it again with `--assign <agent>`,
   or resume it into the general queue with `tower schedule resume <id>`.

## Rules

- Do not remove an agent without confirmation from the user.
- Do not use `herdr` commands to close an agent's pane. Tower must see the
  removal to release the jobs.
