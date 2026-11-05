// The workflows panel's logic, apart from drawing: the top-down graph a
// definition draws as, a run's steps laid over it, the create form's
// TOML, and the formatters. No herdr imports, so it runs outside the app.

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

const pad = (n) => String(n).padStart(2, "0");

/** "14:05" today, else "Nov 3 14:05"; none -> "-". */
export function stamp(ts) {
	if (!ts) return "-";
	const d = new Date(ts);
	const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
	if (d.toDateString() === new Date().toDateString()) return hm;
	return `${MONTHS[d.getMonth()]} ${d.getDate()} ${hm}`;
}

/** A node's agent: a name, or `builder | designer (by triage)` for an
 * agent picked by an earlier node's outcome. */
export function agentText(agent) {
	if (typeof agent === "string") return agent;
	if (agent && typeof agent === "object") {
		const names = [...new Set(Object.values(agent.map ?? {}))];
		return `${names.join(" | ")} (by ${agent.from})`;
	}
	return "?";
}

/** One trigger as words: `schedule 0 8 * * MON-FRI`, `daily 09:00`,
 * `event agent.state`, `manual`. */
export function triggerText(t) {
	if (t?.kind === "schedule") {
		const when = t.cron ?? (t.daily ? `daily ${t.daily}` : "");
		return `schedule ${when}${t.timezone ? ` ${t.timezone}` : ""}`.trim();
	}
	if (t?.kind === "event") return `event ${t.type ?? "?"}`;
	return t?.kind ?? "?";
}

/** Every trigger, comma-joined; none means manual starts only. */
export function triggersText(triggers) {
	return triggers?.length ? triggers.map(triggerText).join(", ") : "manual";
}

/** What ended a run: its error, else `→ node outcome: summary`. */
export function runGist(run) {
	if (!run) return "";
	if (run.error) return run.error;
	const r = run.result;
	if (!r?.node) return "";
	const head = `→ ${r.node}${r.outcome ? ` ${r.outcome}` : ""}`;
	const sum = typeof r.summary === "string" ? r.summary.split("\n")[0] : "";
	return sum ? `${head}: ${sum}` : head;
}

/** How a run started: `manual (me)`, `schedule`, `event agent.state`. */
export function triggerOf(run) {
	const t = run?.trigger ?? {};
	if (t.kind === "event") return `event ${t.event_type ?? ""}`.trim();
	if (t.kind === "manual" && t.by) return `manual (${t.by})`;
	return t.kind ?? "?";
}

// --- the graph ------------------------------------------------------------

/** An edge's condition as a label; `completed` (the default) is none. */
function onLabel(e) {
	return e.on && e.on !== "completed" ? `on ${e.on}` : null;
}

/**
 * A definition as rows of a top-down list. A chain of single edges stays at
 * one depth; where a node fans out (several edges, or any conditional one
 * besides a lone forward edge) each target is a branch one level deeper,
 * labeled with its edge's condition. An edge back to a node already above
 * it on the path is a loop row (`↺ work`), not a second copy. A node that
 * several edges lead into (a join) shows once, after its last incoming
 * branch, at the depth where those branches split, listing what it waits
 * for. Several start nodes hang under one `start` row.
 *
 * Rows: { kind: "start" | "node" | "loop", id, node, depth, via, after },
 * plus the rails: `guides[level]` (a line passes through at that level)
 * and `last` (no later row at this depth before a shallower one).
 */
