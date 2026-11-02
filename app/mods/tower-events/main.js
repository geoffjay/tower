// The tower event log, followed live: state changes, queue moves, message
// and schedule traffic — the same feed the TUI's events view shows.
//
// tower's /v1/events is an SSE stream with cursor replay: every event
// carries a seq, and a reconnect from the last seq gets exactly what it
// missed. The mod runtime has no long-lived connection, so this mod rides
// bounded polls: one `curl --max-time 5` every 6s, cursor in hand. Nothing
// is lost between polls (the cursor holds the place); a slow cycle merges
// into the next.
//
// The token is the tower token file's content, read directly — the same
// file every tower client reads. Filters ride the server (?filter=), so a
// filtered view replays only its own events.

import { h_flex, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { exec, home, readTextFile, theme } from "herdr";
import { clock, eventLine, stateColor } from "./tower.js";

const POLL_MS = 6000;
const WINDOW = 5; // seconds per bounded poll
const KEEP = 300; // rows kept
const TYPES = [
	["", "all"],
	["agent", "agents"],
	["task", "jobs"],
	["schedule", "schedules"],
	["message", "messages"],
];

export default class TowerEvents extends View {
	init(_props, cx) {
		this.rows = [];
		this.cursor = 0; // 0: replay the whole log on the first poll
		this.error = null;
		this.type = "";
		this.alive = false; // one poll answered
		this.polling = false;
		this.token = null;
		cx.spawn(async (cx) => {
			// The default tower home, as the CLI resolves it when nothing
			// overrides (mods cannot read the environment).
			this.token =
				(await readTextFile(`${home()}/.local/share/tower/token`))?.trim() ??
				"";
			if (!this.token) {
				this.error = "no tower token found (~/.local/share/tower/token)";
				cx.notify();
			}
		});
		cx.timer.every(POLL_MS, (cx) => {
			if (this.polling) return;
			cx.spawn(async (cx) => {
				this.polling = true;
				await this.poll();
				this.polling = false;
				cx.notify();
			});
		});
	}

	/** One bounded SSE poll: every event after the cursor, then end. */
	async poll() {
		if (!this.token) return;
		const filter = this.type ? `&filter=type:${this.type}` : "";
		const url = `http://127.0.0.1:8266/v1/events?cursor=${this.cursor}${filter}`;
		let r;
		try {
			r = await exec("curl", [
				"-sS",
				"-N",
				"--max-time",
				String(WINDOW),
				"-H",
				`Authorization: Bearer ${this.token}`,
				url,
			]);
		} catch {
			this.error = "curl not found";
			return;
		}
		if (r.code !== 0 && !r.stdout) {
			this.error = "cannot reach the tower server";
			this.alive = false;
			return;
		}
		const seen = new Set(this.rows.map((x) => x.seq));
		let added = 0;
		for (const line of (r.stdout ?? "").split("\n")) {
			if (!line.startsWith("data:")) continue;
			try {
				const e = JSON.parse(line.slice(5).trim());
				if (e.seq <= this.cursor || seen.has(e.seq)) continue;
				this.rows.push(e);
				this.cursor = Math.max(this.cursor, e.seq);
				added++;
			} catch {
				// a torn chunk: the next poll's cursor replay covers it
			}
		}
		if (added > 0) {
			if (this.rows.length > KEEP) this.rows = this.rows.slice(-KEEP);
			this.error = null;
		}
		this.alive = true;
	}

	render() {
		const t = theme();
		const rows = this.rows.slice().reverse(); // newest first
		return v_flex()
			.id("tower-events")
			.size_full()
			.min_h(0)
			.text_size(12)
			.child(
				h_flex()
					.h(36)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.border_b(1)
					.border_color(t.border)
					.child(div().font_semibold().child("Events"))
					.child(
						h_flex()
							.gap(4)
							.children(
								TYPES.map(([v, label]) =>
									h_flex()
										.id(`ty-${v || "all"}`)
										.h(22)
										.px(7)
										.items_center()
										.rounded(6)
										.text_size(11)
										.cursor_pointer()
										.bg(this.type === v ? t.selected : t.raised)
										.text_color(this.type === v ? t.foreground : t.muted)
										.border_1()
										.border_color(this.type === v ? t.accent : t.border)
										.hover((s) => s.bg(t.hover))
										.on_click((_e, cx2) => {
											this.type = v;
											this.cursor = 0;
											this.rows = [];
											this.alive = false;
											cx2.notify();
										})
										.child(label),
								),
							),
					)
					.child(div().flex_1())
					.child(
						div()
							.id("ev-live")
							.text_size(11)
							.text_color(this.alive ? t.dim : t.ansi1)
							.child(this.alive ? "● live" : "◌ connecting"),
					),
			)
			.when(Boolean(this.error), (el) =>
				el.child(div().px(14).py(6).text_color(t.ansi1).child(this.error)),
			)
			.child(
				div()
					.id("tower-events-body")
					.flex_1()
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						v_flex()
							.pb(10)
							.when(rows.length === 0, (el) =>
								el.child(
									div()
										.px(14)
										.py(8)
										.text_color(t.dim)
										.child(this.alive ? "No events." : "Connecting…"),
								),
							)
							.children(
								rows.map((e) =>
									h_flex()
										.id(`ev-${e.seq}`)
										.px(14)
										.py(3)
										.gap(8)
										.items_center()
										.child(
											div()
												.w(38)
												.flex_none()
												.text_size(11)
												.text_color(t.dim)
												.child(clock(e.ts)),
										)
										.child(
											div()
												.w(86)
												.flex_none()
												.text_size(11)
												.text_color(t[stateColor(kindState(e.kind))])
												.child(e.kind.replace(".", " ")),
										)
										.child(
											div()
												.flex_1()
												.min_w(0)
												.truncate()
												.text_color(t.muted)
												.child(eventLine(e)),
										),
								),
							),
					),
			);
	}
}

/** A coarse state for an event kind: failed / completed / unknown / queued. */
function kindState(kind) {
	if (kind.endsWith("failed") || kind === "schedule.skipped") return "failed";
	if (kind === "task.completed" || kind === "schedule.resumed")
		return "completed";
	if (kind.startsWith("agent.state")) return "unknown";
	return "queued";
}
