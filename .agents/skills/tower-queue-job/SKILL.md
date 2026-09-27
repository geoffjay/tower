---
name: tower-queue-job
description: Put a job in the tower job queue from a plain-language description. Use when the user wants an agent to do a piece of work through tower — now, when an agent is free, at a time, or on a recurring cadence. Takes the job description as the argument; interviews the user for anything missing (done criteria, agent, timing), shows the job for approval, then creates it with the tower CLI.
argument-hint: "[job description]"
---

# Queue a tower job

Turn the user's request into one well-formed tower job. Then create it,
assign or reserve it, and report how to follow it.

Tower dispatch is assignment-only. Agents never take work themselves. You
act as the operator: you create the job and you choose the owner.

## Before you start

1. Work from the tower repo root. Use `tower` if `command -v tower` finds
   it. Otherwise use `./target/debug/tower`, and run `cargo build` if that
   file does not exist. The examples below say `tower`.
2. Run `tower doctor`. If the `tower server` check fails, stop. Show the
   user the message. It names the cause (server not started, wrong
   `TOWER_HOME`, bad token).
   If a tower command fails with `Operation not permitted`, a sandbox
   blocks the local connection to the server or to herdr. It does not
   mean that tower is down. Ask the user to allow the command to run
   outside the sandbox.
3. Run `tower ps` to see the agents and their states.

## 1. Collect the job

Take the job description from the text after the command. If there is no
text, ask the user what the job is.

You need these facts. Ask only for facts that are missing or unclear. Ask
them together in one message, not one at a time.

| Fact | Why | Default |
|---|---|---|
| What to do | The agent sees only the title and description | — (required) |
| Done criteria | The agent must know when to report `completed` | Ask; do not guess for non-trivial work |
| Owner | Which agent, or leave it in the queue | Suggest one from `tower ps` by role name |
| Timing | Now, when the agent is free, at a time, or recurring | Now if the owner is idle, else when free |
| Lease | Seconds the agent may go silent before the job requeues | 300 for real work; 60 only for trivial jobs |
| Priority, tags | Queue order and filtering | Priority 0; tag `stack:<name>` if the agent belongs to a stack |

If the job repeats ("every day", "each Monday"), it is a schedule. Use
`tower schedule create` in step 3 instead of `tower task create`.

## 2. Write the job and get approval

Write a title of 60 characters or fewer. Write a description that a new
agent with no conversation history can act on:

- the goal, in one or two sentences
- the inputs: paths, branches, URLs, and data the agent needs
- the steps or limits, if the user gave any
- the done criteria, and what to put in the result summary

Show the user the title, description, owner, timing, and lease. Create the
job only after the user approves. If the user changes something, show the
changed job again.

## 3. Create it

Choose one command:

```sh
# owner idle now: assign and deliver at once
tower task create '<title>' --description '<text>' --assign <agent> --lease-s 300

# owner busy, or "when it is free": reserve it
tower task create '<title>' --description '<text>' --assign <agent> --when-available --lease-s 300

# hold until a time (HH:MM, "YYYY-MM-DD HH:MM" local, or RFC 3339)
tower task create '<title>' --description '<text>' --assign <agent> --at 22:00 --lease-s 300

# no owner yet: leave it in the queue for the operator
tower task create '<title>' --description '<text>' --priority 1 --lease-s 300

# recurring: every run is a new job, reserved for the agent
tower schedule create '<title>' --daily 09:00 --assign <agent> --description '<text>' --lease-s 600
tower schedule create '<title>' --cron '0 9 * * MON-FRI' --tz America/Los_Angeles --assign <agent> --description '<text>'
```

Every form takes `--lease-s`. The job keeps its lease for every later
assignment, so set it at creation. Add `--tag <tag>` once for each tag. Quote the description with single
quotes. If it contains a single quote, write `'\''` in its place.

An agent owns one job at a time. A plain `--assign` to a busy agent fails
with `conflict` and names the open job. In that case, tell the user and
create the job with `--when-available`.

## 4. Report

Tell the user:

- the job id (or schedule id and its next run)
- the owner and the timing that applies
- how to follow it: `tower task show <id>` shows the state and the trail
- for a schedule: `tower schedule show <id>`, and that a run is skipped
  while the previous run is still being worked

## Rules

- Do not start, heartbeat, or complete the job yourself. That is the
  owner's work.
- Do not assign a job that another agent owns. Tower refuses it, and you
  must not work around the refusal.
- Do not put secrets in a title or description. Jobs are stored and shown
  in the event log.
- The full CLI contract is in `docs/getting-started.md` §7. The agent side
  is in `docs/agent-loop.md`.
