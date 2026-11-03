// Event formatting for the events panel: a subject and a one-line detail
// per event in operator words (agent names, job and schedule titles —
// never bare ids), and the facts the detail section lists for one event.
// Mirrors the TUI's events view (subject + detail columns).
//
// Events carry ids; `Names` turns them into names. It learns from the
// events themselves (task.created titles, agent.created names, …) and from
// `tower ps`, `tower task list` and `tower schedule list`, so agents and
// schedules removed since still resolve when the log mentions them.

import { tower } from "./tower.js";

const REFRESH_MS = 60_000; // full name refresh
const MISS_MS = 10_000; // an unknown id refreshes at most this often

/** Ids → names for agents, jobs, schedules and messages. */
export class Names {
	constructor() {
		this.agents = new Map(); // id → name
		this.tasks = new Map(); // id → title
		this.schedules = new Map(); // id → title
		this.messages = new Map(); // id → message.created payload
		this.unknown = new Set(); // ids no list had yet: refresh soon
		this.tried = new Set(); // ids a refresh already missed: never retry
		this.loadedAt = 0;
		this.loading = false;
	}

	/** Re-read every list when stale, or soon after a new unknown id. */
	async refresh() {
		const age = Date.now() - this.loadedAt;
		const due = age >= REFRESH_MS || (this.unknown.size > 0 && age >= MISS_MS);
		if (this.loading || !due) return;
		this.loading = true;
		const asked = [...this.unknown];
		this.unknown.clear();
		try {
			const [ps, tasks, schedules] = await Promise.all([
				tower("ps").catch(() => null),
				tower("task", "list").catch(() => null),
				tower("schedule", "list").catch(() => null),
			]);
			for (const a of ps?.agents ?? []) this.agents.set(a.id, a.name);
			for (const t of tasks?.tasks ?? []) this.tasks.set(t.id, t.title);
			for (const s of schedules?.schedules ?? [])
				this.schedules.set(s.id, s.title);
		} finally {
			for (const id of asked) this.tried.add(id);
			this.loadedAt = Date.now();
			this.loading = false;
		}
	}

	/** Pick up the names an event itself carries. */
	learn(e) {
		const p = e.payload ?? {};
		const id = e.subject_id;
		if ((e.kind === "agent.created" || e.kind === "agent.removed") && p.name)
			this.agents.set(id, p.name);
		if (e.kind === "task.created" && p.title) {
			this.tasks.set(id, p.title);
			// A schedule's jobs carry its title: the only name left for a
			// schedule removed since.
			if (p.schedule_id && !this.schedules.has(p.schedule_id))
				this.schedules.set(p.schedule_id, p.title);
		}
		if (e.kind === "schedule.created" && p.title) this.schedules.set(id, p.title);
		if (e.kind === "message.created") this.messages.set(id, p);
	}

	/** An agent's name, or "agent …<tail>". */
	agent(id) {
		if (!id) return "?";
		const name = this.agents.get(id);
		if (name) return name;
		if (!isUlid(id)) return id; // some payloads carry the name already
		this.miss(id);
		return `agent ${tail(id)}`;
	}

	/** A job's quoted title, or "…<tail>"; callers say "job". */
	task(id) {
		if (!id) return "?";
		const title = this.tasks.get(id);
		if (title) return `"${trunc(title, 48)}"`;
		this.miss(id);
		return tail(id);
	}

	/** A schedule's quoted title, or "…<tail>"; callers say "schedule". */
	schedule(id) {
		if (!id) return "?";
		const title = this.schedules.get(id);
		if (title) return `"${trunc(title, 48)}"`;
		this.miss(id);
		return tail(id);
	}

	/** A message's "from → to", or "message …<tail>". */
	message(id) {
		const m = this.messages.get(id);
		return m ? `${this.who(m.from)} → ${this.who(m.to)}` : `message ${tail(id)}`;
	}

