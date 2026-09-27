---
name: tower-status
description: Summarize the tower fleet and say what needs the user. Use when the user asks what tower or the agents are doing, what is blocked, what is in the queue, or what needs attention. Reads agents, the inbox, open jobs, and schedules, and reports problems first. Read-only; it never answers inbox items or changes jobs.
argument-hint: "[stack name]"
---

# Tower status

Read the whole fleet state and report it. Put the items that need the user
at the top. Do not change anything.

## Before you start

Work from the tower repo root. Use `tower` if `command -v tower` finds it.
Otherwise use `./target/debug/tower`. Run `tower doctor`. If the
`tower server` check fails, report that tower is down and stop.
If a command fails with `Operation not permitted`, a sandbox blocks the
local connection. Tower is not down. Ask the user to allow the command to
run outside the sandbox.

## 1. Read the state

```sh
tower --json ps              # agents: name, kind, state, workdir, id
tower --json inbox           # pending questions and approvals for the user
tower --json task list       # jobs, all states
tower --json schedule list   # schedules: next_run_at, enabled, target_agent_id
```

Jobs and schedules refer to agents by id. Map each `owner_id` and
`target_agent_id` to a name with the `ps` output. Times are milliseconds
since the Unix epoch. Show them as local times.

If the user gave a stack name, keep only its items: agents named
`<stack>-*`, jobs and schedules with the tag `stack:<stack>`, and
schedules titled `<stack>: …`.

## 2. Find what needs the user

Report these first, most urgent first:

| Signal | How to see it | What to tell the user |
|---|---|---|
| Pending inbox item | `inbox` messages | The agent, the question, its age. It expires after its deadline (default 5 minutes), and an expired approval is denied. Answer: `tower approve <id>`, `--deny`, or `--answer '<text>'` |
| Blocked agent | `ps` state `blocked` | Usually has an inbox item. If not, show `tower read <name> \| tail -20` |
| Dead agent | `ps` state `dead` | Its jobs requeue when their leases expire. Offer `/tower-agent-remove` |
| Job not started | state `assigned` for longer than its `lease_s` | The owner did not declare start. It requeues soon and uses an attempt |
| Job near its attempt limit | `attempt_count` = `max_attempts` - 1 | The next lease expiry fails the job |
| Failed job | state `failed` | Show the `result` from `tower task show <id>` |
| Queued job with no owner | state `queued`, no `target_agent_id` | Nobody gets it until the operator assigns it |
| Paused schedule | `enabled` false | It does not fire. A removed target pauses it |

## 3. Report

After the attention list, give a short summary:

- agents: name, kind, state, and the job each one owns
- queue: counts of queued, reserved, assigned, working, and input-required jobs
- schedules: title, target, next run
- recent results: jobs that finished since the last report the user saw,
  if the user asked for them

Keep it short. Use one table for the agents. Use at most one line for
each other item.

## Rules

- Do not answer inbox items. Show them and let the user decide. An
  approval gives an agent permission to act.
- Do not cancel, assign, or release jobs from this skill. Suggest the
  command instead.
