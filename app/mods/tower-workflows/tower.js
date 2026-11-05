// Shared tower access for the tower mods: one `tower` CLI call per question,
// JSON in, JSON out. The CLI resolves the server (TOWER_URL, else
// [server] bind_tcp in the tower home's config.toml, else 127.0.0.1:8266)
// and the token exactly like every other tower client, so a mod never
// holds an address itself.
//
// The CLI prints either the API's JSON envelope (success) or
// {"error": {"code": ..., "message": ...}} with exit 1. Both reach the
// caller; towerError() shapes failures for display.

import { exec, home } from "herdr";

/** A failed tower call. `code` is the server's error code, or "cli". */
export class TowerError extends Error {
	constructor(message, code) {
		super(message);
		this.code = code;
	}
}

// `tower` on PATH wins; a dev checkout elsewhere falls back to the usual spots.
const CANDIDATES = [
	"tower",
	`${home()}/.cargo/bin/tower`,
	"/usr/local/bin/tower",
	"/usr/bin/tower",
];

let resolved;

/** The tower CLI to call, or null when no candidate answers `--version`. */
export async function towerBin() {
	if (resolved !== undefined) return resolved;
	for (const bin of CANDIDATES) {
		try {
			const r = await exec(bin, ["--version"]);
			if (r.code === 0) {
				resolved = bin;
				return bin;
			}
		} catch {
			// not installed there; try the next candidate
		}
	}
	resolved = null;
	return null;
}

/** Run `tower <args...> --json`; resolve the parsed JSON, reject TowerError. */
export async function tower(...args) {
	const bin = await towerBin();
	if (!bin) throw new TowerError("tower CLI not found on this machine", "cli");
	const r = await exec(bin, [...args, "--json"]);
	if (r.code !== 0) {
		let message = (r.stderr || r.stdout || "").trim();
		let code = "cli";
		try {
			const parsed = JSON.parse(message);
			if (parsed?.error) {
				code = parsed.error.code ?? code;
				message = parsed.error.message ?? message;
			}
		} catch {
			// not JSON: the harness's own message
		}
		throw new TowerError(message, code);
	}
	try {
		return JSON.parse(r.stdout);
	} catch {
		throw new TowerError(
			`unparseable output from tower ${args.join(" ")}`,
			"cli",
		);
	}
}

/** An agent's recent terminal output as text. Plain `tower read` (no
 * --json): tower 0.5.0's `read` ignores --json, and the text is the API's
 * `output` field either way. `ansi`: keep the colors (SGR escapes) in the
 * text. Rejects TowerError. */
export async function towerRead(name, { ansi = false } = {}) {
	const bin = await towerBin();
	if (!bin) throw new TowerError("tower CLI not found on this machine", "cli");
	const r = await exec(bin, ansi ? ["read", name, "--ansi"] : ["read", name]);
	if (r.code !== 0) throw new TowerError((r.stderr || r.stdout || "").trim(), "cli");
	return r.stdout ?? "";
}

/** The text of a message's parts, joined. */
export function messageText(m) {
	return (m.parts ?? [])
		.map((p) => p.text ?? "")
		.join("")
		.trim();
}

const MS = 1000;
const MIN = 60 * MS;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** Short "2m ago"/"in 3h" for ms-epoch timestamps. */
export function ago(ts) {
	const s = (Date.now() - ts) / MS;
	if (s < 1) return "now";
	if (s < 60) return `${Math.round(s)}s ago`;
	if (s < 3600) return `${Math.round(s / 60)}m ago`;
	if (s < 86400) return `${Math.round(s / 3600)}h ago`;
	return `${Math.round(s / 86400)}d ago`;
}

/** Short "in 2m"/"in 3h", floored at 1m; a week out spells the weekday. */
export function until(ts) {
	const s = (ts - Date.now()) / MS;
	if (s <= 0) return "now";
	if (s < 60) return "in <1m";
	if (s < 3600) return `in ${Math.round(s / 60)}m`;
	if (s < 86400) return `in ${Math.round(s / 3600)}h`;
	const d = new Date(ts);
	// The mod runtime ignores toLocaleDateString options: spell it out.
	const day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][d.getDay()];
	const hm = d.toTimeString().slice(0, 5);
	return s < 7 * DAY ? `${day} ${hm}` : d.toDateString();
}

// State colors shared by every tower mod: herdr theme names, so the
// whole board follows the person's theme.
export const STATE_COLOR = {
	working: "working",
	blocked: "blocked",
	done: "done",
	idle: "idle",
	launching: "unknown",
	unknown: "unknown",
	dead: "ansi1",
	queued: "dim",
	assigned: "accent",
	"input-required": "blocked",
	completed: "done",
	failed: "ansi1",
	canceled: "dim",
	rejected: "dim",
	// workflow runs and steps (D§5.6)
	running: "working",
	pending: "dim",
	ready: "accent",
};

/** A state's theme color name, unknown states included. */
export function stateColor(state) {
	return STATE_COLOR[state] ?? "unknown";
}