	/** A `by`/`from`/`to` reference: "schedule:<id>", an agent, operator. */
	who(ref) {
		const id = typeof ref === "object" && ref ? ref.id : ref;
		if (!id) return "?";
		if (id.startsWith("schedule:")) return `schedule ${this.schedule(id.slice(9))}`;
		if (typeof ref === "object" && ref.kind === "human") return id;
		return this.agent(id);
	}

	miss(id) {
		if (!this.tried.has(id)) this.unknown.add(id);
	}
}

/** The subject column: who or what the event is about. */
export function subject(e, n) {
	const p = e.payload ?? {};
	switch (e.subject_type) {
		case "agent":
			return n.agent(e.subject_id);
		case "task":
			return noun("job", n.task(e.subject_id));
		case "schedule":
			return noun("schedule", n.schedule(e.subject_id));
		case "message":
			if (p.from || p.to) return `${n.who(p.from)} → ${n.who(p.to)}`;
			return n.message(e.subject_id);
		case "":
		case undefined:
		case null:
			return e.kind.split(".")[0];
		default:
			return `${e.subject_type} ${tail(e.subject_id)}`;
	}
}

/** A title as is; an unresolved "…<tail>" with its noun. */
function noun(word, name) {
	return name.startsWith("…") ? `${word} ${name}` : name;
}

/** The detail column: what happened, in operator words. */
export function detail(e, n) {
	const p = e.payload ?? {};
	const parts = (...xs) => xs.filter(Boolean).join(" · ");
	switch (e.kind) {
		case "server.started":
			return parts(`started ${p.version ?? ""}`.trim(), p.pid && `pid ${p.pid}`);
		case "agent.created":
			return parts(
				p.adopted ? "adopted" : "spawned",
				p.kind,
				p.model,
				p.pane && `pane ${p.pane}`,
			);
		case "agent.removed":
			return "removed";
		case "agent.state":
			return `${p.from ?? "?"} → ${p.to ?? "?"}`;
		case "agent.output":
			return `screen · ${lineCount(p.text)} lines`;
		case "task.created":
			return parts(
				"created",
				p.priority ? `priority ${p.priority}` : null,
				(p.tags ?? []).filter(Boolean).map((x) => `#${x}`).join(" "),
				p.schedule_id && `from schedule ${n.schedule(p.schedule_id)}`,
			);
		case "task.reserved":
			return parts(
				`reserved for ${n.agent(p.target)}`,
				p.not_before && `not before ${stamp(p.not_before)}`,
			);
		case "task.assigned":
			return parts(
				`assigned to ${n.agent(p.owner_id)}`,
				p.by && `by ${n.who(p.by)}`,
			);
		case "task.leased_out":
			return parts(
				`lease missed by ${n.agent(p.prior_owner)}`,
				p.attempt_count && `attempt ${p.attempt_count}/${p.max_attempts ?? "?"}`,
				p.state && `→ ${p.state}`,
			);
		case "task.status":
			return parts(`→ ${p.state ?? "?"}`, p.reason, p.owner_id && n.agent(p.owner_id));
		case "task.completed":
			return parts(
				p.owner_id ? `completed by ${n.agent(p.owner_id)}` : "completed",
				trunc(resultSummary(p.result), 120),
			);
		case "task.failed":
			return parts(
				"failed",
				p.owner_id && n.agent(p.owner_id),
				trunc(p.error ?? resultSummary(p.result), 120),
			);
		case "schedule.created":
			return parts("created", p.cron, p.timezone);
		case "schedule.fired":
			return parts(
				p.manual ? "run now" : "fired",
				p.task_id && `→ job ${n.task(p.task_id)}`,
				p.missed ? `${p.missed} missed` : null,
			);
		case "schedule.skipped":
			return parts("skipped", p.reason);
		case "schedule.paused":
			return parts("paused", p.reason);
		case "schedule.resumed":
			return "resumed";
		case "schedule.removed":
			return "removed";
		case "message.created":
			return parts(
				p.kind,
				p.text && `"${trunc(oneLine(p.text), 100)}"`,
				p.task_id && `job ${n.task(p.task_id)}`,
				p.status,
			);
		case "message.status":
			return parts(
				p.status,
				p.approve === true ? "approved" : p.approve === false ? "denied" : null,
				p.response && `"${trunc(oneLine(String(p.response)), 100)}"`,
			);
		case "approval.expired":
			return parts(`${p.kind ?? "approval"} expired`, p.notified && "agent told");
		default:
			return trunc(JSON.stringify(p), 120);
	}
}

