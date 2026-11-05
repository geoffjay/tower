// The tower fleet: every agent seat, its live state, and the job it works.
// One section per stack (agents named <stack>-<role>), unstacked agents last.
// Clicking a row opens the detail section below the list (the same shape as
// the events panel): the agent's facts, a prompt bar, its actions, and its
// terminal screen, following its newest output.
// Reads: `tower ps`, `tower task list`, `tower read <name>`. Acts: prompt,
// interrupt, stop / remove (two presses), resume (`spawn --continue`, dead
// rows), spawn (a form of installed harnesses).

import {
	Editor,
	EditorState,
	h_flex,
	Input,
	InputState,
	Scrollbar,
	VirtualListScrollHandle,
	v_flex,
	v_virtual_list,
} from "gpui-base";
import { div, View } from "gpui-kit";
import {
	closeOverlay,
	knownAgents,
	openOverlay,
	props,
	settings,
	theme,
} from "herdr";
import { parseAnsi } from "./ansi.js";
import { stateColor, tower, towerRead } from "./tower.js";

const REFRESH_MS = 10000;
const SCREEN_MS = 2000; // the open agent's screen re-reads this often
const SCREEN_LINES = 200; // the tail of `tower read` kept on screen
const LINE_H = 17; // px per screen line (12px mono)
const CHAR_W = 7.3; // px per column, for the screen's scroll width

export default class TowerAgents extends View {
	init(_props, cx) {
		// Opened as an overlay by `openOverlay({ mod: "tower-agents", props:
		// { output } })`: only the read-only output viewer, no polling.
		const out = props()?.output;
		if (out) {
			this.viewer = {
				name: out.name,
				overlay: props().overlay,
				editor: EditorState.new({
					value: out.text,
					readonly: true,
					line_numbers: false,
					soft_wrap: false,
					font_family: settings().font_family,
					font_size: 12,
				}),
			};
			return;
		}
		this.agents = null;
		this.jobs = []; // open jobs (queued / working / input-required)
		this.error = null; // the last action's failure, kept until the next action
		this.loadError = null; // the last poll's failure
		this.busy = null; // agent name with an action in flight
		this.confirm = null; // "stop:<name>" / "remove:<name>" awaiting a second press
		this.spawn = null; // the spawn form's fields
		this.selected = null; // the name of the agent in the detail section
		this.prompt = null; // its prompt bar's InputState
		this.lines = null; // its screen: lines of styled runs (ansi.js)
		this.cols = 0; // its widest line, in characters
		this.screenText = null; // the raw read, to skip unchanged ones
		this.follow = true; // keep the screen pinned to its newest line
		this.reading = false; // a `tower read` in flight
		this.scroll = VirtualListScrollHandle.new();
		cx.timer.every(REFRESH_MS, (cx) => this.refresh(cx));
		cx.timer.every(SCREEN_MS, (cx) => {
			if (!this.selected || this.reading) return;
			cx.spawn(async (cx) => {
				if (await this.readScreen()) cx.notify();
			});
		});
		this.refresh(cx);
	}

	async refresh(cx) {
		try {
			const [ps, tasks] = await Promise.all([
				tower("ps"),
				tower("task", "list").catch(() => null),
			]);
			this.agents = ps.agents ?? [];
			this.jobs = (tasks?.tasks ?? []).filter(
				(j) =>
					!["completed", "failed", "canceled", "rejected"].includes(j.state),
			);
			this.loadError = null;
		} catch (e) {
			this.loadError = String(e.message ?? e);
		}
		if (this.selected && this.agents && !this.find(this.selected)) {
			this.select(null); // removed elsewhere
		}
		if (this.selected) await this.readScreen();
		cx.notify();
	}

	find(name) {
		return (this.agents ?? []).find((a) => a.name === name) ?? null;
	}

