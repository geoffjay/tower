/**
 * Tower API objects as the server serializes them (crates/tower-core/src/objects.rs).
 * Timestamps are ms since the Unix epoch; ids are ULIDs. Agent routes accept a
 * name or an id; tasks, schedules, and messages take their id only.
 */

export type AgentState =
	| "unknown"
	| "launching"
	| "idle"
	| "working"
	| "blocked"
	| "done"
	| "dead";

export interface Agent {
	id: string;
	name: string;
	kind: string;
	machine_id: string;
	pane_id: string | null;
	workdir: string | null;
	worktree: string | null;
	state: AgentState;
	desired_state: string;
	permissions: string;
	adopted: boolean;
	model?: string;
	args: string[];
	created_at: number;
	updated_at: number;
}

export type TaskState =
	| "queued"
	| "assigned"
	| "working"
	| "input-required"
	| "completed"
	| "failed"
	| "canceled"
	| "rejected";

/** Open states, in lifecycle order. The task list route filters one state at a time. */
export const OPEN_TASK_STATES = [
	"queued",
	"assigned",
	"working",
	"input-required",
] as const;
export const TERMINAL_TASK_STATES = [
	"completed",
	"failed",
	"canceled",
	"rejected",
] as const;
export const TASK_STATES = [
	...OPEN_TASK_STATES,
	...TERMINAL_TASK_STATES,
] as const;

export function isOpenTask(state: TaskState): boolean {
	return (OPEN_TASK_STATES as readonly string[]).includes(state);
}

export interface Task {
	id: string;
	owner_id?: string;
	origin: string;
	title: string;
	description?: string;
	state: TaskState;
	priority: number;
	tags: string[];
	attempt_count: number;
	max_attempts: number;
	lease_expires_at?: number;
	lease_s: number;
	target_agent_id?: string;
	not_before?: number;
	schedule_id?: string;
	occurrence_at?: number;
	pre_run?: string;
	post_run?: string;
	/** `pre`/`post` while a lifecycle prompt is running for this job. */
	hook?: string;
	result?: unknown;
	created_at: number;
	updated_at: number;
}

export interface MessagePart {
	text?: string;
	data?: unknown;
	url?: string;
}

export interface Message {
	id: string;
	task_id?: string;
	from_kind: string;
	/** An agent name for agent senders; `me` for the operator. */
	from_id: string;
	to_kind: string;
	to_id: string;
	kind: string;
	parts: MessagePart[];
	status: string;
	deadline_at?: number;
	responded_at?: number;
	created_at: number;
}

export interface Schedule {
	id: string;
	title: string;
	description?: string;
	tags: string[];
	priority: number;
	lease_s: number;
	max_attempts: number;
	target_agent_id?: string;
	cron: string;
	timezone: string;
	enabled: boolean;
	next_run_at?: number;
	last_run_at?: number;
	pre_run?: string;
	post_run?: string;
	created_at: number;
	updated_at: number;
}

export interface Machine {
	id: string;
	name: string;
	role: string;
	status: string;
	last_seen_at?: number;
}

export interface Health {
	ok: boolean;
	version: string;
	started_at: number;
	db: string;
}

/** One entry of the event log (`/v1/events`). The kind names the event type. */
export interface TowerEvent {
	seq: number;
	ts: number;
	kind: string;
	subject_type?: string;
	subject_id?: string;
	payload: Record<string, unknown>;
}

export interface TaskDetail {
	task: Task;
	trail: TowerEvent[];
	messages: Message[];
}

export interface ScheduleDetail {
	schedule: Schedule;
	jobs: Task[];
}
