/** Time display and parsing in the operator's local timezone. Tower timestamps are ms epoch. */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** Compact span: `45s`, `12m`, `3h 5m`, `2d 4h`. Negative spans format as their magnitude. */
export function duration(ms: number): string {
	const span = Math.abs(ms);
	if (span < MINUTE) return `${Math.round(span / 1000)}s`;
	if (span < HOUR) return `${Math.floor(span / MINUTE)}m`;
	if (span < DAY) {
		const minutes = Math.floor((span % HOUR) / MINUTE);
		return minutes
			? `${Math.floor(span / HOUR)}h ${minutes}m`
			: `${Math.floor(span / HOUR)}h`;
	}
	const hours = Math.floor((span % DAY) / HOUR);
	return hours
		? `${Math.floor(span / DAY)}d ${hours}h`
		: `${Math.floor(span / DAY)}d`;
}

function two(value: number): string {
	return String(value).padStart(2, "0");
}

/** Local wall-clock time: `14:05` today, `Mon 14:05` within six days, else `2026-11-02 14:05`. */
export function clock(ts: number, now: number = Date.now()): string {
	const date = new Date(ts);
	const time = `${two(date.getHours())}:${two(date.getMinutes())}`;
	const today = new Date(now);
	if (date.toDateString() === today.toDateString()) return time;
	if (Math.abs(ts - now) < 6 * DAY) return `${WEEKDAYS[date.getDay()]} ${time}`;
	return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())} ${time}`;
}

const RFC3339 =
	/^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?([Zz]|[+-]\d{2}:\d{2})$/;
const LOCAL_DATETIME = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})$/;
const LOCAL_TIME = /^(\d{1,2}):(\d{2})$/;

/**
 * A job time as the operator types it, with the `tower --at` semantics:
 * `HH:MM` (today, or tomorrow when already past), `YYYY-MM-DD HH:MM` in local
 * time, or RFC 3339. Returns ms epoch.
 */
export function parseLocalTime(input: string, now: Date = new Date()): number {
	const text = input.trim();
	if (RFC3339.test(text)) {
		const ts = Date.parse(text.replace(" ", "T"));
		if (!Number.isNaN(ts)) return ts;
	}
	const full = LOCAL_DATETIME.exec(text);
	if (full) {
		const [, year, month, day, hour, minute] = full.map(Number);
		return localMs(year, month, day, hour, minute, text);
	}
	const short = LOCAL_TIME.exec(text);
	if (short) {
		const [, hour, minute] = short.map(Number);
		let ts = localMs(
			now.getFullYear(),
			now.getMonth() + 1,
			now.getDate(),
			hour,
			minute,
			text,
		);
		if (ts <= now.getTime()) {
			const tomorrow = new Date(
				now.getFullYear(),
				now.getMonth(),
				now.getDate() + 1,
			);
			ts = localMs(
				tomorrow.getFullYear(),
				tomorrow.getMonth() + 1,
				tomorrow.getDate(),
				hour,
				minute,
				text,
			);
		}
		return ts;
	}
	throw new Error(
		`unrecognized time "${input}": use HH:MM, "YYYY-MM-DD HH:MM" (local), or RFC 3339`,
	);
}

/** Local date-time to ms, rejecting out-of-range fields and times a DST jump skips. */
function localMs(
	year: number,
	month: number,
	day: number,
	hour: number,
	minute: number,
	text: string,
): number {
	const date = new Date(year, month - 1, day, hour, minute);
	const exact =
		date.getFullYear() === year &&
		date.getMonth() === month - 1 &&
		date.getDate() === day &&
		date.getHours() === hour &&
		date.getMinutes() === minute;
	if (!exact)
		throw new Error(
			`"${text}" is not a valid local time (out of range, or skipped by a DST change)`,
		);
	return date.getTime();
}