	/** Open one agent in the detail section; the same agent again closes it. */
	select(name, cx) {
		this.error = null;
		this.prompt?.release();
		this.prompt = null;
		this.lines = null;
		this.cols = 0;
		this.screenText = null;
		this.follow = true;
		this.confirm = null;
		if (!name || this.selected === name) {
			this.selected = null;
			return;
		}
		this.selected = name;
		const state = InputState.new({ placeholder: `Prompt ${name}…` });
		state.on("submit", (_e, cx2) => this.sendPrompt(cx2));
		this.prompt = state;
		this.lines = parseAnsi("…");
		cx?.spawn(async (cx) => {
			await this.readScreen();
			cx.notify();
		});
	}

	/** The selected agent's recent terminal output, colors kept, onto the
	 * screen. True when the screen changed. While following, the screen
	 * scrolls to the newest line. */
	async readScreen() {
		const name = this.selected;
		const a = this.find(name);
		let text;
		this.reading = true;
		try {
			if (a?.state === "dead") {
				text = "(not running - Resume relaunches it)";
			} else {
				try {
					text = await towerRead(name, { ansi: true });
				} catch (e) {
					text = `cannot read ${name}: ${e.message}`;
				}
			}
		} finally {
			this.reading = false;
		}
		// A late answer for an agent no longer selected is dropped.
		if (this.selected !== name || text === this.screenText) return false;
		this.screenText = text;
		const lines = parseAnsi(text).slice(-SCREEN_LINES);
		this.lines = lines.length > 0 ? lines : parseAnsi("(no output)");
		this.cols = this.lines.reduce((m, l) => Math.max(m, l.len), 0);
		if (this.follow) this.scroll.scroll_to_bottom();
		return true;
	}

	/** Back to the newest line, and stay there as output arrives. */
	followScreen() {
		this.follow = true;
		this.scroll.scroll_to_bottom();
	}

