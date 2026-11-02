// The tower fleet: every agent seat, its live state, and the queue it works.
// One section per stack (agents named <stack>-<role>), unstacked agents last.
// Reads: `tower ps --json`. Acts: prompt (an inline bar per row), interrupt,
// stop (two presses), spawn (a form of installed harnesses).

import { h_flex, Input, InputState, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { knownAgents, theme } from "herdr";
import { stateColor, tower } from "./tower.js";

export default class TowerAgents extends View {
	init(_props, cx) {
		this.agents = null;
		this.error = null;
		this.busy = null; // agent name with an action in flight
		this.confirm = null; // agent name awaiting a second press
		this.promptFor = null; // agent name with the prompt bar open
		this.spawn = null; // the spawn form's fields
		cx.timer.every(10000, (cx) => this.refresh(cx));
		this.refresh(cx);
	}

	async refresh(cx) {
		try {
			const v = await tower("ps");
			this.agents = v.agents ?? [];
			this.error = null;
		} catch (e) {
			this.error = String(e.message ?? e);
		}
		cx.notify();
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

	act(cx, name, args, confirmFirst) {
		if (confirmFirst && this.confirm !== name) {
			this.confirm = name;
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

	/** Open the inline prompt bar for an agent (InputState lives in handlers). */
	openPrompt(a, cx) {
		if (this.promptFor === a.name) return;
		const state = InputState.new({ placeholder: `Prompt ${a.name}…` });
		state.on("submit", (_e, cx2) => {
			const text = state.value().trim();
			if (!text) return;
			this.promptFor = null;
			cx2.spawn(async (cx2) => {
				try {
					await tower("prompt", a.name, text);
					this.error = null;
				} catch (e) {
					this.error = `${a.name}: ${e.message}`;
				}
				await this.refresh(cx2);
			});
		});
		this.promptState = state;
		this.promptFor = a.name;
		state.focus();
		cx.notify();
	}

	/** The spawn form: name, harness pills, workdir, first prompt. */
	openSpawn(cx) {
		const field = (placeholder, value) => {
			const s = InputState.new({ placeholder, value: value ?? "" });
			return s;
		};
		this.spawn = {
			name: field("reviewer"),
			kind: null,
			installed: null,
			def: false,
			workdir: field("/home/you/project"),
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

	promptBar(a, t) {
		return h_flex()
			.id(`prompt-bar-${a.name}`)
			.pl(15)
			.pt(2)
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
					.child(Input.new(this.promptState).w_full().h_full().text_size(12)),
			)
			.child(
				this.button(
					`psend-${a.name}`,
					"Send",
					t,
					(cx) => {
						const text = this.promptState.value().trim();
						if (!text) return;
						this.promptFor = null;
						cx.spawn(async (cx) => {
							try {
								await tower("prompt", a.name, text);
								this.error = null;
							} catch (e) {
								this.error = `${a.name}: ${e.message}`;
							}
							await this.refresh(cx);
						});
					},
					{ solid: true },
				),
			)
			.child(
				this.button(`pclose-${a.name}`, "✕", t, (cx) => {
					this.promptFor = null;
					cx.notify();
				}),
			);
	}

	row(a, t) {
		const c = stateColor(a.state);
		const busy = this.busy === a.name;
		return v_flex()
			.id(`agent-${a.name}`)
			.mx(8)
			.px(8)
			.py(6)
			.gap(4)
			.rounded(7)
			.cursor_pointer()
			.hover((s) => s.bg(t.hover))
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
							.text_size(11)
							.text_color(t[c])
							.child(busy ? "…" : a.state),
					),
			)
			.child(
				h_flex()
					.pl(15)
					.gap(6)
					.items_center()
					.child(
						div()
							.flex_1()
							.min_w(0)
							.truncate()
							.text_size(11)
							.text_color(t.muted)
							.child(a.workdir ?? a.worktree ?? a.kind),
					)
					.child(
						this.button(`p-${a.name}`, "Prompt", t, (cx) =>
							this.openPrompt(a, cx),
						),
					)
					.child(
						this.button(`i-${a.name}`, "Interrupt", t, (cx) =>
							this.act(cx, a.name, ["interrupt", a.name]),
						),
					)
					.child(
						this.button(
							`s-${a.name}`,
							this.confirm === a.name ? "Stop?" : "Stop",
							t,
							(cx) => this.act(cx, a.name, ["stop", a.name], true),
							{ danger: true },
						),
					),
			)
			.when(this.promptFor === a.name, (el) => el.child(this.promptBar(a, t)));
	}

	spawnForm(t) {
		const f = this.spawn;
		const pills = (options, value, pick) =>
			h_flex()
				.flex_wrap()
				.gap(5)
				.children(
					options.map(([v, _label]) =>
						h_flex()
							.id(`k-${v}`)
							.h(24)
							.px(8)
							.items_center()
							.rounded(6)
							.text_size(12)
							.cursor_pointer()
							.bg(v === value ? t.selected : t.raised)
							.text_color(v === value ? t.foreground : t.muted)
							.border_1()
							.border_color(v === value ? t.accent : t.border)
							.hover((s) => s.bg(t.hover))
							.on_click((_e, cx) => pick(v, cx)),
					),
				);
		const fieldBox = (id, state, extra = []) =>
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
				.child(Input.new(state).h_full().w_full().text_size(12))
				.children(extra);
		return v_flex()
			.p(14)
			.gap(6)
			.child(div().text_size(15).font_semibold().child("Spawn a tower agent"))
			.child(
				div()
					.pt(4)
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child("NAME"),
			)
			.child(fieldBox("sf-name", f.name))
			.child(
				div()
					.pt(4)
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child("HARNESS"),
			)
			.child(
				pills(f.installed?.map((k) => [k, k]) ?? [], f.kind, (v, cx) => {
					f.kind = v;
					cx.notify();
				}).when(!f.installed, (el) =>
					el.child(div().text_size(12).text_color(t.dim).child("loading…")),
				),
			)
			.child(
				div()
					.pt(4)
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child("WORKDIR"),
			)
			.child(fieldBox("sf-dir", f.workdir))
			.child(
				div()
					.pt(4)
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child("FIRST PROMPT"),
			)
			.child(fieldBox("sf-task", f.prompt))
			.child(
				h_flex()
					.pt(8)
					.gap(8)
					.justify_end()
					.child(
						this.button("sf-cancel", "Cancel", t, (cx) => {
							this.spawn = null;
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

	render() {
		const t = theme();
		const agents = this.agents ?? [];
		const stacks = new Map();
		for (const a of agents) {
			const stack = a.name.includes("-") ? a.name.split("-")[0] : null;
			const key = stack ?? "";
			if (!stacks.has(key)) stacks.set(key, []);
			stacks.get(key).push(a);
		}
		const rows = [...stacks.entries()].flatMap(([stack, list]) => {
			const label = stack ? stack.toUpperCase() : "AGENTS";
			return [
				h_flex()
					.px(14)
					.pt(12)
					.pb(4)
					.gap(6)
					.items_center()
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child(label),
				...list.map((a) => this.row(a, t)),
			];
		});
		return v_flex()
			.id("tower-agents")
			.size_full()
			.min_h(0)
			.text_size(13)
			.child(this.header(t))
			.when(Boolean(this.error), (el) =>
				el.child(
					div()
						.px(14)
						.py(6)
						.text_size(12)
						.text_color(t.ansi1)
						.child(this.error),
				),
			)
			.child(
				div()
					.id("tower-agents-body")
					.flex_1()
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
												.child("No agents yet. Spawn one below."),
										),
									)
									.children(rows),
					),
			)
			.when(!this.spawn, (el) => el.child(this.footer(t)));
	}

	header(t) {
		return h_flex()
			.h(40)
			.flex_none()
			.px(14)
			.gap(8)
			.items_center()
			.border_b(1)
			.border_color(t.border)
			.child(div().font_semibold().child("Agents"))
			.child(div().flex_1())
			.child(
				div()
					.text_size(11)
					.text_color(t.dim)
					.child(`${this.agents?.length ?? 0} live`),
			);
	}

	footer(t) {
		return h_flex()
			.h(38)
			.flex_none()
			.px(14)
			.gap(8)
			.items_center()
			.border_t(1)
			.border_color(t.border)
			.child(
				this.button("spawn", "＋ Spawn…", t, (cx) => this.openSpawn(cx), {
					solid: true,
				}),
			);
	}
}
