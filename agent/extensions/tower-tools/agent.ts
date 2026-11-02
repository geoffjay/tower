import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TowerClient } from "../lib/client";
import { relative } from "../lib/format";
import type { Agent, Message, Schedule, Task } from "../lib/model";
import { agentNames, fetchOpenTasks } from "../lib/snapshot";
import { messageText, oneLine, table, trunc } from "../lib/text";
import { clock } from "../lib/time";
import {
	approvalByAction,
	callLine,
	need,
	quoted,
	textBody,
	textResult,
} from "./common";

const ACTIONS = [
	"list",
	"show",
	"read",
	"prompt",
	"interrupt",
	"spawn",
	"stop",
	"remove",
] as const;
const SPAWN_TIMEOUT_MS = 120_000;

export function registerAgentTool(pi: ExtensionAPI, client: TowerClient): void {
	const z = pi.zod;
	const parameters = z.object({
		action: z.enum(ACTIONS),
		name: z
			.string()
			.optional()
			.describe("Agent name (or id). Required except for list"),
		text: z
			.string()
			.optional()
			.describe(
				"prompt: the text to type. spawn: the first prompt (standing brief)",
			),
		wait: z
			.boolean()
			.optional()
			.describe("prompt: wait until the agent settles after delivery"),
		lines: z
			.number()
			.int()
			.min(1)
			.max(400)
			.optional()
			.describe("read: trailing screen lines (default 40)"),
		kind: z
			.string()
			.optional()
			.describe("spawn: harness kind, e.g. omp, claude, pi, opencode, codex"),
		workdir: z
			.string()
			.optional()
			.describe("spawn: absolute working directory"),
		worktree: z
			.boolean()
			.optional()
			.describe(
				"spawn: run in its own git worktree at <workdir>/.worktrees/<name> (for agents sharing a repo)",
			),
		model: z.string().optional().describe("spawn: model id for the harness"),
		args: z
			.array(z.string())
			.optional()
			.describe('spawn: extra harness flags, e.g. ["--smol"]'),
		definition: z
			.boolean()
			.optional()
			.describe(
				"spawn: create from the named definition (needs the tower CLI)",
			),
		adopt: z
			.boolean()
			.optional()
			.describe(
				"spawn: take over a harness already running in herdr, or re-attach a dead agent",
			),
		resume: z
			.string()
			.optional()
			.describe(
				'spawn: relaunch a dead agent with this harness session id ("" = the one herdr last saw)',
			),
		continue: z
			.boolean()
			.optional()
			.describe("spawn: relaunch a dead agent continuing its last session"),
	});
	pi.registerTool({
		name: "tower_agent",
		label: "Tower Agent",
		description: [
			"Tower agents (harnesses in herdr panes), addressed by name.",
			"list: every agent with kind, state, machine, workdir, and its open job.",
			"show: one agent's settings, its open job, jobs reserved for it, schedules that target it, and recent messages.",
			"read: the agent's recent screen output (`lines`, default 40).",
			"prompt: type `text` into the agent's harness (`wait` until it settles).",
			"interrupt: send ctrl+c.",
			"spawn: new agent `name` with `kind`, `workdir`, optional `worktree`, `model`, `args`, and a first prompt in `text`; or `definition: true` to create it from ~/.config/tower/agents/<name>/ (other fields override); or `adopt`/`resume`/`continue` to take over a running harness or relaunch a dead agent.",
			"stop: end the session; the agent row (seat) stays and its jobs wait for their leases.",
			"remove: stop and delete the agent and its worktree (if clean); its open job returns to the queue, its reserved jobs lose their target, its schedules pause.",
		].join(" "),
		parameters,
		loadMode: "essential",
		approval: approvalByAction(["list", "show", "read"], {
			remove:
				"Removes the agent and its worktree; its jobs go back to the queue and its schedules pause.",
		}),
		async execute(_toolCallId, params, signal) {
			const { action } = params;
			const path = (suffix = "") =>
				`/v1/agents/${encodeURIComponent(need(params.name, "name", action))}${suffix}`;
			switch (action) {
				case "list": {
					const [{ agents }, open] = await Promise.all([
						client.get<{ agents: Agent[] }>("/v1/agents", { signal }),
						fetchOpenTasks(client, { signal }),
					]);
					const name = agentNames(agents);
					const jobs = new Map(
						open
							.filter((task) => task.owner_id)
							.map((task) => [name(task.owner_id), task]),
					);
					const rows = agents.map((agent) => {
						const job = jobs.get(agent.name);
						return [
							agent.name,
							agent.kind,
							agent.state,
							agent.machine_id,
							agent.worktree ?? agent.workdir ?? "-",
							job ? `${trunc(job.title, 40)} [${job.id}]` : "-",
						];
					});
					const text = agents.length
						? table(
								["NAME", "KIND", "STATE", "MACHINE", "WORKDIR", "JOB"],
								rows,
							)
						: "No agents. Spawn one with action spawn.";
					return textResult(text, { agents: agents.length });
				}
				case "show":
					return textResult(
						await showAgent(client, need(params.name, "name", action), signal),
					);
				case "read": {
					const { output } = await client.get<{ output: string }>(
						path("/read"),
						{ signal },
					);
					const lines = output.split("\n").map((line) => line.trimEnd());
					while (lines.length && !lines[lines.length - 1]) lines.pop();
					const tail = lines.slice(-(params.lines ?? 40)).join("\n");
					return textResult(tail || "(empty screen)");
				}
				case "prompt": {
					const text = need(params.text, "text", action);
					const reply = await client.post<{
						state?: string;
						stalled?: boolean;
					}>(
						path("/prompt"),
						{ text, wait: params.wait ?? false },
						{ signal, timeoutMs: params.wait ? 120_000 : undefined },
					);
					const stalled = reply.stalled
						? "; it has not started on it yet (stalled)"
						: "";
					return textResult(
						`prompt delivered to ${params.name}${reply.state ? ` (state ${reply.state})` : ""}${stalled}`,
					);
				}
				case "interrupt":
					await client.post(path("/interrupt"), undefined, { signal });
					return textResult(`interrupted ${params.name} (ctrl+c)`);
				case "spawn":
					return textResult(await spawn(pi, client, params, signal));
				case "stop":
				case "remove": {
					const remove = action === "remove";
					const reply = await client.post<{
						worktree?: { path: string; removed: boolean; reason?: string };
					}>(
						path("/stop"),
						{ remove },
						{ signal, timeoutMs: SPAWN_TIMEOUT_MS },
					);
					const lines = [
						remove
							? `removed ${params.name}`
							: `stopped ${params.name} (the agent row stays)`,
					];
					const worktree = reply.worktree;
					if (worktree) {
						lines.push(
							worktree.removed
								? `worktree removed: ${worktree.path}`
								: `worktree kept: ${worktree.path}${worktree.reason ? ` (${worktree.reason})` : ""}`,
						);
					}
					return textResult(lines.join("\n"));
				}
			}
		},
		renderCall(args, _options, theme) {
			const detail = [
				args.action,
				args.name ?? "",
				args.action === "prompt" ? quoted(args.text) : "",
			];
			if (args.action === "spawn" && args.kind) detail.push(`(${args.kind})`);
			return callLine("tower agent", detail.filter(Boolean).join(" "), theme);
		},
		renderResult(result, options, theme) {
			return textBody(result, options, theme);
		},
	});
}