	sendPrompt(cx) {
		const name = this.selected;
		const text = this.prompt?.value().trim();
		if (!name || !text) return;
		this.prompt.set_value("");
		this.busy = name;
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				await tower("prompt", name, text);
				this.error = null;
			} catch (e) {
				this.error = `${name}: ${e.message}`;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	button(id, label, t, onClick, opts = {}) {
		return h_flex()
			.id(id)
			.h(opts.h ?? 22)
			.px(opts.px ?? 6)
			.flex_none()
			.flex()
			.items_center()
			.rounded(6)
			.text_size(11)
			.bg(opts.solid ? t.solid : t.raised)
			.text_color(
				opts.solid ? t.solid_foreground : opts.danger ? t.ansi1 : t.muted,
			)
			.border_1()
			.border_color(opts.solid ? t.solid : t.border)
			.cursor_pointer()
			.hover((s) => s.opacity(0.8))
			.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
			.on_click((_e, cx) => onClick(cx))
			.child(label);
	}

	/** Run a tower verb for an agent; `confirmKey` asks for a second press. */
	act(cx, name, args, confirmKey) {
		if (confirmKey && this.confirm !== confirmKey) {
			this.confirm = confirmKey;
			cx.notify();
			return;
		}
		this.confirm = null;
		this.busy = name;
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				await tower(...args);
				this.error = null;
			} catch (e) {
				this.error = `${name}: ${e.message}`;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	/** The spawn form: name, harness pills, workdir, first prompt. */
	openSpawn(cx) {
		this.error = null;
		this.spawn = {
			name: InputState.new({ placeholder: "reviewer" }),
			kind: null,
			installed: null,
			def: false,
			workdir: InputState.new({ placeholder: "/home/you/project" }),
			prompt: InputState.new({
				placeholder: "First prompt (sent when it settles)",
			}),
		};
		cx.spawn(async (cx) => {
			try {
				this.spawn.installed = (await knownAgents())
					.filter((a) => a.installed)
					.map((a) => a.key);
			} catch {
				this.spawn.installed = [];
			}
			if (!this.spawn.installed.includes(this.spawn.kind))
				this.spawn.kind = this.spawn.installed[0] ?? null;
			cx.notify();
		});
		cx.notify();
	}

	async submitSpawn(cx) {
		const f = this.spawn;
		const name = f.name.value().trim();
		if (!name || !f.kind) {
			this.error = "spawn needs a name and a kind";
			cx.notify();
			return;
		}
		cx.spawn(async (cx) => {
			try {
				const args = ["spawn", name, "--kind", f.kind];
				if (f.def) args.push("--worktree");
				if (f.workdir.value().trim())
					args.push("--workdir", f.workdir.value().trim());
				if (f.prompt.value().trim())
					args.push("--prompt", f.prompt.value().trim());
				await tower(...args);
				this.spawn = null;
				this.error = null;
			} catch (e) {
				this.error = `spawn: ${e.message}`;
			}
			await this.refresh(cx);
		});
	}

	/** The open job an agent owns, if any. */
	jobOf(a) {
		return this.jobs.find((j) => j.owner_id === a.id) ?? null;
	}

	row(a, t) {
		const c = stateColor(a.state);
		const picked = this.selected === a.name;
		const job = this.jobOf(a);
		return v_flex()
			.id(`agent-${a.name}`)
			.mx(8)
			.px(8)
			.py(5)
			.gap(2)
			.rounded(7)
			.cursor_pointer()
			.when(picked, (el) => el.bg(t.selected))
			.hover((s) => s.bg(t.hover))
			.on_click((_e, cx) => {
				this.select(a.name, cx);
				cx.notify();
			})
			.child(
				h_flex()
					.gap(7)
					.items_center()
					.child(div().size(8).rounded(4).flex_none().bg(t[c]))
					.child(
						div().flex_1().min_w(0).truncate().font_semibold().child(a.name),
					)
					.child(
						div()
							.flex_none()
							.text_size(11)
							.text_color(t[c])
							.child(this.busy === a.name ? "…" : a.state),
					),
			)
			.child(
				div()
					.pl(15)
					.min_w(0)
					.truncate()
					.text_size(11)
					.text_color(t.muted)
					.child(
						job ? `job: ${job.title}` : (a.workdir ?? a.worktree ?? a.kind),
					),
			);
	}

	/** Label / value rows for the detail section. */
	facts(a) {
		const job = this.jobOf(a);
		const reserved = this.jobs.filter(
			(j) => j.state === "queued" && !j.owner_id && j.target_agent_id === a.id,
		).length;
		const when = (ts) => {
			if (!ts) return "-";
			const d = new Date(ts);
			return `${d.toDateString().slice(4, 10)} ${d.toTimeString().slice(0, 5)}`;
		};
		// [label, value, wide]: wide facts take a full row, the rest pair up
		// two per row so the screen below keeps the room.
		return [
			["job", job ? `${job.title} (${job.state})` : "-", true],
			["workdir", a.workdir ?? "-", true],
			["kind", a.kind],
			["reserved", reserved ? `${reserved} queued` : "-"],
			["permissions", a.permissions],
			["worktree", a.worktree ?? "-"],
			["machine", a.machine_id ?? "local"],
			["pane", a.pane_id ?? "-"],
			["desired", a.desired_state],
			["adopted", a.adopted ? "yes" : "no"],
			["created", when(a.created_at)],
			["updated", when(a.updated_at)],
			["id", a.id, true],
		];
	}

	/** The detail section: header, prompt bar, actions, facts, then screen. */
	details(t) {
		const a = this.find(this.selected);
		if (!a) return div();
		const c = stateColor(a.state);
		const dead = a.state === "dead";
		const stopKey = `stop:${a.name}`;
		const removeKey = `remove:${a.name}`;
		return v_flex()
			.id("ag-detail")
			.flex_1()
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
					.child(div().size(8).rounded(4).flex_none().bg(t[c]))
					.child(
						div().flex_1().min_w(0).truncate().font_semibold().child(a.name),
					)
					.child(div().text_size(11).text_color(t[c]).child(a.state))
					.child(
						div()
							.id("ag-detail-close")
							.px(6)
							.rounded(4)
							.cursor_pointer()
							.text_color(t.dim)
							.hover((s) => s.bg(t.hover))
							.on_click((_e, cx) => {
								this.select(null);
								cx.notify();
							})
							.child("×"),
					),
			)
			.when(!dead, (el) =>
				el.child(
					h_flex()
						.flex_none()
						.px(14)
						.pb(6)
						.gap(6)
						.items_center()
						.child(
							div()
								.flex_1()
								.h(26)
								.px(8)
								.flex()
								.items_center()
								.rounded(6)
								.border_1()
								.border_color(t.input)
								.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
								.child(Input.new(this.prompt).w_full().h_full().text_size(12)),
						)
						.child(
							this.button("ag-send", "Send", t, (cx) => this.sendPrompt(cx), {
								solid: true,
							}),
						),
				),
			)
			.child(
				h_flex()
					.flex_none()
					.flex_wrap()
					.px(14)
					.pb(8)
					.gap(6)
					.when(!dead, (el) =>
						el
							.child(
								this.button("ag-int", "Interrupt", t, (cx) =>
									this.act(cx, a.name, ["interrupt", a.name]),
								),
							)
							.child(
								this.button(
									"ag-stop",
									this.confirm === stopKey ? "Stop?" : "Stop",
									t,
									(cx) => this.act(cx, a.name, ["stop", a.name], stopKey),
									{ danger: true },
								),
							),
					)
					.when(dead, (el) =>
						el.child(
							this.button(
								"ag-resume",
								"Resume",
								t,
								(cx) => this.act(cx, a.name, ["spawn", a.name, "--continue"]),
								{ solid: true },
							),
						),
					)
					.child(
						this.button(
							"ag-remove",
							this.confirm === removeKey ? "Remove?" : "Remove",
							t,
							(cx) =>
								this.act(cx, a.name, ["stop", a.name, "--remove"], removeKey),
							{ danger: true },
						),
					)
					.child(div().flex_1())
					.child(
						this.button("ag-copy", "Copy…", t, (cx) => this.openOutput(cx)),
					)
					.child(
						this.button(
							"ag-follow",
							this.follow ? "Following" : "↓ Follow",
							t,
							(cx) => {
								if (this.follow) this.follow = false;
								else this.followScreen();
								cx.notify();
							},
							{ solid: !this.follow },
						),
					),
			)
			.child(
				h_flex()
					.flex_none()
					.flex_wrap()
					.px(14)
					.pb(8)
					.text_size(12)
					.children(
						this.facts(a).map(([label, value, wide], i) =>
							h_flex()
								.id(`ag-fact-${i}`)
								.w(wide ? "100%" : "50%")
								.min_w(0)
								.py(1)
								.pr(8)
								.gap(8)
								.child(
									div()
										.w(84)
										.flex_none()
										.text_size(11)
										.text_color(t.dim)
										.child(label),
								)
								.child(
									div()
										.flex_1()
										.min_w(0)
										.truncate()
										.text_color(t.foreground)
										.child(String(value ?? "-")),
								),
						),
					),
			)
			.child(this.screenView(t));
	}

	/** The agent's screen: one row per line in a virtual list, each line its
	 * styled runs. While following, each new read scrolls it to the newest
	 * line; scrolling up stops that until Follow is pressed. */
	screenView(t) {
		const lines = this.lines ?? [];
		const cols = this.cols;
		return div()
			.flex_1()
			.min_h(0)
			.relative()
			.border_t(1)
			.border_color(t.border)
			.font_family(settings().font_family ?? "JetBrains Mono")
			.text_size(12)
			.on_scroll_wheel((e, cx) => {
				// GPUI: a positive y delta moves toward the top.
				if (this.follow && e.delta.y > 0) {
					this.follow = false;
					cx.notify();
				}
			})
			.child(
				div()
					.id("ag-screen-x")
					.size_full()
					.overflow_x_scrollbar()
					.child(
						div()
							.w_full()
							.min_w(Math.ceil(cols * CHAR_W) + 20)
							.h_full()
							.child(
								v_virtual_list(
									"ag-screen",
									lines.length,
									LINE_H,
									(i) => String(i),
									(range) =>
										lines.slice(range.start, range.end).map((l) =>
											h_flex()
												.h(LINE_H)
												.px(10)
												.whitespace_nowrap()
												.text_color(t.foreground)
												.children(
													l.runs.length > 0
														? l.runs.map((r) => this.run(r, t))
														: [div().child(" ")],
												),
										),
								)
									.track_scroll(this.scroll)
									.size_full(),
							),
					),
			)
			.child(
				Scrollbar.vertical("ag-screen")
					.viewport_from_layout()
					.absolute()
					.inset_0(),
			);
	}

	/** One styled run of a screen line. Basic colors are theme names
	 * (ansi0..ansi15); the rest are hex. */
	run(r, t) {
		const s = r.s;
		let fg = s.fg ? (t[s.fg] ?? s.fg) : null;
		let bg = s.bg ? (t[s.bg] ?? s.bg) : null;
		if (s.inv) [fg, bg] = [bg ?? t.background, fg ?? t.foreground];
		let el = div().flex_none().h(LINE_H).child(r.t);
		if (fg) el = el.text_color(fg);
		if (bg) el = el.bg(bg);
		if (s.b) el = el.font_bold();
		if (s.i) el = el.italic();
		if (s.u) el = el.underline();
		if (s.x) el = el.line_through();
		if (s.d) el = el.opacity(0.6);
		return el;
	}

	spawnForm(t) {
		const f = this.spawn;
		const label = (text) =>
			div().pt(4).text_size(11).font_semibold().text_color(t.dim).child(text);
		const fieldBox = (id, state) =>
			div()
				.id(id)
				.h(30)
				.px(9)
				.flex()
				.items_center()
				.rounded(7)
				.border_1()
				.border_color(t.input)
				.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
				.child(Input.new(state).h_full().w_full().text_size(12));
		const pills = h_flex()
			.flex_wrap()
			.gap(5)
			.children(
				(f.installed ?? []).map((v) =>
					h_flex()
						.id(`k-${v}`)
						.h(24)
						.px(8)
						.items_center()
						.rounded(6)
						.text_size(12)
						.cursor_pointer()
						.bg(v === f.kind ? t.selected : t.raised)
						.text_color(v === f.kind ? t.foreground : t.muted)
						.border_1()
						.border_color(v === f.kind ? t.accent : t.border)
						.hover((s) => s.bg(t.hover))
						.on_click((_e, cx) => {
							f.kind = v;
							cx.notify();
						})
						.child(v),
				),
			)
			.when(!f.installed, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("loading…")),
			);
		return v_flex()
			.p(14)
			.gap(6)
			.child(div().text_size(15).font_semibold().child("Spawn a tower agent"))
			.child(label("NAME"))
			.child(fieldBox("sf-name", f.name))
			.child(label("HARNESS"))
			.child(pills)
			.child(label("WORKDIR"))
			.child(fieldBox("sf-dir", f.workdir))
			.child(label("FIRST PROMPT"))
			.child(fieldBox("sf-task", f.prompt))
			.child(
				h_flex()
					.pt(8)
					.gap(8)
					.justify_end()
					.child(
						this.button("sf-cancel", "Cancel", t, (cx) => {
							this.spawn = null;
							this.error = null;
							cx.notify();
						}),
					)
					.child(
						this.button("sf-go", "Spawn", t, (cx) => this.submitSpawn(cx), {
							solid: true,
						}),
					),
			);
	}

