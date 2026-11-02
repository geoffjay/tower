/** The fleet overview as plain text: what the model reads from `tower_status` and the welcome card. */

import { countBy, type FleetView } from "./snapshot";
import { table, trunc } from "./text";
import { clock, duration } from "./time";

function counted(
	counts: Record<string, number>,
	order: readonly string[],
): string {
	return order
		.filter((key) => counts[key])
		.map((key) => `${counts[key]} ${key}`)
		.join(", ");
}

export function fleetText(view: FleetView): string {
	if (view.error) return `tower is unreachable at ${view.url}: ${view.error}`;
	const now = view.at;
	const states = counted(
		countBy(view.agents, (agent) => agent.state),
		["working", "blocked", "launching", "idle", "done", "dead", "unknown"],
	);
	const jobs = counted(
		countBy(view.jobs, (job) => job.state),
		["queued", "assigned", "working", "input-required"],
	);
	const paused = view.schedules.filter((schedule) => !schedule.enabled).length;
	const lines = [
		`tower ${view.version ?? "?"} at ${view.url}, ${clock(now, now)}${view.stack ? ` (stack ${view.stack})` : ""}`,
		`agents ${view.agents.length}${states ? ` (${states})` : ""} · inbox ${view.inbox.length} · open jobs ${view.jobs.length}${jobs ? ` (${jobs})` : ""} · schedules ${view.schedules.length}${paused ? ` (${paused} paused)` : ""}`,
		"",
	];

	if (view.attention.length) {
		lines.push("Needs you:");
		for (const item of view.attention)
			lines.push(`- [${item.level}] ${item.text}`);
	} else {
		lines.push("Nothing needs you.");
	}

	if (view.agents.length) {
		lines.push("", "Agents:");
		lines.push(
			table(
				["NAME", "KIND", "STATE", "JOB", "NOTE"],
				view.agents.map((agent) => [
					agent.name,
					agent.kind,
					agent.state,
					agent.job ? `${trunc(agent.job, 48)} [${agent.jobId}]` : "-",
					[
						agent.machine !== "local" ? `@${agent.machine}` : "",
						agent.note ?? "",
					]
						.filter(Boolean)
						.join(" ") || "-",
				]),
			),
		);
	}

	if (view.jobs.length) {
		lines.push("", "Open jobs:");
		lines.push(
			table(
				["ID", "STATE", "OWNER", "PRI", "TRIES", "TITLE"],
				view.jobs.map((job) => {
					let owner = job.owner ?? (job.target ? `→${job.target}` : "-");
					if (job.notBefore && job.notBefore > now)
						owner += ` after ${clock(job.notBefore, now)}`;
					const state = job.hook ? `${job.state} (${job.hook}-run)` : job.state;
					return [
						job.id,
						state,
						owner,
						String(job.priority),
						job.attempts,
						trunc(job.title, 70),
					];
				}),
			),
		);
	}

	if (view.schedules.length) {
		lines.push("", "Schedules:");
		lines.push(
			table(
				["ID", "NEXT", "TARGET", "CADENCE", "TITLE"],
				view.schedules.map((schedule) => [
					schedule.id,
					schedule.enabled
						? schedule.next
							? clock(schedule.next, now)
							: "-"
						: "paused",
					schedule.target ?? "queue",
					schedule.cadence,
					trunc(schedule.title, 60),
				]),
			),
		);
	}

	if (view.machines.length > 1) {
		lines.push(
			"",
			`Machines: ${view.machines.map((machine) => `${machine.name} (${machine.status})`).join(", ")}`,
		);
	}

	const { hours, completed, failed, canceled } = view.recent;
	lines.push(
		"",
		`Last ${hours}h: ${completed} completed, ${failed} failed, ${canceled} canceled.`,
	);
	if (view.inbox.some((item) => item.context)) {
		lines.push(
			"Blocked agents attached their screen to the inbox item: tower_inbox list shows it.",
		);
	}
	return lines.join("\n");
}

/** `in 3m` / `3m ago` relative to now. */
export function relative(ts: number, now: number = Date.now()): string {
	return ts >= now ? `in ${duration(ts - now)}` : `${duration(now - ts)} ago`;
}