async function showAgent(
	client: TowerClient,
	name: string,
	signal?: AbortSignal,
): Promise<string> {
	const encoded = encodeURIComponent(name);
	const [{ agent }, mine, queued, schedules, messages] = await Promise.all([
		client.get<{ agent: Agent }>(`/v1/agents/${encoded}`, { signal }),
		client.get<{ tasks: Task[] }>(`/v1/tasks?mine=${encoded}`, { signal }),
		client.get<{ tasks: Task[] }>("/v1/tasks?state=queued", { signal }),
		client.get<{ schedules: Schedule[] }>("/v1/schedules", { signal }),
		client.get<{ messages: Message[] }>(`/v1/messages?agent=${encoded}`, {
			signal,
		}),
	]);
	const now = Date.now();
	const lines = [
		`${agent.name}  ${agent.kind}  ${agent.state}  (id ${agent.id})`,
		`machine ${agent.machine_id} · pane ${agent.pane_id ?? "-"} · permissions ${agent.permissions} · desired ${agent.desired_state}${agent.adopted ? " · adopted" : ""}`,
		`workdir ${agent.workdir ?? "-"}${agent.worktree ? ` · worktree ${agent.worktree}` : ""}`,
	];
	if (agent.model || agent.args.length) {
		lines.push(
			`model ${agent.model ?? "(harness default)"}${agent.args.length ? ` · args ${agent.args.join(" ")}` : ""}`,
		);
	}
	lines.push(
		`created ${clock(agent.created_at, now)} · updated ${relative(agent.updated_at, now)}`,
	);

	const job = mine.tasks[0];
	lines.push(
		"",
		job
			? `Open job: ${job.id} ${job.state} "${job.title}"${job.lease_expires_at ? ` · lease ${relative(job.lease_expires_at, now)}` : ""} · tries ${job.attempt_count}/${job.max_attempts}`
			: "Open job: none",
	);
	const reserved = queued.tasks.filter(
		(task) => task.target_agent_id === agent.id,
	);
	if (reserved.length) {
		lines.push("Reserved for it:");
		for (const task of reserved) {
			const after =
				task.not_before && task.not_before > now
					? ` (after ${clock(task.not_before, now)})`
					: "";
			lines.push(`- ${task.id} "${trunc(task.title, 70)}"${after}`);
		}
	}
	const targeting = schedules.schedules.filter(
		(schedule) => schedule.target_agent_id === agent.id,
	);
	if (targeting.length) {
		lines.push("Schedules targeting it:");
		for (const schedule of targeting) {
			const next = schedule.enabled
				? schedule.next_run_at
					? `next ${clock(schedule.next_run_at, now)}`
					: ""
				: "paused";
			lines.push(
				`- ${schedule.id} "${trunc(schedule.title, 60)}" ${schedule.cron} ${schedule.timezone} ${next}`,
			);
		}
	}
	const recent = messages.messages.slice(0, 6);
	if (recent.length) {
		lines.push("Recent messages (newest first):");
		for (const message of recent) {
			const from =
				message.from_id === agent.name || message.from_id === agent.id
					? "agent"
					: message.from_id;
			const to =
				message.to_id === agent.name || message.to_id === agent.id
					? "agent"
					: message.to_id;
			lines.push(
				`- ${clock(message.created_at, now)} ${message.kind} ${from}→${to} ${message.status}: ${trunc(oneLine(messageText(message)), 160)}`,
			);
		}
	}
	return lines.join("\n");
}

