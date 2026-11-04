// The tower event log, followed live: state changes, queue moves, message
// and schedule traffic — the same feed the TUI's events view shows. Each
// row reads time · kind · subject · detail, in names rather than ids;
// clicking a row opens the detail section below the list (every payload
// field, resolved; an agent's screen or a job's result in full).
//
// Loading: the newest page first (GET /v1/events/history, 50 events,
// newest first), then live from the page's `head` over the /v1/events SSE
// stream, so nothing between the two is lost or shown twice. "Load older"
// at the end of the list pages back with `before` = the oldest seq held.
// The mod runtime has no long-lived connection, so the live part rides
// bounded polls: one `curl --max-time 5` every 6s, cursor in hand. Every
// SSE batch carries the seq it reached, so nothing is lost between polls;
// a slow cycle merges into the next.
//
// agent.output events (an agent's whole screen, ~99% of the log) stay out
// of every view but "screens" (?exclude=type:agent.output), as the TUI
// hides them by default. Filters ride the server too (?filter=), for both
// the page and the stream.
//
// The token is the tower token file's content, read directly — the same
// file every tower client reads — and re-read on every poll until one is
// found, so a read that missed (a layout restored before window() reports
// the platform, a token written after the app opened) never sticks the
// panel.

import { Editor, EditorState, h_flex, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { exec, home, readTextFile, settings, shared, theme, window } from "herdr";
import { body, detail, facts, Names, stamp, subject } from "./events.js";
import { stateColor } from "./tower.js";

/** The tower token files to try: a shared-state override if set (mods
 * cannot read TOWER_HOME/TOWER_TOKEN), else the same default tower home
 * the CLI uses per platform (dirs-rs): the OS data dir + "tower/token".
 * Before window() reports the platform, every platform's default. */
function tokenCandidates() {
	const override = shared("tower.token_file");
	if (typeof override === "string" && override.trim()) return [override];
	const mac = `${home()}/Library/Application Support/tower/token`;
	const win = `${home()}/AppData/Roaming/tower/token`;
	const linux = `${home()}/.local/share/tower/token`;
	const platform = window()?.platform;
	if (platform === "macos") return [mac];
	if (platform === "windows") return [win];
	if (platform) return [linux];
	return [mac, linux, win];
}

/** The tower server's base URL: the CLI's default bind, or a shared-state
 * override for non-default setups (mods cannot read TOWER_URL). */
function serverUrl() {
	return String(shared("tower.server_url") ?? "http://127.0.0.1:8266").replace(
		/\/+$/,
		"",
	);
}

const POLL_MS = 6000;
const WINDOW = 5; // seconds per bounded poll
const PAGE = 50; // events per history page
const KEEP = 2000; // rows kept; trimming the oldest leaves "Load older"
const OUTPUT = "agent.output";
const TYPES = [
	["", "all"],
	["agent", "agents"],
	["task", "jobs"],
	["schedule", "schedules"],
	["message", "messages"],
	[OUTPUT, "screens"],
];

export default class TowerEvents extends View {
	init(_props, cx) {
		this.rows = []; // oldest first
		this.cursor = null; // null: load the newest page, then follow from its head
		this.more = false; // older matching events exist
		this.older = false; // a "Load older" page in flight
		this.gen = 0; // bumped by a filter change: older answers are dropped
		this.error = null;
		this.type = "";
		this.alive = false; // one poll answered
		this.polling = false;
		this.token = null; // read by poll() until a token file answers
		this.names = new Names();
		this.selected = null; // the event the detail section shows
		this.editor = null; // its body, read-only
		this.tick = (cx) => {
			if (this.polling) return;
			this.polling = true;
			cx.spawn(async (cx) => {
				let gen = this.gen;
				await Promise.all([this.poll(), this.names.refresh()]);
				// a filter change during the poll: load its page now, not
				// a cycle later
				while (gen !== this.gen) {
					gen = this.gen;
					await this.poll();
				}
				this.polling = false;
				cx.notify();
			});
		};
		this.tick(cx);
		cx.timer.every(POLL_MS, this.tick);
	}

	/** Reads the first non-empty token file; the error names exactly the
	 * paths this attempt tried. */
	async loadToken() {
		const tried = tokenCandidates();
		for (const file of tried) {
			const text = (await readTextFile(file).catch(() => null))?.trim();
			if (text) {
				this.token = text;
				this.error = null;
				return;
			}
		}
		this.error = `no tower token found (tried ${tried.join(", ")})`;
	}

	/** The type filters both routes take, for the current view. */
	query() {
		if (this.type === OUTPUT) return `&filter=type:${OUTPUT}`;
		const filter = this.type ? `&filter=type:${this.type}` : "";
		return `${filter}&exclude=type:${OUTPUT}`;
	}

	/** One history page (newest first) before `before` (null: from the
	 * head), or null when it failed or the filter changed meanwhile. */
	async history(before) {
		const gen = this.gen;
		const from = before === null ? "" : `&before=${before}`;
		const url = `${serverUrl()}/v1/events/history?limit=${PAGE}${from}${this.query()}`;
		let r;
		try {
			r = await exec("curl", [
				"-sS",
				"--max-time",
				"10",
				"-w",
				"\n%{http_code}",
				"-H",
				`Authorization: Bearer ${this.token}`,
				url,
			]);
		} catch {
			this.error = "curl not found";
			return null;
		}
		if (gen !== this.gen) return null;
		const out = r.stdout ?? "";
		const cut = out.lastIndexOf("\n");
		const status = Number(out.slice(cut + 1));
		if (r.code !== 0 || !status) {
			this.error = "cannot reach the tower server";
			this.alive = false;
			return null;
		}
		if (status === 404) {
			this.error = "this panel needs tower 0.6.0 or later (GET /v1/events/history)";
			return null;
		}
		let page;
		try {
			page = JSON.parse(out.slice(0, cut));
		} catch {
			this.error = `unreadable events page (HTTP ${status})`;
			return null;
		}
		if (status !== 200) {
			this.error = page?.error?.message ?? `events page failed (HTTP ${status})`;
			return null;
		}
		for (const e of page.events) this.names.learn(e);
		this.error = null;
		return page;
	}

	/** The newest page, then the live cursor at its head. */
	async loadHead() {
		const page = await this.history(null);
		if (!page) return;
		this.rows = page.events.slice().reverse();
		this.more = page.more;
		this.cursor = page.head;
		this.alive = true;
	}

	/** "Load older": the page before the oldest row held. */
	loadOlder(cx) {
		if (this.older || !this.more || this.cursor === null || !this.rows.length) return;
		this.older = true;
		const gen = this.gen;
		cx.spawn(async (cx) => {
			const page = await this.history(this.rows[0].seq);
			if (gen === this.gen) {
				this.older = false;
				if (page) {
					const seen = new Set(this.rows.map((x) => x.seq));
					const add = page.events.filter((e) => !seen.has(e.seq)).reverse();
					this.rows = add.concat(this.rows);
					this.more = page.more;
				}
			}
			cx.notify();
		});
	}

	/** The newest page on first load, then one bounded SSE poll: every
	 * event after the cursor, then end. */
	async poll() {
		if (!this.token) await this.loadToken();
		if (!this.token) return;
		if (this.cursor === null) return this.loadHead();
		const gen = this.gen;
		const url = `${serverUrl()}/v1/events?cursor=${this.cursor}${this.query()}`;
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
		if (gen !== this.gen) return; // the filter changed mid-poll
		// 28: --max-time ended the poll — the normal end of a bounded read,
		// with nothing new when the stream is caught up
		if (r.code !== 0 && r.code !== 28 && !r.stdout) {
			this.error = "cannot reach the tower server";
			this.alive = false;
			return;
		}
		const text = r.stdout ?? "";
		// SSE messages end in a blank line; a tail cut by --max-time is
		// left for the next poll's replay.
		const blocks = text.split("\n\n");
		if (!text.endsWith("\n\n")) blocks.pop();
		const seen = new Set(this.rows.map((x) => x.seq));
		for (const block of blocks) {
			let reached = null; // the batch's id: the seq it scanned to
			for (const line of block.split("\n")) {
				if (line.startsWith("id:")) reached = Number(line.slice(3).trim());
				if (!line.startsWith("data:")) continue;
				let e;
				try {
					e = JSON.parse(line.slice(5).trim());
				} catch {
					continue; // not an event line
				}
				if (e.seq <= this.cursor || seen.has(e.seq)) continue;
				this.names.learn(e);
				this.rows.push(e);
				seen.add(e.seq);
			}
			// The id covers events the filter skipped, so a filtered view
			// never rescans them.
			if (Number.isFinite(reached)) this.cursor = Math.max(this.cursor, reached);
		}
		if (this.rows.length > KEEP) {
			this.rows = this.rows.slice(-KEEP);
			this.more = true;
		}
		this.error = null;
		this.alive = true;
	}

	/** Show one event in the detail section; the same event again closes it. */
	select(e) {
		this.editor?.release();
		this.editor = null;
		if (!e || this.selected?.seq === e.seq) {
			this.selected = null;
			return;
		}
		this.selected = e;
		const b = body(e);
		if (b) {
			this.editor = EditorState.new({
				language: b.language,
				value: b.text,
				readonly: true,
				line_numbers: false,
				folding: b.language === "json",
				soft_wrap: b.language === "json",
				font_family: settings().font_family,
				font_size: 12,
			});
		}
	}

	render() {
		const t = theme();
		const rows = this.rows.slice().reverse(); // newest first
		return v_flex()
			.id("tower-events")
			.size_full()
			.min_h(0)
			.text_size(12)
			.child(this.header(t))
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
							.children(rows.map((e) => this.row(e, t)))
							.when(rows.length > 0, (el) => el.child(this.listEnd(t))),
					),
			)
			.when(Boolean(this.selected), (el) => el.child(this.details(t)));
	}

	header(t) {
		return h_flex()
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
									if (this.type === v) return;
									this.gen++;
									this.type = v;
									this.cursor = null;
									this.rows = [];
									this.more = false;
									this.older = false;
									this.alive = false;
									this.select(null);
									this.tick(cx2);
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
			);
	}

	/** The end of the list (its oldest end): "Load older" while older
	 * matching events exist, else the start of the log. */
	listEnd(t) {
		if (!this.more) {
			return div().px(14).py(8).text_size(11).text_color(t.dim).child("Start of the log.");
		}
		return h_flex()
			.px(14)
			.py(8)
			.child(
				div()
					.id("ev-older")
					.px(10)
					.py(3)
					.rounded(6)
					.text_size(11)
					.border_1()
					.border_color(t.border)
					.bg(t.raised)
					.text_color(this.older ? t.dim : t.foreground)
					.cursor_pointer()
					.hover((s) => s.bg(t.hover))
					.on_click((_e, cx2) => {
						this.loadOlder(cx2);
						cx2.notify();
					})
					.child(this.older ? "Loading…" : `Load older (${PAGE})`),
			);
	}

	row(e, t) {
		const picked = this.selected?.seq === e.seq;
		// Two lines, so a narrow side panel still shows the detail: time ·
		// kind · subject, then what happened.
		return v_flex()
			.id(`ev-${e.seq}`)
			.px(14)
			.py(4)
			.gap(1)
			.cursor_pointer()
			.when(picked, (el) => el.bg(t.selected))
			.hover((s) => s.bg(t.hover))
			.on_click((_ev, cx2) => {
				this.select(e);
				cx2.notify();
			})
			.child(
				h_flex()
					.gap(8)
					.items_center()
					.child(
						div()
							.w(84)
							.flex_none()
							.truncate()
							.text_size(11)
							.text_color(t.dim)
							.child(stamp(e.ts)),
					)
					.child(
						div()
							.w(100)
							.flex_none()
							.truncate()
							.text_size(11)
							.text_color(t[kindColor(e)])
							.child(e.kind.replace(".", " ")),
					)
					.child(
						div()
							.flex_1()
							.min_w(0)
							.truncate()
							.text_color(t.foreground)
							.child(subject(e, this.names)),
					),
			)
			.child(
				div()
					.pl(92)
					.min_w(0)
					.truncate()
					.text_color(t.muted)
					.child(detail(e, this.names)),
			);
	}

	/** The detail section: the selected event's facts, then its body. */
	details(t) {
		const e = this.selected;
		return v_flex()
			.id("ev-detail")
			.when(Boolean(this.editor), (el) => el.flex_1())
			.when(!this.editor, (el) => el.flex_none())
			.min_h(0)
			.border_t(1)
			.border_color(t.border)
			.child(
				h_flex()
					.h(30)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.child(div().text_color(t[kindColor(e)]).child(e.kind))
					.child(
						div()
							.flex_1()
							.min_w(0)
							.truncate()
							.font_semibold()
							.child(subject(e, this.names)),
					)
					.child(
						div()
							.id("ev-detail-close")
							.px(6)
							.rounded(4)
							.cursor_pointer()
							.text_color(t.dim)
							.hover((s) => s.bg(t.hover))
							.on_click((_ev, cx2) => {
								this.select(null);
								cx2.notify();
							})
							.child("×"),
					),
			)
			.child(
				v_flex()
					.flex_none()
					.px(14)
					.pb(8)
					.gap(2)
					.children(
						facts(e, this.names).map(([label, value], i) =>
							h_flex()
								.id(`ev-fact-${i}`)
								.gap(8)
								.child(
									div()
										.w(110)
										.flex_none()
										.text_size(11)
										.text_color(t.dim)
										.child(label),
								)
								.child(
									div()
										.flex_1()
										.min_w(0)
										.text_color(t.foreground)
										.child(value),
								),
						),
					),
			)
			.when(Boolean(this.editor), (el) =>
				el.child(
					div()
						.flex_1()
						.min_h(0)
						.border_t(1)
						.border_color(t.border)
						.child(Editor.new(this.editor).size_full()),
				),
			);
	}
}

/** A row's kind color: the state an event moved to, else a coarse one. */
function kindColor(e) {
	const p = e.payload ?? {};
	if (e.kind === "agent.state") return stateColor(p.to);
	if (e.kind === "task.status" || e.kind === "task.leased_out")
		return stateColor(p.state);
	if (e.kind.endsWith("failed") || e.kind === "schedule.skipped") return "ansi1";
	if (e.kind === "task.completed" || e.kind === "schedule.resumed")
		return stateColor("completed");
	return "dim";
}
