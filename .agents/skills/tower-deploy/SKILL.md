---
name: tower-deploy
description: Design and deploy a tower stack — a named group of agents with roles, recurring jobs, workflows, and starting jobs — from the user's goals. Use when the user wants to set up agents for a project or purpose, change an existing stack, or tear one down. Interviews the user, proposes the stack for approval, deploys it with the tower CLI, and verifies each agent, schedule, and workflow.
argument-hint: "[stack purpose | update <stack> | teardown <stack>]"
---

# Deploy a tower stack

A stack is a named group of tower objects that serve one purpose. Tower has
no stack object. The stack exists only through these names, so follow them
exactly:

| Object | Convention | Example |
|---|---|---|
| Stack name | Short, lowercase letters, digits, `-` | `docs` |
| Agent | `<stack>-<role>` | `docs-writer` |
| Schedule | Title `<stack>: <title>`, tag `stack:<stack>` | `docs: nightly link check` |
| Workflow | Name `<stack>-<name>` | `docs-daily` |
| Job | Tag `stack:<stack>` | — |

With these names, the current state of any stack is visible from
`tower ps`, `tower schedule list`, `tower workflow list`, and
`tower task list --tag stack:<stack>`. A workflow step's job has no tags,
so `task list --tag` does not show it: use `tower run list --workflow
<name>` and `tower run show <id>`.

Choose the mode from the text after the command:

- a purpose, or no text: **deploy** a new stack (steps 1–5)
- `update <stack>`: read the stack (step 0), then run steps 1–5 as changes
- `teardown <stack>`: go to "Teardown"

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
3. Tell the user that schedules and workflow schedule triggers fire only
   while `tower serve` runs. For a stack with recurring work, suggest
   `tower service install` so the server keeps running.

## 0. Read an existing stack (update and teardown)

```sh
tower --json ps              # agents named <stack>-*
tower --json schedule list   # titles "<stack>: …" or tag stack:<stack>
tower --json workflow list   # names <stack>-*
tower --json run list --workflow <stack>-<name>
tower --json task list --tag stack:<stack>
```

Show the user what exists: each agent (kind, state, workdir), each
schedule (cadence, target, next run), each workflow (version, triggers,
open (waiting or running) runs), and the open jobs.

## 1. Interview

Ask about goals first, then details. Ask in groups of two to four
questions. Do not ask for a fact the user already gave.

1. **Purpose.** What must the stack achieve? What does a good week look
   like? What is out of scope?
2. **Project.** Which directory or repo does it work in? Must agents write
   to it, or only read and report?
3. **Work.** What recurring work is there (what, how often, what time,
   which timezone)? What one-off work must start now? Does one piece of
   work feed another (collect, then summarize)? Must work be reviewed and
   fixed before it reaches the user?
4. **People.** Who answers the agents' questions and approvals? How
   quickly? (An approval that nobody answers is denied after its deadline.)
5. **Limits.** Harness preference, and a limit on the number of agents
   (each agent uses model credit).

## 2. Propose

Design the smallest stack that meets the purpose:

- Give each agent one clear role. Add an agent only for work that can run
  in parallel, or work that needs a different kind or directory. An agent
  works one job at a time, so one agent serializes its jobs.
- Give `--worktree` to each agent that writes in a git repo that another
  agent also writes in.
- Give each schedule an owner, a cadence, and a timezone. Choose a lease
  of 600 seconds or more for real work. Put the done criteria and the
  result summary format in each schedule description.
- Keep starting jobs few, and give each one done criteria.
- Choose a schedule or a workflow for each piece of recurring work:
  - A **schedule** fits work that stands alone: one agent, one job, and
    nothing waits for its result.
  - A **workflow** fits work in steps: a step that needs another step's
    result (collect, then curate), steps that run in parallel and then
    join, or a review loop (author → reviewer → fix → reviewer). Do not
    chain steps with schedule times that guess how long each step takes.
    A workflow starts each step when the step before it ends, and gives
    it that step's summary.
  - Never give the same work to both. When a workflow takes over
    scheduled work, remove those schedules in the same deploy.

Show the proposal in this format, then ask for approval:

```text
Stack: <stack> — <purpose in one sentence>
Project: <dir>

Agents
| name | kind | workdir | worktree | role |

Schedules
| title | cadence (tz) | owner | lease | description (short) |

Workflows
| name | trigger | steps (agent) and edges | loop limits |

Starting jobs
| title | owner | timing | description (short) |

Notes: <cost, what needs the user, what the stack does not do>
```

For an update, show only the changes: add, change, remove.
`tower schedule edit <id>` changes a schedule's cadence, timezone, title,
description, and pre-run or post-run prompts. A new target is a remove and
a create. A changed workflow is a new version of the same name
(`tower workflow apply`). Deploy nothing until the user approves the
proposal.

## 3. Deploy

Deploy in this order. Stop at the first failure, report it, and ask the
user how to continue.

