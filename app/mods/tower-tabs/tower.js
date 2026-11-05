// Shared tower access for the tower mods: one `tower` CLI call per question,
// JSON in, JSON out, on the machine the panels follow. The CLI resolves the
// server (TOWER_URL, else [server] bind_tcp in the tower home's
// config.toml, else 127.0.0.1:8266) and the token exactly like every other
// tower client, so a mod never holds an address itself.
//
// The machine: shared state `tower.machine` pins one ("" = this computer,
// else a name from `hc machine ls`); unset (null) follows the shown
// space's machine. A remote call runs that machine's own `tower` through
// herdr-core's ssh connection (`exec` with `on`), so it reaches the server
// and token there; nothing listens beyond loopback.
//
// The CLI prints either the API's JSON envelope (success) or
// {"error": {"code": ..., "message": ...}} with exit 1. Both reach the
// caller; towerError() shapes failures for display.

import {
	exec,
	home,
	readTextFile,
	setShared,
	shared,
	spaces,
	writeTextFile,
} from "herdr";

/** A failed tower call. `code` is the server's error code, or "cli". */
export class TowerError extends Error {
	constructor(message, code) {
		super(message);
		this.code = code;
	}
}

const MACHINE = "tower.machine";

/** The pinned machine: "" this computer, a name, or null (follow the space). */
export function pinned() {
	const v = shared(MACHINE);
	return typeof v === "string" ? v : null;
}

/** Pin a machine ("" this computer), or null to follow the shown space. */
export function pin(machine) {
	setShared(MACHINE, machine);
}

/** The machine tower calls go to: null for this computer, else its name. */
export function target() {
	const p = pinned();
	if (p !== null) return p || null;
	return spaces().find((s) => s.active)?.machine ?? null;
}

/** A machine's display name. */
export function machineLabel(machine) {
	return machine ?? "this computer";
}

/** Calls `onChange(machine, cx)` when target() changes; checked every
 * 500 ms (render has no cx, and a space switch must not wait for a
 * panel's own refresh period). Returns the machine at the start. */
export function followMachine(cx, onChange) {
	let last = target();
	cx.timer.every(500, (cx) => {
		const now = target();
		if (now === last) return;
		last = now;
		onChange(now, cx);
	});
	return last;
}

// `tower` on PATH wins; a dev checkout elsewhere falls back to the usual
// spots. A remote call starts in that machine's home, so its fallbacks are
// home-relative.
function candidates(on) {
	if (on === null)
		return [
			"tower",
			`${home()}/.cargo/bin/tower`,
			"/usr/local/bin/tower",
			"/usr/bin/tower",
		];
	return [
		"tower",
		".cargo/bin/tower",
		".local/bin/tower",
		"/usr/local/bin/tower",
		"/usr/bin/tower",
	];
}

const resolved = new Map(); // machine ("" local) -> the CLI that answered

/** The tower CLI on `on` (null: this computer). Only a found CLI is kept:
 * a machine that was offline or lacked tower is asked again next call.
 * Rejects TowerError when no candidate answers `--version`. */
export async function towerBin(on = target()) {
	const key = on ?? "";
	const known = resolved.get(key);
	if (known) return known;
	let failure = null; // a rejection other than "not installed there"
	for (const bin of candidates(on)) {
		try {
			const r = await exec(bin, ["--version"], { on });
			if (r.code === 0) {
				resolved.set(key, bin);
				return bin;
			}
		} catch (e) {
			const message = String(e?.message ?? e);
			if (!/cannot start|no such file|not found/i.test(message))
				failure = message;
		}
	}
	if (failure) throw new TowerError(`${machineLabel(on)}: ${failure}`, "cli");
	throw new TowerError(`tower CLI not found on ${machineLabel(on)}`, "cli");
}

/** `exec` on `on`, a rejection (the machine is unreachable) as TowerError. */
async function run(on, bin, args) {
	try {
		return await exec(bin, args, { on });
	} catch (e) {
		throw new TowerError(`${machineLabel(on)}: ${e?.message ?? e}`, "cli");
	}
}