export function graphRows(def) {
	const nodes = def?.nodes ?? [];
	const byId = new Map(nodes.map((n) => [n.id, n]));
	const out = new Map(nodes.map((n) => [n.id, []]));
	for (const e of def?.edges ?? []) {
		if (byId.has(e.from) && byId.has(e.to)) out.get(e.from).push(e);
	}
	const entered = new Set([...out.values()].flat().map((e) => e.to));
	const starts = nodes.filter((n) => !entered.has(n.id)).map((n) => n.id);

	// Back edges (loops): an edge to a node still on the depth-first path.
	const back = new Set();
	const seen = new Map(); // id -> 1 on the path, 2 finished
	const dfs = (id) => {
		seen.set(id, 1);
		for (const e of out.get(id)) {
			const s = seen.get(e.to);
			if (s === 1) back.add(e);
			else if (!s) dfs(e.to);
		}
		seen.set(id, 2);
	};
	for (const id of [...starts, ...nodes.map((n) => n.id)]) {
		if (!seen.has(id)) dfs(id);
	}
	const preds = new Map(nodes.map((n) => [n.id, []]));
	for (const es of out.values()) {
		for (const e of es) if (!back.has(e)) preds.get(e.to).push(e);
	}

	const rows = [];
	const emitted = new Set();
	const waiting = []; // joins reached before all their branches were
	const ready = (id) => preds.get(id).every((e) => emitted.has(e.from));

	const walk = (id, depth, via) => {
		const into = preds.get(id);
		rows.push({
			kind: "node",
			id,
			node: byId.get(id),
			depth,
			via,
			after:
				into.length > 1
					? into.map((e) => (onLabel(e) ? `${e.from} (${e.on})` : e.from))
					: null,
		});
		emitted.add(id);
		const outs = out.get(id);
		const fwd = outs.filter((e) => !back.has(e));
		const loops = outs.filter((e) => back.has(e));
		if (fwd.length === 1 && loops.length === 0) {
			follow(fwd[0], depth, onLabel(fwd[0]));
			return;
		}
		for (const e of fwd) follow(e, depth + 1, onLabel(e) ?? "then");
		for (const e of loops) {
			rows.push({
				kind: "loop",
				id: e.to,
				node: byId.get(e.to),
				depth: depth + 1,
				via: onLabel(e) ?? "then",
				after: null,
			});
		}
		flush(depth);
	};
	const follow = (e, depth, via) => {
		if (emitted.has(e.to)) return;
		if (preds.get(e.to).length > 1) {
			if (!waiting.includes(e.to)) waiting.push(e.to);
			return;
		}
		walk(e.to, depth, via);
	};
	const flush = (depth) => {
		for (;;) {
			const i = waiting.findIndex((id) => !emitted.has(id) && ready(id));
			if (i < 0) return;
			walk(waiting.splice(i, 1)[0], depth, null);
		}
	};

	if (starts.length > 1) {
		rows.push({ kind: "start", id: null, node: null, depth: 0, via: null });
		for (const id of starts) walk(id, 1, null);
	} else if (starts.length === 1) {
		walk(starts[0], 0, null);
	}
	flush(0);
	// Defensive: nodes no start reaches, and joins that never became ready.
	for (const n of nodes) {
		if (!emitted.has(n.id)) walk(n.id, 0, "not reached from a start");
	}
	for (const id of waiting) if (!emitted.has(id)) walk(id, 0, null);

	// Rails, from the bottom up: `more[level]` = a later row sits at that
	// level before any shallower one.
	const more = [];
	for (let i = rows.length - 1; i >= 0; i--) {
		const r = rows[i];
		r.guides = Array.from({ length: r.depth }, (_, l) => Boolean(more[l]));
		r.last = !more[r.depth];
		r.parent = rows[i + 1]?.depth > r.depth;
		more[r.depth] = true;
		more.length = r.depth + 1;
	}
	return rows;
}

// --- a run over the graph --------------------------------------------------

/** A run's steps per node, oldest visit first. */
export function stepsByNode(steps) {
	const m = new Map();
	for (const s of steps ?? []) {
		if (!m.has(s.node_id)) m.set(s.node_id, []);
		m.get(s.node_id).push(s);
	}
	for (const list of m.values()) {
		list.sort((a, b) => a.visit - b.visit || a.created_at - b.created_at);
	}
	return m;
}

/** A step state's mark: ok / bad / run / wait / skip; no step -> todo. */
export function stepTone(state) {
	switch (state) {
		case "completed":
			return "ok";
		case "failed":
			return "bad";
		case "running":
			return "run";
		case "pending":
		case "ready":
			return "wait";
		case "canceled":
			return "skip";
		default:
			return "todo";
	}
}

/** The latest step of a node, or null when the run never reached it. */
export function latestStep(list) {
	return list?.length ? list[list.length - 1] : null;
}

// --- the create form -------------------------------------------------------

/** Letters, digits, `-` and `_`, starting alphanumeric (the server's rule
 * for names, node ids, outcomes and input keys). */
export function isIdent(s) {
	return /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(s);
}

/** "a, b  c" -> ["a", "b", "c"], blanks and repeats dropped. */
export function splitList(s) {
	return [
		...new Set(
			String(s ?? "")
				.split(/[,\s]+/)
				.map((x) => x.trim())
				.filter(Boolean),
		),
	];
}

/** Where each of a step's endings goes: its declared outcomes, else
 * `completed`. */
export function routeKeys(step) {
	const outcomes = splitList(step.outcomes);
	return outcomes.length ? outcomes : ["completed"];
}

/** A route's target: a step id, "end", or "next" (the following step; the
 * last step's "next" ends the run). Returns a step id or null. */
export function routeTarget(steps, i, target) {
	if (target === "end") return null;
	if (target === "next" || target == null) return steps[i + 1]?.id || null;
	return target;
}

