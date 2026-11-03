// ANSI SGR (color and style) parsing for an agent's screen, as
// `tower read --ansi` prints it: herdr's rendered screen with the colors
// kept. Each line becomes runs of text that share one style; the panel
// draws each run as its own colored element.
//
// Colors are "#rrggbb" for truecolor and the 256-color cube/gray ramp, or
// a theme name "ansi0".."ansi15" for the 16 basic colors, so those follow
// the person's theme. Other escapes (cursor moves, OSC titles) are dropped.

/** @typedef {{ fg?: string, bg?: string, b?: boolean, d?: boolean, i?: boolean, u?: boolean, x?: boolean, inv?: boolean }} Style */
/** @typedef {{ t: string, s: Style }} Run */
/** @typedef {{ runs: Run[], len: number }} Line */

const PLAIN = Object.freeze({});
// CSI (SGR when it ends in "m"), OSC (BEL or ST terminated), other 2-char
// escapes, and the line breaks.
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching escapes is the point
const TOKEN = /\x1b\[([0-9;:?<=>]*)[ -/]*([@-~])|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[@-Z\\-_]|\r?\n|\r/g;
const CUBE = [0, 95, 135, 175, 215, 255];

/** Parse `tower read --ansi` output into lines of styled runs. Style
 * carries across lines, as on a terminal. Trailing blank space (no
 * background) and trailing blank lines are trimmed. */
export function parseAnsi(text) {
	/** @type {Line[]} */
	const lines = [];
	/** @type {Run[]} */
	let runs = [];
	/** @type {Style} */
	let style = PLAIN;
	const push = (t) => {
		if (!t) return;
		const last = runs[runs.length - 1];
		if (last && sameStyle(last.s, style)) last.t += t;
		else runs.push({ t, s: style });
	};
	const endLine = () => {
		lines.push(finishLine(runs));
		runs = [];
	};
	let at = 0;
	for (const m of text.matchAll(TOKEN)) {
		push(text.slice(at, m.index));
		at = m.index + m[0].length;
		if (m[0] === "\n" || m[0] === "\r\n") endLine();
		else if (m[2] === "m") style = applySgr(style, m[1]);
	}
	push(text.slice(at));
	endLine();
	while (lines.length > 0 && lines[lines.length - 1].len === 0) lines.pop();
	return lines;
}

/** Drop trailing whitespace a person cannot see (no background, not
 * inverse), then measure the line in characters. */
function finishLine(runs) {
	while (runs.length > 0) {
		const r = runs[runs.length - 1];
		if (r.s.bg || r.s.inv) break;
		const t = r.t.replace(/\s+$/, "");
		if (t) {
			r.t = t;
			break;
		}
		runs.pop();
	}
	let len = 0;
	for (const r of runs) len += r.t.length;
	return { runs, len };
}

function sameStyle(a, b) {
	return (
		a === b ||
		(a.fg === b.fg &&
			a.bg === b.bg &&
			!a.b === !b.b &&
			!a.d === !b.d &&
			!a.i === !b.i &&
			!a.u === !b.u &&
			!a.x === !b.x &&
			!a.inv === !b.inv)
	);
}

/** One SGR sequence's parameters applied to a style; a new style object. */
function applySgr(style, params) {
	const p = params === "" ? [0] : params.split(/[;:]/).map((n) => (n === "" ? 0 : Number(n)));
	const s = { ...style };
	for (let k = 0; k < p.length; k++) {
		const n = p[k];
		if (n === 0) {
			for (const key of Object.keys(s)) delete s[key];
		} else if (n === 1) s.b = true;
		else if (n === 2) s.d = true;
		else if (n === 3) s.i = true;
		else if (n === 4) s.u = true;
		else if (n === 7) s.inv = true;
		else if (n === 9) s.x = true;
		else if (n === 22) s.b = s.d = false;
		else if (n === 23) s.i = false;
		else if (n === 24) s.u = false;
		else if (n === 27) s.inv = false;
		else if (n === 29) s.x = false;
		else if (n >= 30 && n <= 37) s.fg = `ansi${n - 30}`;
		else if (n >= 90 && n <= 97) s.fg = `ansi${n - 82}`;
		else if (n === 39) delete s.fg;
		else if (n >= 40 && n <= 47) s.bg = `ansi${n - 40}`;
		else if (n >= 100 && n <= 107) s.bg = `ansi${n - 92}`;
		else if (n === 49) delete s.bg;
		else if (n === 38 || n === 48) {
			const key = n === 38 ? "fg" : "bg";
			if (p[k + 1] === 5) {
				const c = color256(p[k + 2]);
				if (c) s[key] = c;
				k += 2;
			} else if (p[k + 1] === 2) {
				// `38;2;r;g;b`, or the colon form with a colorspace slot
				// (`38:2::r:g:b`) when six values follow.
				const skip = params.includes(":") && p.length - k >= 6 ? 1 : 0;
				const [r, g, b] = p.slice(k + 2 + skip, k + 5 + skip);
				s[key] = hex(r, g, b);
				k += 4 + skip;
			}
		}
	}
	return s;
}

/** A 256-color index: the 16 basic colors by theme name, else hex. */
function color256(n) {
	if (!Number.isInteger(n) || n < 0 || n > 255) return null;
	if (n < 16) return `ansi${n}`;
	if (n >= 232) {
		const v = 8 + 10 * (n - 232);
		return hex(v, v, v);
	}
	const i = n - 16;
	return hex(CUBE[Math.floor(i / 36)], CUBE[Math.floor(i / 6) % 6], CUBE[i % 6]);
}

function hex(r, g, b) {
	const h = (v) => Math.max(0, Math.min(255, v | 0)).toString(16).padStart(2, "0");
	return `#${h(r)}${h(g)}${h(b)}`;
}