/** The detail section's facts for one event: [label, value] rows. Payload
 * scalars only, ids resolved, timestamps local; long text and nested
 * objects go to `body()`. */
export function facts(e, n) {
	const rows = [
		["when", `${stamp(e.ts, true)} · seq ${e.seq}`],
		["event", e.kind],
	];
	if (e.subject_type)
		rows.push(["subject", `${subject(e, n)} · ${e.subject_type} ${e.subject_id}`]);
	for (const [k, v] of Object.entries(e.payload ?? {})) {
		if (v === null || v === undefined || isBodyValue(v)) continue;
		rows.push([k.replaceAll("_", " "), factValue(k, v, n)]);
	}
	return rows;
}

/** The detail section's long body, or null: the screen of an
 * agent.output, else the payload's nested and long values as JSON. */
export function body(e) {
	const p = e.payload ?? {};
	if (e.kind === "agent.output") return { language: "txt", text: p.text ?? "" };
	const long = Object.fromEntries(
		Object.entries(p).filter(([, v]) => v !== null && isBodyValue(v)),
	);
	if (Object.keys(long).length === 0) return null;
	return { language: "json", text: JSON.stringify(long, null, 2) };
}

function isBodyValue(v) {
	return typeof v === "object" || (typeof v === "string" && (v.length > 120 || v.includes("\n")));
}

function factValue(key, v, n) {
	if (typeof v === "number" && v > 1e12 && /(_at|not_before)$/.test(key))
		return stamp(v, true);
	if (typeof v !== "string") return String(v);
	if (/^(owner_id|agent_id|prior_owner|target)$/.test(key)) return named(n.agent(v), v);
	if (key === "task_id") return named(n.task(v), v);
	if (key === "schedule_id") return named(n.schedule(v), v);
	if (key === "by") return named(n.who(v), v);
	return v;
}

/** "name (id)", or just the id when no name resolved. */
function named(name, id) {
	return name === id || name.includes(tail(id)) ? id : `${name} (${id})`;
}

/** A task result's one-line gist: its summary, else its first string. */
function resultSummary(result) {
	if (!result || typeof result !== "object") return result ? String(result) : "";
	const s = result.summary ?? result.message ?? result.error;
	if (typeof s === "string") return oneLine(s);
	const first = Object.values(result).find((v) => typeof v === "string");
	return first ? oneLine(first) : "";
}

/** Local time: "HH:MM" today, "Mon DD HH:MM" otherwise; `full` adds
 * the year and seconds. */
export function stamp(ts, full = false) {
	const d = new Date(ts);
	const hm = d.toTimeString().slice(0, full ? 8 : 5);
	if (full) {
		const ymd = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
		return `${ymd} ${hm}`;
	}
	const now = new Date();
	if (d.toDateString() === now.toDateString()) return hm;
	return `${MONTHS[d.getMonth()]} ${d.getDate()} ${hm}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function pad(x) {
	return String(x).padStart(2, "0");
}

function isUlid(s) {
	return typeof s === "string" && /^[0-9A-HJKMNP-TV-Z]{26}$/.test(s);
}

/** "…" + the last 6 characters of an id, as the TUI shows them. */
function tail(id) {
	const s = String(id ?? "");
	return s.length > 8 ? `…${s.slice(-6)}` : s;
}

function trunc(s, max) {
	const t = String(s ?? "");
	return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

function oneLine(s) {
	return String(s).replace(/\s+/g, " ").trim();
}

function lineCount(text) {
	return text ? String(text).split("\n").length : 0;
}
