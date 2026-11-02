/**
 * Fleet snapshot: one parallel read of everything an operator overview needs,
 * and the derived `FleetView` that tools, the welcome card, and the widget
 * render. The view is small and JSON-safe, so it can ride in tool `details`
 * and session entries.
 */

import type { RequestOptions, TowerClient } from "./client";
import type { Agent, Health, Machine, Message, Schedule, Task } from "./model";
import { OPEN_TASK_STATES } from "./model";
import { contextTail, messageText, oneLine, resultText, trunc } from "./text";
import { duration } from "./time";

/** Closed jobs updated within this window count as recent results. */
export const RECENT_WINDOW_MS = 24 * 60 * 60 * 1000;

export interface Snapshot {
	at: number;
	baseUrl: string;
	health: Health;
	agents: Agent[];
	/** Pending questions and approvals addressed to the operator. */
	inbox: Message[];
	/** Open jobs: queued, assigned, working, input-required. */
	open: Task[];
	/** Jobs of any state updated within `RECENT_WINDOW_MS`. */
	recent: Task[];
	schedules: Schedule[];
	machines: Machine[];
}

/** Open jobs, in lifecycle order: the list route filters one state per call. `query` adds filters (`&tags=…`). */
export async function fetchOpenTasks(
	client: TowerClient,
	options: RequestOptions = {},
	query = "",
): Promise<Task[]> {
	const pages = await Promise.all(
		OPEN_TASK_STATES.map((state) =>
			client.get<{ tasks: Task[] }>(
				`/v1/tasks?state=${state}${query}`,
				options,
			),
		),
	);
	return pages.flatMap((page) => page.tasks);
}

export async function fetchSnapshot(
	client: TowerClient,
	options: RequestOptions = {},
): Promise<Snapshot> {
	const since = Date.now() - RECENT_WINDOW_MS;
	const [health, agents, inbox, schedules, machines, recent, open] =
		await Promise.all([
			client.get<Health>("/healthz", options),
			client.get<{ agents: Agent[] }>("/v1/agents", options),
			client.get<{ messages: Message[] }>(
				"/v1/messages?to=me&status=pending",
				options,
			),
			client.get<{ schedules: Schedule[] }>("/v1/schedules", options),
			client.get<{ machines: Machine[] }>("/v1/machines", options),
			client.get<{ tasks: Task[] }>(`/v1/tasks?since=${since}`, options),
			fetchOpenTasks(client, options),
		]);
	return {
		at: Date.now(),
		baseUrl: client.describe(),
		health,
		agents: agents.agents,
		inbox: inbox.messages,
		open,
		recent: recent.tasks,
		schedules: schedules.schedules,
		machines: machines.machines,
	};
}

/** The stack an agent belongs to by the `<stack>-<role>` naming convention. */
export function stackOf(name: string): string | undefined {
	const dash = name.indexOf("-");
	return dash > 0 ? name.slice(0, dash) : undefined;
}

/** Agent id → name; names, unknown ids, and `undefined` pass through. Jobs and schedules refer to agents by id. */
export type AgentNames = (idOrName: string | undefined) => string | undefined;

export function agentNames(agents: Agent[]): AgentNames {
	const byId = new Map(agents.map((agent) => [agent.id, agent.name]));
	return (idOrName) =>
		idOrName === undefined ? undefined : (byId.get(idOrName) ?? idOrName);
}

export type AttentionLevel = "error" | "warn" | "info";

export interface AttentionItem {
	level: AttentionLevel;
	text: string;
}

export interface AgentRow {
	name: string;
	kind: string;
	state: string;
	machine: string;
	job?: string;
	jobId?: string;
	note?: string;
}

export interface InboxRow {
	id: string;
	from: string;
	kind: string;
	at: number;
	deadline?: number;
	text: string;
	context?: string;
}

export interface JobRow {
	id: string;
	state: string;
	owner?: string;
	target?: string;
	title: string;
	priority: number;
	attempts: string;
	notBefore?: number;
	hook?: string;
}

