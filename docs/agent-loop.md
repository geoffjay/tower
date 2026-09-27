# The agent work loop

The contract every tower-managed agent follows. Agents' prompts reference
this document; the server enforces it (design
[§5.2.1](knowledgebase/concepts/design/05-core-objects.md),
[job-queue decision](knowledgebase/decisions/job-queue.md)).

## Rules

1. **You never take work.** There is no claim or pull. Jobs are *assigned*
   to you by the operator (later: an orchestrator). An assignment arrives
   as a prompt naming the job id — the delegation notice.
2. **You own an assigned job exclusively** until you finish it, release
   it, or your lease expires. Nobody else can be assigned it meanwhile.
3. **Declare, heartbeat, report.** Say when you start, keep your lease
   alive, and report the outcome. A job you stop heartbeating goes back
   to the queue for someone else.
4. **A `conflict` answer means you no longer own the job** (lease swept,
   canceled, or reassigned). Stop working on it.

## Identity

| Surface | How tower knows who you are |
|---|---|
| CLI (any harness with a shell, e.g. pi) | `$TOWER_AGENT`, exported into your pane at spawn; `--as <name>` overrides |
| MCP (`POST /mcp`, e.g. claude) | the `X-Tower-Agent` header in your MCP config; an `as` argument overrides |

## The loop

| Step | MCP tool | CLI |
|---|---|---|
| 1. Find your assigned job(s) | `tower_task_list {"mine": true}` | `tower task list --mine $TOWER_AGENT` |
| 2. Read the details | `tower_task_show {"task_id": …}` | `tower task show <id>` |
| 3. Declare start (`assigned` → `working`) | `tower_task_start {"task_id": …}` | `tower task start <id>` |
| 4. Heartbeat while working | `tower_task_heartbeat {"task_id": …}` | `tower task heartbeat <id>` |
| 5. Need a human? Ask | `tower_ask {"text": …, "task_id": …}` | *(block normally; tower turns it into an inbox item)* |
| 6. Finish | `tower_task_status {"task_id": …, "state": "completed", "result": …}` | `tower task status <id> completed --result '…'` |
| or give it back | `tower_task_release {"task_id": …, "reason": …}` | `tower task release <id> --reason '…'` |

**Heartbeat cadence**: every `lease_s / 3` seconds (default lease 60s →
every 20s), at least between turns. Each heartbeat extends ownership by
`lease_s`. If no heartbeat arrives, the lease expires and the job is
requeued with its attempt count bumped; after `max_attempts` (default 3)
it fails with `lease_exhausted` instead.

**Blocked on a human**: when you block (question or permission prompt),
tower marks your job `input-required` and **pauses lease expiry** — a slow
human never costs you the job. The answer arrives as a prompt (questions)
or as your dialog being answered (approvals); your job resumes `working` with a fresh lease.
If nobody answers before the deadline (default 5 min), questions come back
as "proceed with your default or stop", and approvals are **denied**.

**Results**: `result` is free-form JSON; the CLI stores plain text as
`{"summary": "…"}`. Report `failed` (with a reason in `result`) rather
than going silent — silence costs a lease window and an attempt.

## Wiring an MCP client

```console
$ TOKEN_FILE=…   # `tower serve` prints it ("token  <path> (0600)")
$ claude mcp add --transport http tower http://127.0.0.1:8266/mcp \
    --header "Authorization: Bearer $(cat "$TOKEN_FILE")" \
    --header "X-Tower-Agent: backend"
```

The endpoint speaks MCP streamable HTTP (JSON responses, tools only). The
same tools serve the operator: without `X-Tower-Agent` a client acts as the
operator and may use `tower_task_assign`; an agent-identified client may
not.

## For the operator

```console
$ tower task create 'implement CSV error column names' --tag rust
$ tower task assign <id> backend      # exactly one owner; a second assign conflicts
$ tower task show <id>                # assignment / lease / status trail
```