interface SpawnParams {
	name?: string;
	text?: string;
	kind?: string;
	workdir?: string;
	worktree?: boolean;
	model?: string;
	args?: string[];
	definition?: boolean;
	adopt?: boolean;
	resume?: string;
	continue?: boolean;
}

async function spawn(
	pi: ExtensionAPI,
	client: TowerClient,
	params: SpawnParams,
	signal?: AbortSignal,
): Promise<string> {
	const name = need(params.name, "name", "spawn");
	let agent: Agent;
	if (params.definition) {
		// Named definitions are read client-side by the tower CLI; let it expand them.
		const args = ["spawn", "--name", name];
		if (params.kind) args.push("--kind", params.kind);
		if (params.workdir) args.push("--workdir", params.workdir);
		if (params.worktree) args.push("--worktree");
		if (params.text) args.push("--prompt", params.text);
		if (params.model) args.push("--model", params.model);
		for (const arg of params.args ?? []) args.push(`--arg=${arg}`);
		const bin = process.env.TOWER_BIN || "tower";
		const result = await pi.exec(bin, args, {
			signal,
			timeout: SPAWN_TIMEOUT_MS,
		});
		if (result.code !== 0) {
			const reason = (result.stderr || result.stdout)
				.trim()
				.replace(/^Error:\s*/, "");
			throw new Error(
				`tower spawn --name ${name} failed${result.killed ? " (timed out)" : ""}: ${reason}`,
			);
		}
		({ agent } = await client.get<{ agent: Agent }>(
			`/v1/agents/${encodeURIComponent(name)}`,
			{ signal },
		));
	} else {
		const relaunch =
			params.adopt || params.resume !== undefined || params.continue;
		if (!relaunch) need(params.kind, "kind", "spawn");
		({ agent } = await client.post<{ agent: Agent }>(
			"/v1/agents",
			{
				name,
				kind: params.kind,
				workdir: params.workdir,
				worktree: params.worktree ?? false,
				prompt: params.text,
				model: params.model,
				args: params.args ?? [],
				adopt: params.adopt ?? false,
				resume: params.resume,
				continue: params.continue ?? false,
			},
			{ signal, timeoutMs: SPAWN_TIMEOUT_MS },
		));
	}
	const lines = [
		`spawned ${agent.name} (${agent.kind}) — state ${agent.state}`,
		`id ${agent.id} · machine ${agent.machine_id} · workdir ${agent.workdir ?? "-"}${agent.worktree ? ` · worktree ${agent.worktree}` : ""}`,
	];
	if (agent.model) lines.push(`model ${agent.model}`);
	lines.push(
		"Confirm it started: wait ~15s, then tower_agent read (look for its reply) and tower_status; a blocked agent usually has an inbox item.",
	);
	return lines.join("\n");
}
