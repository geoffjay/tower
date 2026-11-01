/** TUI rendering of a `FleetView`: the fleet card (welcome, /fleet, tower_status) and the live strip widget. */

import type { Component, Theme, ThemeColor } from "@oh-my-pi/pi-tui";
import { truncateToWidth, visibleWidth } from "@oh-my-pi/pi-tui";
import { relative } from "./format";
import { countBy, type FleetView, stackOf } from "./snapshot";
import { oneLine, trunc } from "./text";
import { clock } from "./time";

/** A component whose rows are rebuilt from live state when the width changes or `invalidate()` runs. */
export class Lines implements Component {
	#build: (width: number) => string[];
	#width = -1;
	#rows: string[] = [];

	constructor(build: (width: number) => string[]) {
		this.#build = build;
	}

	render(width: number): readonly string[] {
		if (width !== this.#width) {
			this.#rows = this.#build(width).map((row) => truncateToWidth(row, width));
			this.#width = width;
		}
		return this.#rows;
	}

	invalidate(): void {
		this.#width = -1;
	}
}

const GLYPH: Record<string, string> = {
	working: "●",
	blocked: "◉",
	idle: "○",
	done: "○",
	launching: "◌",
	dead: "✕",
	unknown: "?",
};

const COLOR: Record<string, ThemeColor> = {
	working: "accent",
	blocked: "warning",
	idle: "muted",
	done: "muted",
	launching: "accent",
	dead: "error",
	unknown: "dim",
};

export function stateGlyph(theme: Theme, state: string): string {
	return theme.fg(COLOR[state] ?? "dim", GLYPH[state] ?? "?");
}

const LEVEL: Record<string, { glyph: string; color: ThemeColor }> = {
	error: { glyph: "✕", color: "error" },
	warn: { glyph: "⚠", color: "warning" },
	info: { glyph: "·", color: "muted" },
};

/** Pack `chips` into rows of at most `width` cells; continuation rows start with `indent`. */
function pack(
	lead: string,
	chips: string[],
	width: number,
	indent: string,
): string[] {
	const rows: string[] = [];
	let row = lead;
	for (const chip of chips) {
		const candidate =
			row === lead || row === indent ? `${row}${chip}` : `${row}  ${chip}`;
		if (visibleWidth(candidate) > width && row !== lead && row !== indent) {
			rows.push(row);
			row = `${indent}${chip}`;
		} else {
			row = candidate;
		}
	}
	rows.push(row);
	return rows;
}

/** Agents grouped by stack, one glyph-and-role chip per agent. */
function stackRows(view: FleetView, theme: Theme, width: number): string[] {
	const groups = new Map<string, FleetView["agents"]>();
	for (const agent of view.agents) {
		const stack = stackOf(agent.name) ?? "";
		groups.set(stack, [...(groups.get(stack) ?? []), agent]);
	}
	const label = Math.min(
		12,
		Math.max(...[...groups.keys()].map((stack) => stack.length), 1),
	);
	const rows: string[] = [];
	for (const [stack, agents] of [...groups].sort(([a], [b]) =>
		a.localeCompare(b),
	)) {
		const lead = `${theme.fg("accent", (stack || "·").padEnd(label))}  `;
		const chips = agents.map((agent) => {
			const role = stack ? agent.name.slice(stack.length + 1) : agent.name;
			return `${stateGlyph(theme, agent.state)} ${agent.state === "dead" ? theme.fg("error", role) : role}`;
		});
		rows.push(...pack(lead, chips, width, " ".repeat(label + 2)));
	}
	return rows;
}

function headline(view: FleetView, theme: Theme): string {
	const states = countBy(view.agents, (agent) => agent.state);
	const parts = [`${theme.bold(String(view.agents.length))} agents`];
	for (const state of ["working", "blocked", "launching", "dead"]) {
		if (states[state])
			parts.push(`${stateGlyph(theme, state)} ${states[state]} ${state}`);
	}
	const idle = (states.idle ?? 0) + (states.done ?? 0);
	if (idle) parts.push(`${stateGlyph(theme, "idle")} ${idle} idle`);
	const inbox = view.inbox.length
		? theme.fg("warning", `inbox ${view.inbox.length}`)
		: theme.fg("muted", "inbox 0");
	const jobs = countBy(view.jobs, (job) => job.state);
	const queue = ["queued", "assigned", "working", "input-required"]
		.filter((state) => jobs[state])
		.map((state) => `${jobs[state]} ${state}`)
		.join(" · ");
	return `${parts.join("  ")}   ${inbox}   ${theme.fg("muted", "jobs")} ${queue || "none open"}`;
}

