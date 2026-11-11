import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TowerClient } from "../lib/client";
import { relative } from "../lib/format";
import {
	isOpenTask,
	TASK_STATES,
	type Task,
	type TaskDetail,
	type TowerEvent,
} from "../lib/model";
import { type AgentNames, fetchOpenTasks } from "../lib/snapshot";
import { messageText, oneLine, resultText, table, trunc } from "../lib/text";
import { clock, parseLocalTime } from "../lib/time";
import {
	approvalByAction,
	callLine,
	fetchNames,
	need,
	quoted,
	textBody,
	textResult,
} from "./common";

const ACTIONS = [
	"list",
	"show",
	"create",
	"assign",
	"edit",
	"cancel",
	"release",
] as const;

export function registerTaskTool(pi: ExtensionAPI, client: TowerClient): void {
	const z = pi.zod;
	const parameters = z.object({
		action: z.enum(ACTIONS),
		id: z
			.string()
			.optional()
			.describe("Job id: show, assign, edit, cancel, release"),
		state: z
			.enum(TASK_STATES)
			.optional()
			.describe("list: only this state (default: every open state)"),
		tags: z
			.array(z.string())
			.optional()
			.describe(
				"list: jobs carrying every tag. create: tags to set (stacks use stack:<stack>)",
			),
		owner: z
			.string()
			.optional()
			.describe("list: jobs this agent owns or owned, any state"),
		limit: z
			.number()
			.int()
			.min(1)
			.max(500)
			.optional()
			.describe("list: maximum rows (default 50)"),
		title: z
			.string()
			.optional()
			.describe("create (required), edit: at most ~60 characters"),
		description: z
			.string()
			.optional()
			.describe("create, edit: the full brief the agent works from"),
		priority: z
			.number()
			.int()
			.optional()
			.describe("create: higher runs first (default 0)"),
		agent: z
			.string()
			.optional()
			.describe(
				"create: the owner (or reservation target). assign: the new owner. Delivered when the agent is free, else reserved for it",
			),
		at: z
			.string()
			.optional()
			.describe(
				'create, edit: hold a reserved job until HH:MM, "YYYY-MM-DD HH:MM" (local), or RFC 3339',
			),
		lease_s: z
			.number()
			.int()
			.min(1)
			.optional()
			.describe(
				"create, assign: seconds the owner may go silent before the job requeues (default 60; use 300+ for real work)",
			),
		max_attempts: z
			.number()
			.int()
			.min(1)
			.optional()
			.describe("create: lease expiries before the job fails (default 3)"),
		pre_run: z
			.string()
			.optional()
			.describe(
				"create, edit: prompt typed into the harness before each delivery, e.g. /clear",
			),
		post_run: z
			.string()
			.optional()
			.describe(
				"create, edit: prompt typed after the owner reports completed or failed, e.g. /reflect",
			),
		reason: z
			.string()
			.optional()
			.describe("release: why the job goes back to the queue"),
	});
	pi.registerTool({
		name: "tower_task",
		label: "Tower Job",
		description: [
			"The job queue (API and CLI name: task). Agents never take work: the operator creates and assigns it.",
			"list: open jobs by default; `state` picks one state (closed states too), `tags` and `owner` filter.",
			"show: one job with description, result, lifecycle prompts, trail, and messages.",
			"create: `title` and a standalone `description` (goal, inputs, limits, done criteria, what to report); with `agent` it is delivered when the agent is free, else reserved for it until it is; `at` holds it until a time.",
			"assign: dispatch queued job `id` to `agent`. An agent owns one open job at a time: a busy, claimed, or dead agent gets the job reserved, and the result says why it waits.",
			'edit: change `title`, `description`, `pre_run`, `post_run`, `at` ("" clears); queued jobs take every field, delivered jobs only post_run.',
			"cancel: cancel and interrupt the owner. release: return the job to the queue for its owner.",
		].join(" "),
		parameters,
		loadMode: "essential",
		approval: approvalByAction(["list", "show"]),
		async execute(_toolCallId, params, signal) {
			const { action } = params;
			const route = (suffix = "") =>
				`/v1/tasks/${encodeURIComponent(need(params.id, "id", action))}${suffix}`;
			switch (action) {
				case "list":
					return textResult(await listJobs(client, params, signal));
				case "show": {
					const [detail, names] = await Promise.all([
						client.get<TaskDetail>(route(), { signal }),
						fetchNames(client, signal),
					]);
					return textResult(showJob(detail, names));
				}
				case "create": {
					const { task, waiting } = await client.post<Assigned>(
						"/v1/tasks",
						{
							title: need(params.title, "title", action),
							description: params.description,
							priority: params.priority,
							tags: params.tags ?? [],
							assign: params.agent,
							not_before: params.at ? parseLocalTime(params.at) : undefined,
							lease_s: params.lease_s,
							max_attempts: params.max_attempts,
							pre_run: params.pre_run,
							post_run: params.post_run,
						},
						{ signal },
					);
					return textResult(
						`created ${describeJob(task, await fetchNames(client, signal), waiting)}`,
					);
				}
				case "assign": {
					const { task, waiting } = await client.post<Assigned>(
						route("/assign"),
						{
							to: need(params.agent, "agent", action),
							lease_s: params.lease_s,
						},
						{ signal },
					);
					return textResult(
						describeJob(task, await fetchNames(client, signal), waiting),
					);
				}
				case "edit": {
					const body: Record<string, unknown> = {};
					if (params.title !== undefined) body.title = params.title;
					if (params.description !== undefined)
						body.description = params.description || null;
					if (params.pre_run !== undefined)
						body.pre_run = params.pre_run || null;
					if (params.post_run !== undefined)
						body.post_run = params.post_run || null;
					if (params.at !== undefined)
						body.not_before = params.at ? parseLocalTime(params.at) : null;
					const fields = Object.keys(body);
					if (!fields.length)
						throw new Error(
							"edit needs at least one of title, description, pre_run, post_run, at",
						);
					const { task } = await client.patch<{ task: Task }>(route(), body, {
						signal,
					});
					return textResult(
						`updated ${task.id} (${fields.join(", ")}): ${task.state} "${task.title}"`,
					);
				}
				case "cancel": {
					const { task } = await client.post<{ task: Task }>(
						route("/cancel"),
						undefined,
						{ signal },
					);
					return textResult(
						`canceled ${task.id} "${task.title}" (its owner, if any, was interrupted)`,
					);
				}
				case "release": {
					const { task: before } = await client.get<{ task: Task }>(route(), {
						signal,
					});
					if (!before.owner_id)
						throw new Error(
							`job ${before.id} has no owner (state ${before.state})`,
						);
					const { task } = await client.post<{ task: Task }>(
						route("/release"),
						{ as: before.owner_id, reason: params.reason },
						{ signal },
					);
					return textResult(
						`released ${task.id} back to the queue (${task.state})`,
					);
				}
			}
		},
		renderCall(args, _options, theme) {
			const detail = [
				args.action,
				args.id ?? "",
				args.state ?? "",
				quoted(args.title),
			];
			if (args.agent) detail.push(`→ ${args.agent}`);
			if (args.at) detail.push(`at ${args.at}`);
			return callLine("tower job", detail.filter(Boolean).join(" "), theme);
		},
		renderResult(result, options, theme) {
			return textBody(result, options, theme);
		},
	});
}