export interface ScheduleRow {
	id: string;
	title: string;
	cadence: string;
	target?: string;
	next?: number;
	enabled: boolean;
}

export interface FleetView {
	at: number;
	url: string;
	version?: string;
	/** Set when tower could not be read; the other fields are then empty. */
	error?: string;
	stack?: string;
	agents: AgentRow[];
	inbox: InboxRow[];
	jobs: JobRow[];
	schedules: ScheduleRow[];
	recent: {
		hours: number;
		completed: number;
		failed: number;
		canceled: number;
	};
	machines: { name: string; status: string }[];
	attention: AttentionItem[];
}

export function offlineView(
	url: string,
	error: string,
	at: number = Date.now(),
): FleetView {
	return {
		at,
		url,
		error,
		agents: [],
		inbox: [],
		jobs: [],
		schedules: [],
		recent: {
			hours: RECENT_WINDOW_MS / 3_600_000,
			completed: 0,
			failed: 0,
			canceled: 0,
		},
		machines: [],
		attention: [],
	};
}

/** Derive the operator view, optionally narrowed to one stack (`<stack>-*` agents, `stack:<stack>` tags). */
export function buildView(snapshot: Snapshot, stack?: string): FleetView {
	const now = snapshot.at;
	const name = agentNames(snapshot.agents);
	const inStack = (agent: string | undefined) =>
		stack === undefined || (agent !== undefined && stackOf(agent) === stack);
	const taskInStack = (task: Task) =>
		stack === undefined ||
		task.tags.includes(`stack:${stack}`) ||
		inStack(name(task.owner_id)) ||
		inStack(name(task.target_agent_id));

	const agents = snapshot.agents.filter((agent) => inStack(agent.name));
	const open = snapshot.open.filter(taskInStack);
	const recent = snapshot.recent.filter(taskInStack);
	const inbox = snapshot.inbox.filter((message) => inStack(message.from_id));
	const schedules = snapshot.schedules.filter(
		(schedule) =>
			stack === undefined ||
			schedule.title.startsWith(`${stack}: `) ||
			schedule.tags.includes(`stack:${stack}`) ||
			inStack(name(schedule.target_agent_id)),
	);
	const ownedBy = new Map(
		open
			.filter((task) => task.owner_id)
			.map((task) => [name(task.owner_id), task]),
	);

	const attention: AttentionItem[] = [];
	for (const message of inbox) {
		const expires = message.deadline_at
			? `, expires in ${duration(message.deadline_at - now)}`
			: "";
		attention.push({
			level: "warn",
			text: `${message.from_id} ${message.kind} ${duration(now - message.created_at)} ago [${message.id}]${expires}: "${trunc(oneLine(messageText(message)), 120)}"`,
		});
	}
	const asking = new Set(inbox.map((message) => message.from_id));
	for (const agent of agents) {
		if (agent.state === "blocked" && !asking.has(agent.name)) {
			attention.push({
				level: "warn",
				text: `${agent.name} is blocked with no inbox item; read its screen`,
			});
		}
		if (agent.state === "dead") {
			const job = ownedBy.get(agent.name);
			const owns = job
				? `; its job ${job.id} requeues when the lease expires`
				: "";
			attention.push({ level: "error", text: `${agent.name} is dead${owns}` });
		}
	}
	for (const task of recent
		.filter((task) => task.state === "failed")
		.slice(-3)) {
		attention.push({
			level: "error",
			text: `job ${task.id} "${trunc(task.title, 60)}" failed (${name(task.owner_id) ?? "no owner"}): ${trunc(oneLine(resultText(task.result)), 160)}`,
		});
	}
	for (const task of open) {
		if (task.max_attempts > 1 && task.attempt_count >= task.max_attempts - 1) {
			attention.push({
				level: "warn",
				text: `job ${task.id} is on its last attempt (${task.attempt_count}/${task.max_attempts})`,
			});
		}
		if (
			task.state === "assigned" &&
			task.hook !== "pre" &&
			now - task.updated_at > task.lease_s * 1000
		) {
			attention.push({
				level: "warn",
				text: `job ${task.id} was assigned to ${name(task.owner_id)} ${duration(now - task.updated_at)} ago and never started`,
			});
		}
	}
	// Info-level items collapse to one line each when they pile up: a paused
	// stack must not bury the inbox and failures.
	const unowned = open.filter(
		(task) => task.state === "queued" && !task.target_agent_id,
	);
	if (unowned.length > 2) {
		attention.push({
			level: "info",
			text: `${unowned.length} queued jobs wait for an owner`,
		});
	} else {
		for (const task of unowned) {
			attention.push({
				level: "info",
				text: `job ${task.id} "${trunc(task.title, 60)}" waits for an owner`,
			});
		}
	}
	const paused = schedules.filter((schedule) => !schedule.enabled);
	if (paused.length > 2) {
		attention.push({
			level: "info",
			text: `${paused.length} schedules are paused`,
		});
	} else {
		for (const schedule of paused) {
			attention.push({
				level: "info",
				text: `schedule ${schedule.id} "${trunc(schedule.title, 60)}" is paused`,
			});
		}
	}

	return {
		at: now,
		url: snapshot.baseUrl,
		version: snapshot.health.version,
		stack,
		agents: agents.map((agent) => {
			const job = ownedBy.get(agent.name);
			const notes = [
				agent.adopted ? "adopted" : "",
				agent.worktree ? "worktree" : "",
				agent.model ? agent.model : "",
				agent.desired_state === "stopped" && agent.state !== "dead"
					? "stopping"
					: "",
			].filter(Boolean);
			return {
				name: agent.name,
				kind: agent.kind,
				state: agent.state,
				machine: agent.machine_id,
				job: job?.title,
				jobId: job?.id,
				note: notes.join(", ") || undefined,
			};
		}),
		inbox: inbox.map((message) => ({
			id: message.id,
			from: message.from_id,
			kind: message.kind,
			at: message.created_at,
			deadline: message.deadline_at,
			text: messageText(message),
			context: contextTail(message, 12),
		})),
		jobs: open.map((task) => ({
			id: task.id,
			state: task.state,
			owner: name(task.owner_id),
			target: name(task.target_agent_id),
			title: task.title,
			priority: task.priority,
			attempts: `${task.attempt_count}/${task.max_attempts}`,
			notBefore: task.not_before,
			hook: task.hook,
		})),
		schedules: schedules
			.map((schedule) => ({
				id: schedule.id,
				title: schedule.title,
				cadence: `${schedule.cron} ${schedule.timezone}`,
				target: name(schedule.target_agent_id),
				next: schedule.enabled ? schedule.next_run_at : undefined,
				enabled: schedule.enabled,
			}))
			.sort(
				(a, b) =>
					(a.next ?? Number.MAX_SAFE_INTEGER) -
					(b.next ?? Number.MAX_SAFE_INTEGER),
			),
		recent: {
			hours: RECENT_WINDOW_MS / 3_600_000,
			completed: recent.filter((task) => task.state === "completed").length,
			failed: recent.filter((task) => task.state === "failed").length,
			canceled: recent.filter(
				(task) => task.state === "canceled" || task.state === "rejected",
			).length,
		},
		machines: snapshot.machines.map((machine) => ({
			name: machine.name,
			status: machine.status,
		})),
		attention,
	};
}

/** Tally rows by a string key: agent states for headers, job states for queue summaries. */
export function countBy<T>(
	rows: readonly T[],
	key: (row: T) => string,
): Record<string, number> {
	const counts: Record<string, number> = {};
	for (const row of rows) counts[key(row)] = (counts[key(row)] ?? 0) + 1;
	return counts;
}