1. **Agents.** For each new agent, follow `.agents/skills/tower-agent-add/SKILL.md`
   steps 2–4. The approved proposal is the confirmation, so do not ask
   again for each agent. Put the stack name and purpose in each brief.
   Confirm that every agent replied `ready <name>` before you continue.
2. **Schedules.** For each schedule:

   ```sh
   tower schedule create '<stack>: <title>' --cron '<expr>' --tz <zone> \
     --assign <stack>-<role> --tag stack:<stack> --lease-s 600 --description '<text>'
   ```

   Use `--daily HH:MM` in place of `--cron` for a daily run.
3. **Workflows.** Write each workflow as a TOML file. The format is in
   `docs/knowledgebase/decisions/workflows.md` §9. Keep the file outside
   the agents' repos, for example next to the stack's other config. Then:

   ```sh
   tower workflow apply <file>    # validates and stores a new version
   tower workflow show <name>
   ```

   Follow these rules when you write the file. `apply` does not check
   all of them:
   - A node with an incoming edge does not start when the run starts. To
     loop back to an author, add a separate fix node for that agent
     (`author → review → fix → review`) instead of an edge back into the
     first node.
   - Every loop needs a node with `max_visits`. When a step goes past
     the limit, it fails.
   - A failed step with no `on = "failed"` edge fails the whole run and
     cancels its open steps. Add `failed` edges where the rest of the run
     must continue.
   - A node that branches declares `outcomes`. Its prompt must name the
     outcomes and tell the agent to report one, with a `summary` of at
     most 2,000 characters.
   - A node with `pre_run = "/clear"` starts without the agent's brief.
     Start each prompt with "read your brief again" and the path of its
     `PROMPT.md`. The same applies to schedules with `--pre-run /clear`.

   A run claims every agent its definition names when it starts, and
   keeps them until it ends. While a run claims an agent, the agent gets
   only that run's jobs. So two workflows that share an agent run one
   after another: the later run waits (state `waiting`) until the
   earlier one ends. A standalone schedule or job for a claimed agent also
   waits for the run. Give a workflow its own agents when it must not
   wait, and do not schedule a standalone job for an agent at a time when
   a run of a workflow that names it usually runs.

   If the user agrees, start one test run with `tower workflow run
   <name>`, and show its steps with `tower run show <id>`. A `waiting`
   run shows the agents it waits for and the run that holds each one.
4. **Starting jobs.** For each job, follow `.agents/skills/tower-queue-job/SKILL.md`
   step 3, and add `--tag stack:<stack>`. Every assignment waits for a
   free agent, so two starting jobs for the same agent run one after the
   other.
5. **Removals (update only).** Remove workflows and schedules first, with
   `tower workflow rm <name>` and `tower schedule rm <id>`. Then remove
   agents as in `.agents/skills/tower-agent-remove/SKILL.md`. If you
   remove the agent first, its schedules pause and its reserved jobs go to
   the general queue.

## 4. Verify

```sh
tower ps
tower schedule list
tower workflow list
tower task list --tag stack:<stack>
tower inbox
```

Every agent must be `idle` or `working`. Every schedule must show the
correct target and next run. Every workflow must be `enabled`, and its
schedule trigger must show in `tower schedule list` as `workflow <name>`
with the correct next run. Every starting job must be `assigned`,
`working`, or reserved (`→<agent>`). If an agent is `blocked`, show the
inbox item to the user. Do not answer it without the user.

## 5. Report

Give the user:

- the stack summary table with ids
- the next run of each schedule and each workflow
- how to watch it: `/tower-status <stack>`, `tower task show <id>`,
  `tower run list --workflow <name>`, `tower run show <id>`,
  `tower read <agent>`
- how to change it: `/tower-deploy update <stack>`, and how to remove it:
  `/tower-deploy teardown <stack>`

## Teardown

1. Read the stack (step 0) and show the user everything that will go.
   Say which jobs are `working` now. That work stops.
2. Ask for confirmation.
3. Remove every stack workflow and schedule first, so no new job or run
   fires during the teardown. Cancel each waiting or running run with
   `tower run cancel <id>`, then `tower workflow rm <name>` (it refuses
   while a run is waiting or running, and it removes the workflow's
   schedules).
   Then `tower schedule rm <id>` for each schedule.
4. Cancel open stack jobs that the user does not want to keep:
   `tower task cancel <id>`.
5. Remove each agent: `tower stop <stack>-<role> --remove`.
6. Run `tower ps`, `tower schedule list`, and `tower workflow list`, and
   confirm that nothing with the stack name is left.

## Rules

- Do not deploy or tear down without the user's approval of the plan.
- Do not use an agent name, schedule title, or workflow name outside the
  convention. The convention is the only record of the stack.
- Do not put secrets in briefs, descriptions, or titles. Tower stores
  them in the event log.
- Do not answer inbox items for the user.