/** Why an assignment left the job reserved instead of delivering it. */
interface Waiting {
	reason: string;
	agent_state: string;
	busy_with?: string;
	held_by?: string;
}

/** The answer of create and assign: `waiting` is null when delivered. */
interface Assigned {
	task: Task;
	waiting?: Waiting | null;
}

interface ListParams {
	state?: (typeof TASK_STATES)[number];
	tags?: string[];
	owner?: string;
	limit?: number;
}

async function listJobs(
	client: TowerClient,
	params: ListParams,
	signal?: AbortSignal,
): Promise<string> {
	const filters = [
		params.tags?.length
			? `&tags=${encodeURIComponent(params.tags.join(","))}`
			: "",
		params.owner ? `&owner=${encodeURIComponent(params.owner)}` : "",
	].join("");
	const [tasks, names] = await Promise.all([
		params.state || params.owner
			? client
					.get<{ tasks: Task[] }>(
						`/v1/tasks?${params.state ? `state=${params.state}` : ""}${filters}`,
						{ signal },
					)
					.then((page) => page.tasks)
			: fetchOpenTasks(client, { signal }, filters),
		fetchNames(client, signal),
	]);
	const closedFirst = params.state
		? !isOpenTask(params.state)
		: Boolean(params.owner);
	const rows = closedFirst
		? [...tasks].sort((a, b) => b.updated_at - a.updated_at)
		: tasks;
	const limit = params.limit ?? 50;
	if (!rows.length)
		return params.state || params.owner || params.tags
			? "No jobs match."
			: "No open jobs.";
	const now = Date.now();
	const text = table(
		["ID", "STATE", "OWNER", "PRI", "TRIES", "UPDATED", "TITLE"],
		rows
			.slice(0, limit)
			.map((task) => [
				task.id,
				task.hook ? `${task.state} (${task.hook}-run)` : task.state,
				names(task.owner_id) ??
					(task.target_agent_id ? `→${names(task.target_agent_id)}` : "-"),
				String(task.priority),
				`${task.attempt_count}/${task.max_attempts}`,
				clock(task.updated_at, now),
				trunc(task.title, 70),
			]),
	);
	return rows.length > limit
		? `${text}\n… ${rows.length - limit} more (raise limit)`
		: text;
}

