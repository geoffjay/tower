// Job and schedule detail formatting for the jobs panel: the trail in
// operator words (the TUI's `trail_line`, `tower task show`), names for
// the ids events and messages carry, and the edit form's field rules and
// CLI arguments (D§5.2.3: a queued job takes every field, a delivered one
// only post-run).

export const OPEN = ["queued", "assigned", "working", "input-required"];
export const CLOSED = ["completed", "failed", "canceled", "rejected"];

const MONTHS = [
	"Jan",
	"Feb",
	"Mar",
	"Apr",
	"May",
	"Jun",
	"Jul",
	"Aug",
	"Sep",
	"Oct",
	"Nov",
	"Dec",
];

function pad(x) {
	return String(x).padStart(2, "0");
}

/** Local time: "HH:MM" today, "Mon DD HH:MM" otherwise; "-" when unset. */
export function stamp(ts) {
	if (!ts) return "-";
	const d = new Date(ts);
	const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
	if (d.toDateString() === new Date().toDateString()) return hm;
	return `${MONTHS[d.getMonth()]} ${d.getDate()} ${hm}`;
}

/** "YYYY-MM-DD HH:MM" local: the form `tower task edit --at` reads back. */
export function atValue(ts) {
	if (!ts) return "";
	const d = new Date(ts);
	return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** A name for an id a job, event or message carries: agents by name,
 * `schedule:<id>` by the schedule's title, `me` as you. */
export function who(id, names) {
	if (typeof id !== "string" || id === "") return "?";
	if (id === "me") return "you";
	if (id.startsWith("schedule:")) {
		const title = names.schedules[id.slice(9)];
		return title ? `schedule "${title}"` : "a schedule";
	}
	return names.agents[id] ?? id;
}

/** One trail event, in operator words. */
export function trailLine(e, names) {
	const p = e.payload ?? {};
	const n = (id) => who(id, names);
	switch (e.kind) {
		case "task.created":
			return p.schedule_id
				? `created by ${n(`schedule:${p.schedule_id}`)}`
				: "created";
		case "task.assigned":
			return `assigned to ${n(p.owner_id)} by ${n(p.by)}`;
		case "task.reserved":
			if (!p.target) return "reservation dropped (target removed)";
			return `reserved for ${n(p.target)}${p.not_before ? ` from ${stamp(p.not_before)}` : ""}`;
		case "task.status":
			if (p.released_by) {
				return `released by ${n(p.released_by)}${p.reason ? ` - "${p.reason}"` : ""}`;
			}
			if (p.state === "working") return `working (${n(p.owner_id)})`;
			return `status: ${p.state ?? "?"}`;
		case "task.leased_out":
			return `lease expired (${n(p.prior_owner)}) -> ${p.state ?? "?"}, attempt ${p.attempt_count}/${p.max_attempts}`;
		case "task.completed":
			return `completed by ${n(p.owner_id)}`;
		case "task.failed":
			return `failed: ${resultSummary(p.result) || "no reason given"}`;
		case "task.updated":
			return `edited: ${(p.fields ?? []).map(fieldLabel).join(", ")}`;
		case "task.hook": {
			const phase = p.phase === "post" ? "post-run" : "pre-run";
			if (p.status === "running") return `${phase} prompt sent`;
			if (p.status && p.error) return `${phase} prompt ${p.status}: ${p.error}`;
			return `${phase} prompt ${p.status ?? ""}`.trim();
		}
		default:
			return e.kind;
	}
}

function fieldLabel(f) {
	return (
		{
			description: "prompt",
			pre_run: "pre-run",
			post_run: "post-run",
			not_before: "at",
		}[f] ?? f
	);
}

/** A job result's one-line gist: its summary, message or error, else its
 * first string; a bare string as is. */
export function resultSummary(result) {
	if (result == null) return "";
	if (typeof result !== "object") return String(result);
	const s = result.summary ?? result.message ?? result.error;
	if (typeof s === "string") return s;
	const first = Object.values(result).find((v) => typeof v === "string");
	return first ?? "";
}

/** The result's other fields, as [key, text] pairs (the gist excluded). */
export function resultFields(result) {
	if (result == null || typeof result !== "object") return [];
	const gist = resultSummary(result);
	return Object.entries(result)
		.filter(([, v]) => v !== gist)
		.map(([k, v]) => [k, typeof v === "string" ? v : JSON.stringify(v)]);
}

/** What holds a queued job back, in words. */
export function waitingFor(task, names) {
	if (task.state !== "queued") return null;
	const parts = [];
	if (task.target_agent_id)
		parts.push(`for ${who(task.target_agent_id, names)}`);
	if (task.not_before && task.not_before > Date.now()) {
		parts.push(`from ${stamp(task.not_before)}`);
	}
	return parts.length ? parts.join(" · ") : "the general queue";
}

/** The lifecycle prompt in flight, in words. */
export function hookText(hook) {
	if (hook === "pre") return "pre-run prompt running (lease paused)";
	if (hook === "post") return "post-run prompt running (owner busy)";
	return null;
}

// The edit form. [key, label, multi-line, CLI flag]; `at` is the job's
// reservation hold, cadence the schedule's.
const JOB_FIELDS = [
	["title", "TITLE", false, "--title"],
	["description", "PROMPT", true, "--description"],
	["at", "AT (HH:MM, YYYY-MM-DD HH:MM)", false, "--at"],
	["pre_run", "PRE-RUN", true, "--pre-run"],
	["post_run", "POST-RUN", true, "--post-run"],
];
const SCHEDULE_FIELDS = [
	["title", "TITLE", false, "--title"],
	["description", "PROMPT", true, "--description"],
	["cron", "CRON", false, "--cron"],
	["timezone", "TIMEZONE (IANA)", false, "--tz"],
	["pre_run", "PRE-RUN", true, "--pre-run"],
	["post_run", "POST-RUN", true, "--post-run"],
];

/** The fields an edit form shows for a job or schedule, each with its
 * current value and whether it can change now. Null when nothing can (a
 * closed job). */
export function editFields(kind, obj) {
	if (kind === "schedule") {
		return SCHEDULE_FIELDS.map(([key, label, multi, flag]) => ({
			key,
			label,
			multi,
			flag,
			value: String(obj[key] ?? ""),
			locked: null,
		}));
	}
	if (CLOSED.includes(obj.state)) return null;
	const queued = obj.state === "queued";
	return JOB_FIELDS.filter(
		([key]) => key !== "at" || (queued && obj.target_agent_id),
	).map(([key, label, multi, flag]) => ({
		key,
		label,
		multi,
		flag,
		value: key === "at" ? atValue(obj.not_before) : String(obj[key] ?? ""),
		locked:
			queued || key === "post_run"
				? null
				: "delivered: only post-run can change",
	}));
}

/** `tower task edit` / `tower schedule edit` arguments for the fields that
 * changed (an emptied field clears it); null when nothing changed. */
export function editArgs(kind, id, fields, values) {
	const args = [kind === "schedule" ? "schedule" : "task", "edit", id];
	for (const f of fields) {
		if (f.locked) continue;
		const v = (values[f.key] ?? "").trim();
		if (v === f.value.trim()) continue;
		if (v === "" && ["title", "cron", "timezone"].includes(f.key)) {
			throw new Error(`${f.label.split(" ")[0].toLowerCase()} cannot be empty`);
		}
		// --flag=value: a value starting with "-" stays a value
		args.push(`${f.flag}=${v}`);
	}
	return args.length > 3 ? args : null;
}

/**
 * `tower task create` or `tower schedule create` arguments for the create
 * form. `when`: "now" queues it, "at" reserves it for its owner until a
 * time (`--at` needs an owner), "repeat" makes a schedule (`--daily` or
 * `--cron`, optional `--tz`; an owner gets each job reserved). `tags`
 * (a comma/space list) become `--tag`s: a workflow's event trigger can
 * match them (`tagTriggers`). Values go as `--flag=value`, so a leading
 * "-" stays a value. Throws with what is missing.
 */
export function createArgs(f) {
	const title = f.title.trim();
	if (!title) throw new Error("a job needs a title");
	const repeat = f.when === "repeat";
	const args = [repeat ? "schedule" : "task", "create", title];
	const add = (flag, v) => {
		if (v?.trim()) args.push(`${flag}=${v.trim()}`);
	};
	if (repeat) {
		const { key, value } = cadenceOf(f);
		add(`--${key}`, value);
		add("--tz", f.timezone);
	}
	add("--description", f.description);
	const p = Number.parseInt(f.priority, 10);
	if (Number.isFinite(p) && p !== 0) args.push(`--priority=${p}`);
	if (f.agent) {
		args.push(`--assign=${f.agent}`);
		if (f.when === "at") {
			if (!f.at?.trim()) throw new Error("give a time to hold the job until");
			add("--at", f.at);
		} else if (f.when === "now" && f.when_available) {
			args.push("--when-available");
		}
	} else if (f.when === "at") {
		throw new Error("a job held until a time needs an owner");
	}
	add("--pre-run", f.pre_run);
	add("--post-run", f.post_run);
	for (const tag of splitTags(f.tags)) args.push(`--tag=${tag}`);
	return args;
}

/** "deploy, infra  x" -> ["deploy", "infra", "x"], blanks and repeats dropped. */
export function splitTags(s) {
	return [
		...new Set(
			String(s ?? "")
				.split(/[,\s]+/)
				.filter(Boolean),
		),
	];
}

/** The cadence fields, checked: "--daily"/"--cron" and the value. */
function cadenceOf(f) {
	const value = f.cadence_value?.trim();
	if (!value) {
		throw new Error(
			f.cadence === "cron"
				? "a schedule needs a cron expression"
				: "a schedule needs a time (HH:MM)",
		);
	}
	return { key: f.cadence === "cron" ? "cron" : "daily", value };
}

/** The declared inputs that are required, have no default and are blank. */
export function missingInputs(inputs, values) {
	return Object.entries(inputs ?? {})
		.filter(
			([k, spec]) =>
				spec.required && spec.default == null && !(values[k] ?? "").trim(),
		)
		.map(([k]) => k);
}

/** `tower workflow run <name> --input k=v ...` for the filled inputs. */
export function runArgs(name, inputs, values) {
	if (!name) throw new Error("pick a workflow");
	const missing = missingInputs(inputs, values);
	if (missing.length) throw new Error(`needs ${missing.join(", ")}`);
	return [
		"workflow",
		"run",
		name,
		...Object.entries(values)
			.filter(([, v]) => v.trim() !== "")
			.flatMap(([k, v]) => ["--input", `${k}=${v.trim()}`]),
	];
}

/**
 * A workflow's schedules live in its definition (decision: workflows §5),
 * so a repeating run is a `[[triggers]]` table appended to the TOML
 * source, applied as a new version. The filled inputs become the
 * trigger's `input`. TOML strings use JSON's escapes, which TOML shares.
 */
export function withScheduleTrigger(source, f, values) {
	const { key, value } = cadenceOf(f);
	const lines = ["", "[[triggers]]", 'kind = "schedule"'];
	lines.push(`${key} = ${JSON.stringify(value)}`);
	if (f.timezone?.trim()) {
		lines.push(`timezone = ${JSON.stringify(f.timezone.trim())}`);
	}
	const input = Object.entries(values).filter(([, v]) => v.trim() !== "");
	if (input.length) {
		const pairs = input.map(([k, v]) => `${k} = ${JSON.stringify(v.trim())}`);
		lines.push(`input = { ${pairs.join(", ")} }`);
	}
	return `${source.replace(/\s*$/, "\n")}${lines.join("\n")}\n`;
}

const JOB_EVENTS = {
	"task.created": "queueing",
	"task.completed": "completing",
	"task.failed": "failing",
};

/** The workflows a job with these tags would start, from their event
 * triggers on the job's events whose `match` is only a tag:
 * [{ name, when: "completing" | "failing" | "queueing" }]. */
export function tagTriggers(workflows, tags) {
	const out = [];
	for (const w of workflows ?? []) {
		if (!w.enabled) continue;
		for (const t of w.triggers ?? []) {
			const when = t.kind === "event" && JOB_EVENTS[t.type];
			const match = t.match ?? {};
			const keys = Object.keys(match);
			if (!when || keys.length !== 1 || keys[0] !== "tags") continue;
			if (tags.includes(match.tags)) out.push({ name: w.name, when });
		}
	}
	return out;
}