/** The fleet card: header, attention, agents by stack, open jobs, upcoming schedules, last 24h. */
export function fleetCard(
	view: FleetView,
	theme: Theme,
	width: number,
): string[] {
	const now = view.at;
	const title = `${theme.bold(theme.fg("accent", "tower"))} ${theme.fg("muted", `${view.version ?? ""} · ${view.url} · ${clock(now, now)}`)}`;
	if (view.error) {
		return [title, theme.fg("error", `✕ unreachable: ${view.error}`)];
	}
	const rows = [
		view.stack
			? `${title} ${theme.fg("accent", `stack ${view.stack}`)}`
			: title,
		headline(view, theme),
		"",
	];

	if (view.attention.length) {
		rows.push(theme.fg("warning", "needs you"));
		for (const item of view.attention.slice(0, 6)) {
			const level = LEVEL[item.level] ?? LEVEL.info;
			rows.push(` ${theme.fg(level.color, level.glyph)} ${item.text}`);
		}
		if (view.attention.length > 6)
			rows.push(theme.fg("muted", ` … ${view.attention.length - 6} more`));
	} else {
		rows.push(theme.fg("success", "✓ nothing needs you"));
	}

	if (view.agents.length) rows.push("", ...stackRows(view, theme, width));

	if (view.jobs.length) {
		rows.push("");
		for (const job of view.jobs.slice(0, 5)) {
			const who = job.owner ?? (job.target ? `→${job.target}` : "unassigned");
			rows.push(
				`${theme.fg("muted", "job")}  ${theme.fg("dim", job.id)}  ${job.state.padEnd(14)} ${who.padEnd(16)} ${trunc(oneLine(job.title), 80)}`,
			);
		}
		if (view.jobs.length > 5)
			rows.push(
				theme.fg("muted", `     … ${view.jobs.length - 5} more open jobs`),
			);
	}

	const upcoming = view.schedules
		.filter((schedule) => schedule.next)
		.slice(0, 3);
	if (upcoming.length) {
		const next = upcoming.map(
			(schedule) =>
				`${clock(schedule.next ?? 0, now)} ${trunc(schedule.title, 40)} → ${schedule.target ?? "queue"}`,
		);
		rows.push(
			`${theme.fg("muted", "next")} ${next.join(theme.fg("muted", "  ·  "))}`,
		);
	}
	const { hours, completed, failed, canceled } = view.recent;
	const failedText = failed
		? theme.fg("error", `${failed} failed`)
		: `${failed} failed`;
	rows.push(
		`${theme.fg("muted", `${hours}h`)}  ${completed} completed · ${failedText} · ${canceled} canceled`,
	);
	rows.push(
		"",
		theme.fg("dim", "/fleet · /inbox · /peek <agent> — or ask in plain words"),
	);
	return rows;
}

export type LinkState = "connecting" | "live" | "offline";

/** The live strip above the editor: one status row, plus the top attention item when there is one. */
export function fleetStrip(
	view: FleetView | undefined,
	link: LinkState,
	theme: Theme,
	width: number,
): string[] {
	const brand = theme.bold(theme.fg("accent", "tower"));
	if (link === "offline" || view?.error) {
		const reason = view?.error ?? "unreachable";
		return [
			`${brand} ${theme.fg("error", "✕ offline")} ${theme.fg("muted", trunc(oneLine(reason), 160))}`,
		];
	}
	if (!view) return [`${brand} ${theme.fg("dim", "◌ connecting…")}`];

	const sep = theme.fg("dim", " │ ");
	const live =
		link === "live"
			? theme.fg("success", "● live")
			: theme.fg("dim", "◌ reconnecting");
	const groups = new Map<string, string[]>();
	for (const agent of view.agents) {
		const stack = stackOf(agent.name) ?? agent.name;
		groups.set(stack, [
			...(groups.get(stack) ?? []),
			stateGlyph(theme, agent.state),
		]);
	}
	const stacks = [...groups]
		.sort(([a], [b]) => a.localeCompare(b))
		.map(([stack, glyphs]) => `${stack} ${glyphs.join("")}`);
	const inbox = view.inbox.length
		? theme.fg("warning", `inbox ${view.inbox.length}`)
		: theme.fg("muted", "inbox 0");
	const jobs = countBy(view.jobs, (job) => job.state);
	const queue = ["queued", "assigned", "working", "input-required"]
		.filter((state) => jobs[state])
		.map(
			(state) =>
				`${jobs[state]} ${state === "input-required" ? "waiting" : state}`,
		)
		.join(" · ");
	const tail = [inbox, `${theme.fg("muted", "jobs")} ${queue || "0"}`];
	let first = [brand, live, ...stacks, ...tail].join(sep);
	if (visibleWidth(first) > width) {
		const states = countBy(view.agents, (agent) => agent.state);
		const compact = ["working", "blocked", "idle", "dead"]
			.filter((state) => states[state])
			.map((state) => `${stateGlyph(theme, state)}${states[state]}`)
			.join(" ");
		first = [
			brand,
			live,
			`${view.agents.length} agents ${compact}`,
			...tail,
		].join(sep);
	}
	const rows = [first];
	const top = view.inbox[0];
	if (top) {
		rows.push(
			`${theme.fg("warning", "⚠")} ${top.from} ${top.kind} ${relative(top.at)}: ${theme.fg("muted", `"${trunc(oneLine(top.text), 120)}"`)} ${theme.fg("dim", "— /inbox")}`,
		);
	} else {
		const urgent = view.attention.find((item) => item.level === "error");
		if (urgent) rows.push(`${theme.fg("error", "✕")} ${urgent.text}`);
	}
	return rows;
}