/** The first thing wrong with the create form, or null. */
export function draftError(draft) {
	if (!isIdent(draft.name)) {
		return "name: letters, digits, '-' and '_', starting with a letter or digit";
	}
	for (const k of splitList(draft.inputs)) {
		if (!isIdent(k)) return `input ${k}: letters, digits, '-' and '_'`;
	}
	if (!draft.steps.length) return "add a step";
	const ids = new Set();
	for (const [i, s] of draft.steps.entries()) {
		const at = `step ${i + 1}`;
		if (!isIdent(s.id)) return `${at}: id uses letters, digits, '-' and '_'`;
		if (ids.has(s.id)) return `${at}: id ${s.id} is used twice`;
		ids.add(s.id);
		if (!s.agent) return `${at} (${s.id}): pick an agent`;
		if (!s.prompt.trim()) return `${at} (${s.id}): write a prompt`;
	}
	return null;
}

/** A TOML basic string. JSON's escapes (\" \\ \n \t \uXXXX) are all
 * TOML's too. */
function str(s) {
	return JSON.stringify(String(s));
}

/** A multi-line TOML basic string, kept readable. */
function multi(s) {
	const body = String(s)
		.replace(/\\/g, "\\\\")
		.replace(/"""/g, '""\\"')
		.replace(/\r/g, "\\r");
	return `"""\n${body}\n"""`;
}

/**
 * The create form as a definition: `{ name, description, inputs (a list),
 * steps: [{ id, agent, prompt, outcomes (a list), routes: { key: target } }] }`.
 * Every ending of a step becomes an edge unless it goes to "end". A route
 * to the same or an earlier step is a loop; its target gets
 * `max_visits = 3`, the guard every loop needs.
 */
export function draftToml(draft) {
	const steps = draft.steps;
	const edges = [];
	const guarded = new Set();
	for (const [i, s] of steps.entries()) {
		for (const key of routeKeys(s)) {
			const to = routeTarget(steps, i, s.routes?.[key]);
			if (!to) continue;
			edges.push({ from: s.id, to, on: key === "completed" ? null : key });
			const j = steps.findIndex((x) => x.id === to);
			if (j >= 0 && j <= i) guarded.add(to);
		}
	}
	const lines = [`name = ${str(draft.name)}`];
	if (draft.description?.trim()) {
		lines.push(`description = ${str(draft.description.trim())}`);
	}
	const inputs = splitList(draft.inputs);
	if (inputs.length) {
		lines.push("", "[inputs]");
		for (const k of inputs) lines.push(`${k} = { required = true }`);
	}
	for (const s of steps) {
		lines.push("", "[[nodes]]", `id = ${str(s.id)}`, `agent = ${str(s.agent)}`);
		const outcomes = splitList(s.outcomes);
		if (outcomes.length) {
			lines.push(`outcomes = [${outcomes.map(str).join(", ")}]`);
		}
		if (guarded.has(s.id)) lines.push("max_visits = 3");
		lines.push(`prompt = ${multi(s.prompt.trim())}`);
	}
	for (const e of edges) {
		lines.push("", "[[edges]]", `from = ${str(e.from)}`, `to = ${str(e.to)}`);
		if (e.on) lines.push(`on = ${str(e.on)}`);
	}
	return `${lines.join("\n")}\n`;
}

/** A blank definition for the TOML editor's first open. */
export function starterToml(agent) {
	return `name = "my-workflow"
description = "What this workflow does"

[inputs]
issue = { required = true, description = "Issue id, e.g. ENG-123" }

[[nodes]]
id = "triage"
agent = ${str(agent ?? "agent-name")}
outcomes = ["build", "skip"]
prompt = "Read issue {{ input.issue }}. Build it, or skip it?"

[[nodes]]
id = "build"
agent = ${str(agent ?? "agent-name")}
prompt = "Do the work for issue {{ input.issue }}."

[[edges]]
from = "triage"
to = "build"
on = "build"
`;
}

/** `--input` args for `tower workflow run`: one `k=v` per filled field. */
export function inputArgs(values) {
	return Object.entries(values)
		.filter(([, v]) => v.trim() !== "")
		.flatMap(([k, v]) => ["--input", `${k}=${v.trim()}`]);
}

/** The declared inputs that are required, have no default and are blank. */
export function missingInputs(def, values) {
	return Object.entries(def?.inputs ?? {})
		.filter(
			([k, spec]) =>
				spec.required && spec.default == null && !(values[k] ?? "").trim(),
		)
		.map(([k]) => k);
}