/** One line: id, state, and who has it or will get it; `waiting` is the
 * server's reason a reserved job waits. */
function describeJob(
	task: Task,
	names: AgentNames,
	waiting?: Waiting | null,
): string {
	const now = Date.now();
	let where = "queued for the operator to assign";
	if (task.owner_id) {
		where = `assigned to ${names(task.owner_id)} (lease ${task.lease_s}s${task.hook === "pre" ? ", pre-run prompt first" : ""})`;
	} else if (task.target_agent_id) {
		const after =
			task.not_before && task.not_before > now
				? `, not before ${clock(task.not_before, now)}`
				: "";
		const why = waiting?.reason ?? "delivered when it is free";
		where = `reserved for ${names(task.target_agent_id)} (${why}${after})`;
	}
	return `${task.id} ${task.state}: ${where} — "${task.title}"`;
}

function showJob(detail: TaskDetail, names: AgentNames): string {
	const { task, trail, messages } = detail;
	const now = Date.now();
	const lease = task.lease_expires_at
		? `, expires ${relative(task.lease_expires_at, now)}`
		: "";
	const lines = [
		`job ${task.id} "${task.title}"`,
		`state ${task.state}${task.hook ? ` (${task.hook}-run prompt running)` : ""} · owner ${names(task.owner_id) ?? "-"}${task.target_agent_id ? ` · reserved for ${names(task.target_agent_id)}` : ""} · priority ${task.priority} · tries ${task.attempt_count}/${task.max_attempts} · lease ${task.lease_s}s${lease}`,
		`origin ${task.origin}${task.schedule_id ? ` (schedule ${task.schedule_id})` : ""} · created ${clock(task.created_at, now)} · updated ${relative(task.updated_at, now)}${task.not_before ? ` · not before ${clock(task.not_before, now)}` : ""}`,
	];
	if (task.tags.length) lines.push(`tags ${task.tags.join(", ")}`);
	if (task.pre_run) lines.push(`pre-run: ${task.pre_run}`);
	if (task.post_run) lines.push(`post-run: ${task.post_run}`);
	if (task.description) lines.push("", "description:", task.description);
	if (task.result !== undefined)
		lines.push("", `result: ${resultText(task.result)}`);
	if (trail.length) {
		lines.push("", "trail:");
		for (const event of trail)
			lines.push(`  ${clock(event.ts, now)}  ${trailLine(event, names)}`);
	}
	if (messages.length) {
		lines.push("", "messages:");
		for (const message of messages) {
			lines.push(
				`  ${clock(message.created_at, now)}  ${message.kind} ${message.from_id}→${message.to_id} ${message.status}: ${trunc(oneLine(messageText(message)), 200)}`,
			);
		}
	}
	return lines.join("\n");
}

function trailLine(event: TowerEvent, names: AgentNames): string {
	const payload = event.payload;
	const text = (key: string) => {
		const value = payload[key];
		return typeof value === "string" ? value : undefined;
	};
	const time = (key: string) => {
		const value = payload[key];
		return typeof value === "number" ? clock(value) : undefined;
	};
	switch (event.kind) {
		case "task.created":
			return "created";
		case "task.assigned":
			return `assigned to ${names(text("owner_id"))} by ${text("by") ?? "?"}${time("lease_expires_at") ? ` (lease until ${time("lease_expires_at")})` : ""}`;
		case "task.reserved":
			return `reserved for ${names(text("target"))} by ${text("by") ?? "?"}${time("not_before") ? ` (not before ${time("not_before")})` : ""}`;
		case "task.status": {
			if (text("released_by"))
				return `released by ${names(text("released_by"))}: ${text("reason") ?? "no reason"}`;
			return `status ${text("state") ?? "?"}${text("reason") ? ` (${text("reason")})` : ""}`;
		}
		case "task.completed":
		case "task.failed":
			return `${event.kind.slice(5)} by ${names(text("owner_id")) ?? "?"}: ${trunc(oneLine(resultText(payload.result)), 200)}`;
		case "task.leased_out":
			return `lease expired for ${names(text("prior_owner"))} (attempt ${String(payload.attempt_count)}/${String(payload.max_attempts)}) → ${text("state") ?? "?"}`;
		case "task.updated":
			return `edited ${JSON.stringify(payload.fields ?? [])}`;
		case "task.hook":
			return `${text("phase") ?? "?"}-run prompt ${text("status") ?? "?"}${text("error") ? `: ${text("error")}` : ""}`;
		default:
			return `${event.kind} ${trunc(JSON.stringify(payload), 160)}`;
	}
}