/** The CLI's result as JSON, or TowerError. */
function parse(r, args) {
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

/** Run `tower <args...> --json` on target(); resolve the parsed JSON,
 * reject TowerError. */
export function tower(...args) {
	return towerOn(target(), ...args);
}

/** tower() on a given machine (null: this computer). */
export async function towerOn(on, ...args) {
	const bin = await towerBin(on);
	return parse(await run(on, bin, [...args, "--json"]), args);
}

/** An agent's recent terminal output as text, from target(). Plain `tower
 * read` (no --json): tower 0.5.0's `read` ignores --json, and the text is
 * the API's `output` field either way. `ansi`: keep the colors (SGR
 * escapes) in the text. Rejects TowerError. */
export async function towerRead(name, { ansi = false, on = target() } = {}) {
	const bin = await towerBin(on);
	const r = await run(
		on,
		bin,
		ansi ? ["read", name, "--ansi"] : ["read", name],
	);
	if (r.code !== 0)
		throw new TowerError((r.stderr || r.stdout || "").trim(), "cli");
	return r.stdout ?? "";
}

/** `tower workflow apply` with `source` as the file: on this computer
 * through `scratch` (a file the mod owns), on another machine through a
 * temporary file there (mods cannot pass stdin). Rejects TowerError. */
export async function applyWorkflow(source, scratch, on = target()) {
	if (on === null) {
		const current = await readTextFile(scratch);
		await writeTextFile(scratch, source, current);
		return towerOn(null, "workflow", "apply", scratch);
	}
	const bin = await towerBin(on);
	const script =
		"f=$(mktemp) || exit 1; trap 'rm -f \"$f\"' EXIT; " +
		'printf %s "$1" > "$f" && "$2" workflow apply "$f" --json';
	const args = ["workflow", "apply"];
	return parse(
		await run(on, "sh", ["-c", script, "tower-apply", source, bin]),
		args,
	);
}

// The tower server's base URL and token as the CLI resolves them (see
// tower-client: TOWER_URL / TOWER_TOKEN, else the tower home's config.toml
// bind and token file), in sh so it resolves on whichever machine runs it.
// `$1` is the path; the rest are curl's own arguments.
const HTTP = `
if [ -n "$TOWER_HOME" ]; then cfg="$TOWER_HOME/config.toml"; tok="$TOWER_HOME/token"
elif [ "$(uname)" = Darwin ]; then d="$HOME/Library/Application Support/tower"; cfg="$d/config.toml"; tok="$d/token"
else cfg="\${XDG_CONFIG_HOME:-$HOME/.config}/tower/config.toml"; tok="\${XDG_DATA_HOME:-$HOME/.local/share}/tower/token"
fi
token="\${TOWER_TOKEN:-$(cat "$tok" 2>/dev/null)}"
token=$(printf %s "$token" | tr -d '[:space:]')
if [ -z "$token" ]; then echo "no tower token found (TOWER_TOKEN or $tok)" >&2; exit 125; fi
url="$TOWER_URL"
if [ -z "$url" ]; then
  bind=$(sed -n '/^\\[server\\]/,/^\\[/s/^[[:space:]]*bind_tcp[[:space:]]*=[[:space:]]*"\\([^"]*\\)".*/\\1/p' "$cfg" 2>/dev/null | head -n 1)
  bind=\${bind:-127.0.0.1:8266}
  case $bind in 0.0.0.0:*) bind="127.0.0.1:\${bind#0.0.0.0:}";; "[::]:"*) bind="127.0.0.1:\${bind#\\[::\\]:}";; esac
  url="http://$bind"
fi
path=$1; shift
exec curl -H "Authorization: Bearer $token" "$@" "\${url%/}$path"
`;

/** curl `path` (e.g. "/v1/events?cursor=3") on target()'s tower server,
 * with `curlArgs` before the URL. Resolves the exec result; exit 125 with
 * stderr when no token is found. Rejects TowerError when the machine is
 * unreachable. */
export function towerHttp(path, curlArgs = [], on = target()) {
	return run(on, "sh", ["-c", HTTP, "tower-http", path, ...curlArgs]);
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