	/** The screen as plain text, one string, for the copy dialog. */
	plainScreen() {
		return (this.lines ?? [])
			.map((l) => l.runs.map((r) => r.t).join(""))
			.join("\n");
	}

	/** A dialog with a snapshot of the screen as selectable text. */
	openOutput(_cx) {
		if (!this.selected) return;
		openOverlay({
			mod: "tower-agents",
			kind: "dialog",
			width: 0.7,
			height: 0.8,
			props: { output: { name: this.selected, text: this.plainScreen() } },
		});
	}

	close() {
		this.viewer?.editor.release();
	}

	/** The overlay copy's whole view: a title bar over the read-only text. */
	viewerView(t) {
		const v = this.viewer;
		return v_flex()
			.id("tower-agents-output")
			.size_full()
			.min_h(0)
			.bg(t.popover)
			.text_size(13)
			.child(
				h_flex()
					.h(40)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.border_b(1)
					.border_color(t.border)
					.child(div().font_semibold().child(`${v.name} output`))
					.child(
						div().text_size(11).text_color(t.dim).child("select, then copy"),
					)
					.child(div().flex_1())
					.child(
						this.button("out-close", "Close", t, () => closeOverlay(v.overlay)),
					),
			)
			.child(div().flex_1().min_h(0).child(Editor.new(v.editor)));
	}

