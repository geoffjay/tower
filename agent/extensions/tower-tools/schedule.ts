import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TowerClient } from "../lib/client";
import { relative } from "../lib/format";
import type { Schedule, ScheduleDetail } from "../lib/model";
import type { AgentNames } from "../lib/snapshot";
import { table, trunc } from "../lib/text";
import { clock } from "../lib/time";
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
	"edit",
	"pause",
	"resume",
	"run",
	"remove",
] as const;

export function registerScheduleTool(
	pi: ExtensionAPI,
	client: TowerClient,
): void {
	const z = pi.zod;
	const parameters = z.object({
		action: z.enum(ACTIONS),
		id: z
			.string()
			.optional()
			.describe("Schedule id: show, edit, pause, resume, run, remove"),
		title: z.string().optional().describe("create (required), edit"),
		description: z
			.string()
			.optional()
			.describe("create, edit: the brief every fired job carries"),
		daily: z
			.string()
			.optional()
			.describe("create, edit: daily at HH:MM (instead of cron)"),
		cron: z
			.string()
			.optional()
			.describe(
				'create, edit: cron expression (5 or 6 fields), e.g. "0 9 * * MON-FRI"',
			),
		timezone: z
			.string()
			.optional()
			.describe(
				"create, edit: IANA zone, e.g. America/Los_Angeles (default: the server's zone)",
			),
		agent: z
			.string()
			.optional()
			.describe(
				"create: the agent its jobs are reserved for (default: the general queue)",
			),
		tags: z
			.array(z.string())
			.optional()
			.describe("create: tags every job carries (stacks use stack:<stack>)"),
		priority: z
			.number()
			.int()
			.optional()
			.describe("create: job priority (default 0)"),
		lease_s: z
			.number()
			.int()
			.min(1)
			.optional()
			.describe("create: job lease in seconds"),
		max_attempts: z
			.number()
			.int()
			.min(1)
			.optional()
			.describe("create: lease expiries before a job fails (default 3)"),
		pre_run: z
			.string()
			.optional()
			.describe(
				"create, edit: prompt typed into the harness before each job, e.g. /clear",
			),
		post_run: z
			.string()
			.optional()
			.describe(
				"create, edit: prompt typed after each job closes, e.g. /reflect",
			),
	});
	pi.registerTool({
		name: "tower_schedule",
		label: "Tower Schedule",
		description: [
			"Recurring jobs: a job template that fires on a cadence and creates an ordinary job each time, reserved for its target agent. Schedules fire only while `tower serve` runs.",
			"list; show (with the jobs it created).",
			'create: `title` (stacks: "<stack>: <title>"), `daily` HH:MM or `cron`, `timezone` (IANA), target `agent`, a standalone `description`, `tags`, `lease_s` (600+ for real work).',
			'edit: change title, description, cadence, timezone, pre_run, post_run from the next firing ("" clears text fields); the target cannot change: remove and create again.',
			"pause; resume (fires again from now); run (one extra firing now); remove (open jobs it created stay).",
			"Policy: a firing is skipped while the previous job is still worked, replaces an undelivered previous job, and missed firings coalesce into one.",
		].join(" "),
		parameters,
		loadMode: "essential",
		approval: approvalByAction(["list", "show"], {
			remove:
				"Removes the schedule; it stops creating jobs (open jobs it created stay).",
		}),
		async execute(_toolCallId, params, signal) {
			const { action } = params;
			const route = (suffix = "") =>
				`/v1/schedules/${encodeURIComponent(need(params.id, "id", action))}${suffix}`;
			switch (action) {
				case "list": {
					const [{ schedules }, names] = await Promise.all([
						client.get<{ schedules: Schedule[] }>("/v1/schedules", { signal }),
						fetchNames(client, signal),
					]);
					if (!schedules.length) return textResult("No schedules.");
					const now = Date.now();
					const rows = schedules.map((schedule) => [
						schedule.id,
						schedule.enabled ? "active" : "paused",
						schedule.enabled && schedule.next_run_at
							? clock(schedule.next_run_at, now)
							: "-",
						schedule.last_run_at ? clock(schedule.last_run_at, now) : "-",
						names(schedule.target_agent_id) ?? "queue",
						`${schedule.cron} ${schedule.timezone}`,
						trunc(schedule.title, 60),
					]);
					return textResult(
						table(
							["ID", "STATE", "NEXT", "LAST", "TARGET", "CADENCE", "TITLE"],
							rows,
						),
					);
				}
				case "show": {
					const [detail, names] = await Promise.all([
						client.get<ScheduleDetail>(route(), { signal }),
						fetchNames(client, signal),
					]);
					return textResult(showSchedule(detail, names));
				}
				case "create": {
					if (!params.daily && !params.cron)
						throw new Error("create needs `daily` (HH:MM) or `cron`");
					const { schedule } = await client.post<{ schedule: Schedule }>(
						"/v1/schedules",
						{
							title: need(params.title, "title", action),
							description: params.description,
							daily: params.daily,
							cron: params.cron,
							timezone: params.timezone,
							target: params.agent,
							tags: params.tags ?? [],
							priority: params.priority,
							lease_s: params.lease_s,
							max_attempts: params.max_attempts,
							pre_run: params.pre_run,
							post_run: params.post_run,
						},
						{ signal },
					);
					return textResult(
						`created ${scheduleLine(schedule, await fetchNames(client, signal))}`,
					);
				}
				case "edit": {
					const body: Record<string, unknown> = {};
					if (params.title !== undefined) body.title = params.title;
					if (params.description !== undefined)
						body.description = params.description || null;
					if (params.daily !== undefined) body.daily = params.daily;
					if (params.cron !== undefined) body.cron = params.cron;
					if (params.timezone !== undefined) body.timezone = params.timezone;
					if (params.pre_run !== undefined)
						body.pre_run = params.pre_run || null;
					if (params.post_run !== undefined)
						body.post_run = params.post_run || null;
					const fields = Object.keys(body);
					if (!fields.length) {
						throw new Error(
							"edit needs at least one of title, description, daily, cron, timezone, pre_run, post_run",
						);
					}
					const { schedule } = await client.patch<{ schedule: Schedule }>(
						route(),
						body,
						{ signal },
					);
					return textResult(
						`updated (${fields.join(", ")}): ${scheduleLine(schedule, await fetchNames(client, signal))}`,
					);
				}
				case "pause":
				case "resume": {
					const { schedule } = await client.post<{ schedule: Schedule }>(
						route(`/${action}`),
						undefined,
						{ signal },
					);
					return textResult(
						`${action === "pause" ? "paused" : "resumed"}: ${scheduleLine(schedule, await fetchNames(client, signal))}`,
					);
				}
				case "run": {
					const { fired } = await client.post<{
						fired: { outcome: string; task_id?: string };
					}>(route("/run"), undefined, {
						signal,
					});
					return textResult(
						`fired ${params.id}: ${fired.outcome}${fired.task_id ? ` (job ${fired.task_id})` : ""}`,
					);
				}
				case "remove":
					await client.delete(route(), { signal });
					return textResult(
						`removed schedule ${params.id} (open jobs it created stay)`,
					);
			}
		},
		renderCall(args, _options, theme) {
			const detail = [
				args.action,
				args.id ?? "",
				quoted(args.title),
				args.daily ? `daily ${args.daily}` : "",
				args.cron ?? "",
			];
			if (args.agent) detail.push(`→ ${args.agent}`);
			return callLine(
				"tower schedule",
				detail.filter(Boolean).join(" "),
				theme,
			);
		},
		renderResult(result, options, theme) {
			return textBody(result, options, theme);
		},
	});
}

