// The job queue: open jobs (state dots, owner, priority, age), closed ones
// below, and the schedules that feed it. Reads: `tower task list --json`,
// `tower schedule list --json`. Acts: create (a form), assign (a picker of
// idle agents; cancel interrupts the owner), cancel, release, and the
// schedule verbs (pause / resume / run now). Destructive acts confirm.

import { h_flex, Input, InputState, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { theme } from "herdr";
import { ago, stateColor, tower, until } from "./tower.js";

const OPEN = ["queued", "assigned", "working", "input-required"];
const CLOSED = ["completed", "failed", "canceled", "rejected"];

export default class TowerJobs extends View {
	init(_props, cx) {
		this.tasks = null;
		this.schedules = null;
		this.agents = null;
		this.error = null;
		this.busy = null; // task/schedule id with an action in flight
		this.confirm = null; // id awaiting a second press
		this.create = null; // the create form's fields
		this.assigning = null; // task id with the assign picker open
		this.agentMap = null; // id -> name
		cx.timer.every(10000, (cx) => this.refresh(cx));
		this.refresh(cx);
	}

	async refresh(cx) {
		try {
			const [tasks, schedules, ps] = await Promise.all([
				tower("task", "list"),
				tower("schedule", "list"),
				tower("ps"),
			]);
			this.tasks = tasks.tasks ?? [];
			this.schedules = schedules.schedules ?? [];
			this.agents = ps.agents ?? [];
			this.agentMap = Object.fromEntries(
				this.agents.map((a) => [a.id, a.name]),
			);
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
				opts.solid ? t.solid_foreground : opts.danger ? t.ansi1 : opts.muted,
			)
			.border_1()
			.border_color(opts.solid ? t.solid : t.border)
			.cursor_pointer()
			.hover((s) => s.opacity(0.8))
			.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
			.on_click((_e, cx) => onClick(cx))
			.child(label);
	}

	act(cx, id, args, confirmFirst, after) {
		if (confirmFirst && this.confirm !== id) {
			this.confirm = id;
			cx.notify();
			return;
		}
		this.confirm = null;
		this.busy = id;
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				await tower(...args);
				this.error = null;
				after?.();
			} catch (e) {
				this.error = e.message;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	openCreate(cx) {
		const field = (placeholder, value) =>
			InputState.new({ placeholder, value: value ?? "" });
		this.create = {
			title: field("A short job title"),
			description: field("Goal, inputs, limits, done criteria, what to report"),
			priority: "0",
			agent: null,
			when_available: false,
		};
		cx.notify();
	}

	submitCreate(cx) {
		const f = this.create;
		const title = f.title.value().trim();
		if (!title) {
			this.error = "a job needs a title";
			cx.notify();
			return;
		}
		this.busy = "create";
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				const args = ["task", "create", title];
				if (f.description.value().trim())
					args.push("--description", f.description.value().trim());
				const p = Number.parseInt(f.priority, 10);
				if (Number.isFinite(p) && p !== 0) args.push("--priority", String(p));
				if (f.agent) {
					args.push("--assign", f.agent);
					if (f.when_available) args.push("--when-available");
				}
				await tower(...args);
				this.create = null;
				this.error = null;
			} catch (e) {
				this.error = e.message;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	taskRow(task, t) {
		const c = stateColor(task.state);
		const owner = task.owner_id
			? (this.agentMap?.[task.owner_id] ?? "?")
			: null;
		const target = task.target_agent_id
			? (this.agentMap?.[task.target_agent_id] ?? "?")
			: null;
		const held =
			task.not_before && task.state === "queued"
				? until(task.not_before)
				: null;
		const tags = (task.tags ?? []).filter((x) => x !== "");
		const assigning = this.assigning === task.id;
		return v_flex()
			.id(`task-${task.id}`)
			.mx(8)
			.my(4)
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
						div()
							.flex_1()
							.min_w(0)
							.truncate()
							.font_semibold()
							.child(task.title),
					)
					.when(Boolean(task.priority), (el) =>
						el.child(
							div().text_size(11).text_color(t.dim).child(`P${task.priority}`),
						),
					)
					.child(
						div()
							.text_size(11)
							.text_color(t[c])
							.child(this.busy === task.id ? "…" : task.state),
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
							.child(
								[
									owner
										? `→ ${owner}`
										: target
											? `for ${target}${task.state === "queued" ? " (held)" : ""}`
											: null,
									held ? `starts ${held}` : null,
									`created ${ago(task.created_at)}`,
									tags.length ? `#${tags.join(" #")}` : null,
								]
									.filter(Boolean)
									.join(" · "),
							),
					)
					.when(task.state === "queued", (el) =>
						el
							.child(
								this.button(`a-${task.id}`, "Assign…", t, (cx) => {
									this.assigning = assigning ? null : task.id;
									cx.notify();
								}),
							)
							.child(
								this.button(
									`c-${task.id}`,
									this.confirm === task.id ? "Cancel?" : "Cancel",
									t,
									(cx) =>
										this.act(cx, task.id, ["task", "cancel", task.id], true),
									{ danger: true },
								),
							),
					)
					.when(OPEN.slice(1).includes(task.state) && owner, (el) =>
						el
							.child(
								this.button(`r-${task.id}`, "Release", t, (cx) =>
									this.act(cx, task.id, ["task", "release", task.id]),
								),
							)
							.child(
								this.button(
									`c-${task.id}`,
									this.confirm === task.id ? "Cancel?" : "Cancel",
									t,
									(cx) =>
										this.act(cx, task.id, ["task", "cancel", task.id], true),
									{ danger: true },
								),
							),
					),
			)
			.when(assigning, (el) =>
				el.child(
					h_flex()
						.id(`assign-${task.id}`)
						.pl(15)
						.pt(2)
						.gap(5)
						.flex_wrap()
						.children(
							this.agents.map((a) =>
								h_flex()
									.id(`pick-${a.id}`)
									.h(24)
									.px(8)
									.items_center()
									.gap(6)
									.rounded(6)
									.text_size(12)
									.cursor_pointer()
									.bg(t.raised)
									.border_1()
									.border_color(t.border)
									.hover((s) => s.bg(t.hover))
									.on_click((_e, cx) => {
										this.assigning = null;
										this.act(cx, task.id, ["task", "assign", task.id, a.name]);
									})
									.child(div().size(6).rounded(3).bg(t[stateColor(a.state)]))
									.child(a.name),
							),
						),
				),
			);
	}

	scheduleRow(s, t) {
		const paused = !s.enabled;
		return v_flex()
			.id(`sched-${s.id}`)
			.mx(8)
			.my(4)
			.px(8)
			.py(6)
			.gap(4)
			.rounded(7)
			.cursor_pointer()
			.hover((s2) => s2.bg(t.hover))
			.child(
				h_flex()
					.gap(7)
					.items_center()
					.child(
						div()
							.size(8)
							.rounded(4)
							.flex_none()
							.bg(paused ? t.dim : t.working),
					)
					.child(
						div().flex_1().min_w(0).truncate().font_semibold().child(s.title),
					)
					.child(
						div()
							.text_size(11)
							.text_color(paused ? t.dim : t.muted)
							.child(paused ? "paused" : s.cron),
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
							.child(
								[
									s.target_agent_id
										? `for ${this.agentMap?.[s.target_agent_id] ?? "?"}`
										: "general queue",
									s.next_run_at && !paused
										? `next ${until(s.next_run_at)}`
										: null,
								]
									.filter(Boolean)
									.join(" · "),
							),
					)
					.when(!paused, (el) =>
						el.child(
							this.button(`run-${s.id}`, "Run now", t, (cx) =>
								this.act(cx, s.id, ["schedule", "run", s.id]),
							),
						),
					)
					.child(
						this.button(`pause-${s.id}`, paused ? "Resume" : "Pause", t, (cx) =>
							this.act(
								cx,
								s.id,
								paused
									? ["schedule", "resume", s.id]
									: ["schedule", "pause", s.id],
							),
						),
					)
					.child(
						this.button(
							`rm-${s.id}`,
							this.confirm === s.id ? "Remove?" : "Remove",
							t,
							(cx) => this.act(cx, s.id, ["schedule", "rm", s.id], true),
							{ danger: true },
						),
					),
			);
	}

	createForm(t) {
		const f = this.create;
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
		const label = (s) =>
			div().pt(6).text_size(11).font_semibold().text_color(t.dim).child(s);
		return v_flex()
			.p(14)
			.gap(4)
			.child(div().text_size(15).font_semibold().child("Queue a job"))
			.child(label("TITLE"))
			.child(fieldBox("cf-title", f.title))
			.child(label("DESCRIPTION (the brief the agent works from)"))
			.child(fieldBox("cf-desc", f.description))
			.child(label("PRIORITY (higher runs first)"))
			.child(this.priorityPills(f, t))
			.child(label("OWNER"))
			.child(this.agentPills(f, t))
			.when(Boolean(f.agent), (el) =>
				el.child(
					h_flex()
						.pt(4)
						.gap(6)
						.items_center()
						.cursor_pointer()
						.on_click((_e, cx) => {
							f.when_available = !f.when_available;
							cx.notify();
						})
						.child(
							div()
								.w(14)
								.h(14)
								.flex_none()
								.rounded(4)
								.border_1()
								.border_color(t.input)
								.when(f.when_available, (el2) => el2.bg(t.accent)),
						)
						.child(
							div()
								.text_size(12)
								.text_color(t.muted)
								.child("Deliver when the agent is free (reserve)"),
						),
				),
			)
			.child(
				h_flex()
					.pt(10)
					.gap(8)
					.justify_end()
					.child(
						this.button("cf-cancel", "Cancel", t, (cx) => {
							this.create = null;
							cx.notify();
						}),
					)
					.child(
						this.button(
							"cf-go",
							this.busy === "create" ? "Queueing…" : "Queue job",
							t,
							(cx) => this.submitCreate(cx),
							{ solid: true },
						),
					),
			);
	}

	priorityPills(f, t) {
		return h_flex()
			.pt(4)
			.gap(5)
			.children(
				[
					["0", "0"],
					["1", "1"],
					["2", "2"],
					["3", "3"],
					["5", "5"],
				].map(([v, l]) =>
					h_flex()
						.id(`pri-${v}`)
						.h(24)
						.px(8)
						.items_center()
						.rounded(6)
						.text_size(12)
						.cursor_pointer()
						.bg(f.priority === v ? t.selected : t.raised)
						.text_color(f.priority === v ? t.foreground : t.muted)
						.border_1()
						.border_color(f.priority === v ? t.accent : t.border)
						.hover((s) => s.bg(t.hover))
						.on_click((_e, cx) => {
							f.priority = v;
							cx.notify();
						})
						.child(l),
				),
			);
	}

	agentPills(f, t) {
		return h_flex()
			.pt(4)
			.gap(5)
			.flex_wrap()
			.child(
				h_flex()
					.id("pri-none")
					.h(24)
					.px(8)
					.items_center()
					.rounded(6)
					.text_size(12)
					.cursor_pointer()
					.bg(!f.agent ? t.selected : t.raised)
					.text_color(!f.agent ? t.foreground : t.muted)
					.border_1()
					.border_color(!f.agent ? t.accent : t.border)
					.hover((s) => s.bg(t.hover))
					.on_click((_e, cx) => {
						f.agent = null;
						f.when_available = false;
						cx.notify();
					})
					.child("queue"),
			)
			.children(
				(this.agents ?? []).map((a) =>
					h_flex()
						.id(`own-${a.id}`)
						.h(24)
						.px(8)
						.items_center()
						.gap(6)
						.rounded(6)
						.text_size(12)
						.cursor_pointer()
						.bg(f.agent === a.name ? t.selected : t.raised)
						.text_color(f.agent === a.name ? t.foreground : t.muted)
						.border_1()
						.border_color(f.agent === a.name ? t.accent : t.border)
						.hover((s) => s.bg(t.hover))
						.on_click((_e, cx) => {
							f.agent = f.agent === a.name ? null : a.name;
							cx.notify();
						})
						.child(div().size(6).rounded(3).bg(t[stateColor(a.state)]))
						.child(a.name),
				),
			);
	}

	render() {
		const t = theme();
		const tasks = this.tasks ?? [];
		const open = tasks.filter((x) => OPEN.includes(x.state));
		const closed = tasks.filter((x) => CLOSED.includes(x.state)).slice(0, 10);
		return v_flex()
			.id("tower-jobs")
			.size_full()
			.min_h(0)
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
					.child(div().font_semibold().child("Jobs"))
					.child(div().flex_1())
					.child(
						div()
							.text_size(11)
							.text_color(t.dim)
							.child(
								`${open.length} open · ${this.schedules?.length ?? 0} schedules`,
							),
					),
			)
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
					.id("tower-jobs-body")
					.flex_1()
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						this.create
							? this.createForm(t)
							: v_flex()
									.pb(10)
									.when(this.tasks === null, (el) =>
										el.child(
											div()
												.px(14)
												.py(8)
												.text_size(12)
												.text_color(t.dim)
												.child("Loading…"),
										),
									)
									.child(this.section("QUEUE", open.length, t))
									.when(this.tasks !== null && open.length === 0, (el) =>
										el.child(
											div()
												.px(14)
												.py(4)
												.text_size(12)
												.text_color(t.dim)
												.child("Queue is empty."),
										),
									)
									.children(open.map((x) => this.taskRow(x, t)))
									.when(closed.length > 0, (el) =>
										el
											.child(this.section("RECENT", closed.length, t))
											.children(closed.map((x) => this.taskRow(x, t))),
									)
									.child(
										this.section("SCHEDULES", this.schedules?.length ?? 0, t),
									)
									.when(
										(this.schedules ?? []).length === 0 && this.tasks !== null,
										(el) =>
											el.child(
												div()
													.px(14)
													.py(4)
													.text_size(12)
													.text_color(t.dim)
													.child("None."),
											),
									)
									.children(
										(this.schedules ?? []).map((s) => this.scheduleRow(s, t)),
									),
					),
			)
			.when(!this.create, (el) =>
				el.child(
					h_flex()
						.h(38)
						.flex_none()
						.px(14)
						.gap(8)
						.items_center()
						.border_t(1)
						.border_color(t.border)
						.child(
							this.button(
								"new-job",
								"＋ Queue a job…",
								t,
								(cx) => this.openCreate(cx),
								{ solid: true, h: 24, px: 8 },
							),
						),
				),
			);
	}

	section(label, n, t) {
		return h_flex()
			.px(14)
			.pt(12)
			.pb(4)
			.gap(6)
			.items_center()
			.text_size(11)
			.font_semibold()
			.text_color(t.dim)
			.child(label)
			.when(n > 0, (el) =>
				el.child(
					div().px(5).rounded(4).bg(t.raised).text_size(10).child(String(n)),
				),
			);
	}
}