	render() {
		const t = theme();
		if (this.viewer) return this.viewerView(t);
		const agents = this.agents ?? [];
		const stacks = new Map();
		for (const a of agents) {
			const key = a.name.includes("-") ? a.name.split("-")[0] : "";
			if (!stacks.has(key)) stacks.set(key, []);
			stacks.get(key).push(a);
		}
		const rows = [...stacks.entries()].flatMap(([stack, list]) => [
			h_flex()
				.px(14)
				.pt(10)
				.pb(3)
				.text_size(11)
				.font_semibold()
				.text_color(t.dim)
				.child(stack ? stack.toUpperCase() : "AGENTS"),
			...list.map((a) => this.row(a, t)),
		]);
		const detail = Boolean(this.selected) && !this.spawn;
		return v_flex()
			.id("tower-agents")
			.size_full()
			.min_h(0)
			.text_size(13)
			.child(this.header(t))
			.when(Boolean(this.error ?? this.loadError), (el) =>
				el.child(
					div()
						.px(14)
						.py(6)
						.text_size(12)
						.text_color(t.ansi1)
						.child(this.error ?? this.loadError),
				),
			)
			.child(
				div()
					.id("tower-agents-body")
					// With the detail open, the list keeps a third and the
					// detail (and its screen) takes the rest.
					.when(detail, (el) => el.h("34%").flex_none())
					.when(!detail, (el) => el.flex_1())
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						this.spawn
							? this.spawnForm(t)
							: v_flex()
									.pb(10)
									.when(this.agents === null, (el) =>
										el.child(
											div()
												.px(14)
												.py(8)
												.text_size(12)
												.text_color(t.dim)
												.child("Loading…"),
										),
									)
									.when(this.agents !== null && agents.length === 0, (el) =>
										el.child(
											div()
												.px(14)
												.py(8)
												.text_size(12)
												.text_color(t.dim)
												.child("No agents yet. Spawn one."),
										),
									)
									.children(rows),
					),
			)
			.when(detail, (el) => el.child(this.details(t)));
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
			.child(div().font_semibold().child("Agents"))
			.child(
				div()
					.text_size(11)
					.text_color(t.dim)
					.child(`${this.agents?.length ?? 0}`),
			)
			.child(div().flex_1())
			.when(!this.spawn, (el) =>
				el.child(
					this.button("spawn", "＋ Spawn…", t, (cx) => this.openSpawn(cx)),
				),
			);
	}
}