function scheduleLine(schedule: Schedule, names: AgentNames): string {
	const next = schedule.enabled
		? schedule.next_run_at
			? `next ${clock(schedule.next_run_at)}`
			: "no next run"
		: "paused";
	return `${schedule.id} "${schedule.title}" ${schedule.cron} ${schedule.timezone} → ${names(schedule.target_agent_id) ?? "queue"}, ${next}`;
}

function showSchedule(detail: ScheduleDetail, names: AgentNames): string {
	const { schedule, jobs } = detail;
	const now = Date.now();
	const lines = [
		`schedule ${schedule.id} "${schedule.title}" (${schedule.enabled ? "active" : "paused"})`,
		`cadence ${schedule.cron} ${schedule.timezone} · target ${names(schedule.target_agent_id) ?? "general queue"} · next ${schedule.enabled && schedule.next_run_at ? `${clock(schedule.next_run_at, now)} (${relative(schedule.next_run_at, now)})` : "-"} · last ${schedule.last_run_at ? clock(schedule.last_run_at, now) : "never"}`,
		`jobs: priority ${schedule.priority} · lease ${schedule.lease_s}s · ${schedule.max_attempts} attempts${schedule.tags.length ? ` · tags ${schedule.tags.join(", ")}` : ""}`,
	];
	if (schedule.pre_run) lines.push(`pre-run: ${schedule.pre_run}`);
	if (schedule.post_run) lines.push(`post-run: ${schedule.post_run}`);
	if (schedule.description)
		lines.push("", "description:", schedule.description);
	if (jobs.length) {
		lines.push("", "recent jobs:");
		lines.push(
			table(
				["ID", "STATE", "OWNER", "UPDATED"],
				jobs.map((job) => [
					job.id,
					job.state,
					names(job.owner_id) ??
						(job.target_agent_id ? `→${names(job.target_agent_id)}` : "-"),
					clock(job.updated_at, now),
				]),
			),
		);
	}
	return lines.join("\n");
}
