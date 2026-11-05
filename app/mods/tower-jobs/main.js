// The job queue: open jobs (state dots, owner, priority, age), closed ones
// below, and the schedules that feed it. Reads: `tower task list --json`,
// `tower schedule list --json`. Acts: create (a form), assign (a picker of
// idle agents; cancel interrupts the owner), cancel, release, and the
// schedule verbs (pause / resume / run now). Destructive acts confirm.
//
// Clicking a job or schedule opens the detail section below the list (the
// shape of the agents panel): its facts and actions, then a job's prompts,
// result, trail and messages (`tower task show`) or a schedule's prompts
// and runs (`tower schedule show`; a run opens as a job). Edit replaces
// the body with a form (`tower task edit` / `tower schedule edit`).

import {
	h_flex,
	Input,
	InputState,
	Textarea,
	TextareaState,
	v_flex,
} from "gpui-base";
import { div, View } from "gpui-kit";
import { theme } from "herdr";
import {
	CLOSED,
	createArgs,
	editArgs,
	editFields,
	hookText,
	OPEN,
	resultFields,
	resultSummary,
	stamp,
	trailLine,
	waitingFor,
	who,
} from "./jobs.js";
import { ago, messageText, stateColor, tower, until } from "./tower.js";

export default class TowerJobs extends View {
	init(_props, cx) {
		this.tasks = null;
		this.schedules = null;
		this.agents = null;
		this.error = null; // the last action's failure, kept until the next action
		this.loadError = null; // the last poll's failure
		this.busy = null; // task/schedule id with an action in flight
		this.confirm = null; // id awaiting a second press
		this.create = null; // the create form's fields
		this.assigning = null; // task id with the assign picker open
		this.agentMap = null; // id -> name
		this.sel = null; // { kind: "job" | "schedule", id, from } in the detail
		this.detail = null; // its `show` answer
		this.detailError = null;
		this.edit = null; // the edit form: { kind, id, fields, states }
		this.expanded = new Set(); // message ids shown in full
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
			this.loadError = null;
		} catch (e) {
			this.loadError = String(e.message ?? e);
		}
		if (this.sel) await this.loadDetail();
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

	/** Is this job / schedule the one in the detail section? */
	isSel(kind, id) {
		return this.sel?.kind === kind && this.sel.id === id;
	}

	/** Open a job or schedule in the detail section; the same one again
	 * closes it. `from`: the schedule a run was opened from (a way back). */
	select(kind, id, cx, from = null) {
		this.error = null;
		this.closeEdit();
		this.assigning = null;
		this.confirm = null;
		this.expanded = new Set();
		this.detail = null;
		this.detailError = null;
		if (!kind || this.isSel(kind, id)) {
			this.sel = null;
			return;
		}
		this.sel = { kind, id, from };
		cx?.spawn(async (cx) => {
			await this.loadDetail();
			cx.notify();
		});
	}

	/** `tower task show` / `tower schedule show` for the selection. A late
	 * answer for an earlier selection is dropped. */
	async loadDetail() {
		const sel = this.sel;
		if (!sel) return;
		try {
			const d = await tower(
				sel.kind === "job" ? "task" : "schedule",
				"show",
				sel.id,
			);
			if (this.sel !== sel) return;
			this.detail = d;
			this.detailError = null;
		} catch (e) {
			if (this.sel !== sel) return;
			this.detailError = String(e.message ?? e);
		}
	}

	/** The selected job or schedule: the list row, else the detail's copy
	 * (a run older than the list). */
	selObj() {
		const s = this.sel;
		if (!s) return null;
		if (s.kind === "job") {
			return (
				(this.tasks ?? []).find((x) => x.id === s.id) ??
				(this.detail?.task?.id === s.id ? this.detail.task : null)
			);
		}
		return (
			(this.schedules ?? []).find((x) => x.id === s.id) ??
			(this.detail?.schedule?.id === s.id ? this.detail.schedule : null)
		);
	}

	/** Agent and schedule names for the ids jobs, events and messages carry. */
	names() {
		return {
			agents: this.agentMap ?? {},
			schedules: Object.fromEntries(
				(this.schedules ?? []).map((s) => [s.id, s.title]),
			),
		};
	}

	/** The edit form over the detail body: one field per editable value,
	 * prefilled; locked ones (a delivered job's) shown dim. */
	openEdit(cx) {
		const obj = this.selObj();
		const fields = obj ? editFields(this.sel.kind, obj) : null;
		if (!fields) return;
		const states = {};
		for (const f of fields) {
			if (f.locked) continue;
			if (f.multi) {
				const s = TextareaState.new({ value: f.value });
				s.set_auto_grow(3, 12);
				states[f.key] = s;
			} else {
				states[f.key] = InputState.new({ value: f.value });
			}
		}
		this.edit = { kind: this.sel.kind, id: this.sel.id, fields, states };
		cx.notify();
	}

	closeEdit() {
		for (const s of Object.values(this.edit?.states ?? {})) s.release();
		this.edit = null;
	}

	/** Save the fields that changed in one `edit` call; nothing changed
	 * just closes the form. */
	saveEdit(cx) {
		const e = this.edit;
		const values = Object.fromEntries(
			Object.entries(e.states).map(([k, s]) => [k, s.value()]),
		);
		let args;
		try {
			args = editArgs(e.kind, e.id, e.fields, values);
		} catch (err) {
			this.error = err.message;
			cx.notify();
			return;
		}
		if (!args) {
			this.closeEdit();
			cx.notify();
			return;
		}
		this.act(cx, e.id, args, false, () => this.closeEdit());
	}

	/** The create form: a job now, a job held for its owner until a time,
	 * or a schedule (one form; `createArgs` picks the verb). */
	openCreate(cx) {
		this.error = null;
		const field = (placeholder) => InputState.new({ placeholder, value: "" });
		const prompt = (placeholder) => {
			const s = TextareaState.new({ placeholder });
			s.set_auto_grow(1, 6);
			return s;
		};
		this.create = {
			title: field("A short job title"),
			description: field("Goal, inputs, limits, done criteria, what to report"),
			priority: "0",
			agent: null,
			when_available: false,
			when: "now",
			at: field("HH:MM, or YYYY-MM-DD HH:MM"),
			cadence: "daily",
			cadence_value: field("09:00"),
			timezone: field("blank: this machine's zone (e.g. America/Vancouver)"),
			pre_run: prompt("Typed before the job prompt, e.g. /clear"),
			post_run: prompt("Typed after the owner closes the job, e.g. /reflect"),
		};
		cx.notify();
	}

	closeCreate() {
		const f = this.create;
		if (!f) return;
		for (const k of ["title", "description", "at", "cadence_value", "timezone", "pre_run", "post_run"]) {
			f[k].release();
		}
		this.create = null;
	}

	submitCreate(cx) {
		const f = this.create;
		const text = (k) => f[k].value();
		let args;
		try {
			args = createArgs({
				title: text("title"),
				description: text("description"),
				priority: f.priority,
				agent: f.agent,
				when_available: f.when_available,
				when: f.when,
				at: text("at"),
				cadence: f.cadence,
				cadence_value: text("cadence_value"),
				timezone: text("timezone"),
				pre_run: text("pre_run"),
				post_run: text("post_run"),
			});
		} catch (e) {
			this.error = e.message;
			cx.notify();
			return;
		}
		this.busy = "create";
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				await tower(...args);
				this.closeCreate();
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
			.when(this.isSel("job", task.id), (el) => el.bg(t.selected))
			.hover((s) => s.bg(t.hover))
			.on_click((_e, cx) => {
				this.select("job", task.id, cx);
				cx.notify();
			})
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
			.when(assigning && !this.isSel("job", task.id), (el) =>
				el.child(this.assignPicker(task, t).pl(15).pt(2)),
			);
	}

	/** The agents to assign a queued job to, as pills. */
	assignPicker(task, t) {
		return h_flex()
			.id(`assign-${task.id}`)
			.gap(5)
			.flex_wrap()
			.children(
				(this.agents ?? []).map((a) =>
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
						.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
						.on_click((_e, cx) => {
							this.assigning = null;
							this.act(cx, task.id, ["task", "assign", task.id, a.name]);
						})
						.child(div().size(6).rounded(3).bg(t[stateColor(a.state)]))
						.child(a.name),
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
			.when(this.isSel("schedule", s.id), (el) => el.bg(t.selected))
			.hover((s2) => s2.bg(t.hover))
			.on_click((_e, cx) => {
				this.select("schedule", s.id, cx);
				cx.notify();
			})
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
							(cx) =>
								this.act(cx, s.id, ["schedule", "rm", s.id], true, () => {
									if (this.isSel("schedule", s.id)) this.select(null);
								}),
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
		const promptBox = (id, state) =>
			div()
				.id(id)
				.px(9)
				.py(5)
				.rounded(7)
				.border_1()
				.border_color(t.input)
				.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
				.child(Textarea.new(state).w_full().text_size(12));
		const label = (s) =>
			div().pt(6).text_size(11).font_semibold().text_color(t.dim).child(s);
		const note = (s) => div().pt(2).text_size(11).text_color(t.dim).child(s);
		const repeat = f.when === "repeat";
		const submit = {
			now: ["Queue job", "Queueing…"],
			at: ["Reserve job", "Reserving…"],
			repeat: ["Create schedule", "Creating…"],
		}[f.when];
		return v_flex()
			.p(14)
			.gap(4)
			.child(div().text_size(15).font_semibold().child("New job"))
			.child(label("TITLE"))
			.child(fieldBox("cf-title", f.title))
			.child(label("DESCRIPTION (the brief the agent works from)"))
			.child(fieldBox("cf-desc", f.description))
			.child(label("PRIORITY (higher runs first)"))
			.child(this.priorityPills(f, t))
			.child(label("OWNER"))
			.child(this.agentPills(f, t))
			.when(Boolean(f.agent) && f.when === "now", (el) =>
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
			.child(label("WHEN"))
			.child(
				this.choicePills(
					"cf-when",
					[
						["now", "now"],
						["at", "at a time"],
						["repeat", "repeats"],
					],
					f.when,
					(v) => {
						f.when = v;
					},
					t,
				),
			)
			.when(f.when === "at", (el) =>
				el
					.child(label("AT"))
					.child(fieldBox("cf-at", f.at))
					.child(
						note(
							f.agent
								? `Reserved for ${f.agent}; delivered once the time has passed and it is free.`
								: "A held job needs an owner: pick one above.",
						),
					),
			)
			.when(repeat, (el) =>
				el
					.child(label("CADENCE"))
					.child(
						this.choicePills(
							"cf-cadence",
							[
								["daily", "daily at"],
								["cron", "cron"],
							],
							f.cadence,
							(v) => {
								f.cadence = v;
								f.cadence_value.set_value("");
							},
							t,
						),
					)
					.child(fieldBox("cf-cadence-value", f.cadence_value))
					.child(
						note(
							f.cadence === "cron"
								? "5 or 6 fields, e.g. 0 9 * * MON-FRI"
								: "HH:MM, every day",
						),
					)
					.child(label("TIMEZONE (IANA)"))
					.child(fieldBox("cf-tz", f.timezone))
					.child(
						note(
							f.agent
								? `Each firing makes a job reserved for ${f.agent}.`
								: "Each firing queues a job for you to assign.",
						),
					),
			)
			.child(label(repeat ? "PRE-RUN (each job)" : "PRE-RUN"))
			.child(promptBox("cf-pre", f.pre_run))
			.child(label(repeat ? "POST-RUN (each job)" : "POST-RUN"))
			.child(promptBox("cf-post", f.post_run))
			.child(
				h_flex()
					.pt(10)
					.gap(8)
					.justify_end()
					.child(
						this.button("cf-cancel", "Cancel", t, (cx) => {
							this.closeCreate();
							this.error = null;
							cx.notify();
						}),
					)
					.child(
						this.button(
							"cf-go",
							this.busy === "create" ? submit[1] : submit[0],
							t,
							(cx) => this.submitCreate(cx),
							{ solid: true },
						),
					),
			);
	}

	/** One choice out of a few, as pills. */
	choicePills(id, options, current, pick, t) {
		return h_flex()
			.pt(4)
			.gap(5)
			.children(
				options.map(([v, l]) =>
					h_flex()
						.id(`${id}-${v}`)
						.h(24)
						.px(8)
						.items_center()
						.rounded(6)
						.text_size(12)
						.cursor_pointer()
						.bg(current === v ? t.selected : t.raised)
						.text_color(current === v ? t.foreground : t.muted)
						.border_1()
						.border_color(current === v ? t.accent : t.border)
						.hover((s) => s.bg(t.hover))
						.on_click((_e, cx) => {
							pick(v);
							cx.notify();
						})
						.child(l),
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
		// `task list` orders by priority then age (oldest first): recent
		// means the last ten to close
		const closed = tasks
			.filter((x) => CLOSED.includes(x.state))
			.sort((a, b) => b.updated_at - a.updated_at)
			.slice(0, 10);
		const detail = Boolean(this.sel) && !this.create;
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
					.id("tower-jobs-body")
					// With the detail open, the list keeps a third and the
					// detail takes the rest.
					.when(detail, (el) => el.h("34%").flex_none())
					.when(!detail, (el) => el.flex_1())
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
			.when(detail, (el) => el.child(this.details(t)))
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
								"＋ New job…",
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

	// --- the detail section ---------------------------------------------

	/** The detail section: header, actions, facts, then the body (or the
	 * edit form). */
	details(t) {
		const s = this.sel;
		const obj = this.selObj();
		const shell = v_flex()
			.id("jb-detail")
			.flex_1()
			.min_h(0)
			.border_t(1)
			.border_color(t.border);
		if (!obj) {
			return shell.child(
				h_flex()
					.px(14)
					.py(8)
					.gap(8)
					.text_size(12)
					.child(
						div()
							.flex_1()
							.text_color(this.detailError ? t.ansi1 : t.dim)
							.child(this.detailError ?? "Loading…"),
					)
					.child(this.closeButton(t)),
			);
		}
		const job = s.kind === "job";
		const c = job ? stateColor(obj.state) : obj.enabled ? "working" : "dim";
		const state = job ? obj.state : obj.enabled ? "active" : "paused";
		const from = s.from
			? (this.names().schedules[s.from] ?? "schedule")
			: null;
		return shell
			.child(
				h_flex()
					.h(30)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.when(Boolean(from), (el) =>
						el.child(
							this.button("jd-back", `‹ ${from}`, t, (cx) => {
								this.select("schedule", s.from, cx);
								cx.notify();
							}),
						),
					)
					.child(div().size(8).rounded(4).flex_none().bg(t[c]))
					.child(
						div().flex_1().min_w(0).truncate().font_semibold().child(obj.title),
					)
					.child(
						div()
							.text_size(11)
							.text_color(t[c])
							.child(this.busy === obj.id ? "…" : state),
					)
					.child(this.closeButton(t)),
			)
			.child(job ? this.jobActions(obj, t) : this.scheduleActions(obj, t))
			.when(job && this.assigning === obj.id, (el) =>
				el.child(this.assignPicker(obj, t).flex_none().px(14).pb(8)),
			)
			.child(this.factsGrid(job ? this.jobFacts(obj) : this.scheduleFacts(obj), t))
			.child(
				div()
					.id("jb-detail-body")
					.flex_1()
					.min_h(0)
					.border_t(1)
					.border_color(t.border)
					.overflow_y_scrollbar()
					.child(
						this.edit
							? this.editForm(obj, t)
							: job
								? this.jobBody(obj, t)
								: this.scheduleBody(obj, t),
					),
			);
	}

	closeButton(t) {
		return div()
			.id("jd-close")
			.px(6)
			.rounded(4)
			.cursor_pointer()
			.text_color(t.dim)
			.hover((s) => s.bg(t.hover))
			.on_click((_e, cx) => {
				this.select(null);
				cx.notify();
			})
			.child("×");
	}

	actionRow() {
		return h_flex().flex_none().flex_wrap().px(14).pb(8).gap(6);
	}

	/** What a job can do in its state: assign / edit / cancel while queued,
	 * release / edit post-run / cancel while delivered. */
	jobActions(j, t) {
		const queued = j.state === "queued";
		const open = OPEN.includes(j.state);
		const owned = open && !queued && Boolean(j.owner_id);
		const schedule =
			j.schedule_id && !this.sel.from && this.names().schedules[j.schedule_id];
		return this.actionRow()
			.when(queued, (el) =>
				el.child(
					this.button(
						"jd-assign",
						this.assigning === j.id ? "Assign ×" : "Assign…",
						t,
						(cx) => {
							this.assigning = this.assigning === j.id ? null : j.id;
							cx.notify();
						},
					),
				),
			)
			.when(owned, (el) =>
				el.child(
					this.button("jd-release", "Release", t, (cx) =>
						this.act(cx, j.id, ["task", "release", j.id]),
					),
				),
			)
			.when(open && !this.edit, (el) =>
				el.child(
					this.button("jd-edit", queued ? "Edit" : "Edit post-run", t, (cx) =>
						this.openEdit(cx),
					),
				),
			)
			.when(open, (el) =>
				el.child(
					this.button(
						"jd-cancel",
						this.confirm === j.id ? "Cancel?" : "Cancel",
						t,
						(cx) => this.act(cx, j.id, ["task", "cancel", j.id], true),
						{ danger: true },
					),
				),
			)
			.when(Boolean(schedule), (el) =>
				el.child(
					this.button("jd-sched", "Open schedule", t, (cx) => {
						this.select("schedule", j.schedule_id, cx);
						cx.notify();
					}),
				),
			)
			.when(!open, (el) =>
				el.child(
					div().text_size(11).text_color(t.dim).child(`closed ${ago(j.updated_at)}`),
				),
			);
	}

	scheduleActions(s, t) {
		const paused = !s.enabled;
		return this.actionRow()
			.when(!paused, (el) =>
				el.child(
					this.button("sd-run", "Run now", t, (cx) =>
						this.act(cx, s.id, ["schedule", "run", s.id]),
					),
				),
			)
			.child(
				this.button("sd-pause", paused ? "Resume" : "Pause", t, (cx) =>
					this.act(cx, s.id, [
						"schedule",
						paused ? "resume" : "pause",
						s.id,
					]),
				),
			)
			.when(!this.edit, (el) =>
				el.child(this.button("sd-edit", "Edit", t, (cx) => this.openEdit(cx))),
			)
			.child(
				this.button(
					"sd-rm",
					this.confirm === s.id ? "Remove?" : "Remove",
					t,
					(cx) =>
						this.act(cx, s.id, ["schedule", "rm", s.id], true, () =>
							this.select(null),
						),
					{ danger: true },
				),
			);
	}

	/** [label, value, wide] rows: wide ones take a full row, the rest pair
	 * up two per row. */
	jobFacts(j) {
		const n = this.names();
		const open = OPEN.includes(j.state);
		const tags = (j.tags ?? []).filter((x) => x !== "");
		const rows = [];
		if (j.state === "queued") rows.push(["waiting", waitingFor(j, n), true]);
		rows.push(
			["owner", j.owner_id ? who(j.owner_id, n) : "-"],
			[
				"lease",
				open && j.lease_expires_at ? `ends ${until(j.lease_expires_at)}` : "-",
			],
			["attempts", `${j.attempt_count}/${j.max_attempts}`],
			["priority", String(j.priority)],
			["lease time", `${j.lease_s}s`],
			["origin", j.origin],
		);
		if (j.schedule_id) {
			rows.push(
				["schedule", n.schedules[j.schedule_id] ?? j.schedule_id],
				["occurrence", stamp(j.occurrence_at)],
			);
		}
		rows.push(
			["created", stamp(j.created_at)],
			["updated", stamp(j.updated_at)],
			["tags", tags.length ? tags.join(" ") : "-", true],
			["id", j.id, true],
		);
		return rows;
	}

	scheduleFacts(s) {
		const n = this.names();
		const tags = (s.tags ?? []).filter((x) => x !== "");
		return [
			["cadence", `${s.cron} · ${s.timezone}`, true],
			["target", s.target_agent_id ? who(s.target_agent_id, n) : "general queue"],
			[
				"next",
				s.enabled && s.next_run_at
					? `${stamp(s.next_run_at)} (${until(s.next_run_at)})`
					: "-",
			],
			["last", stamp(s.last_run_at)],
			["priority", String(s.priority)],
			["lease time", `${s.lease_s}s`],
			["attempts", String(s.max_attempts)],
			["created", stamp(s.created_at)],
			["updated", stamp(s.updated_at)],
			["tags", tags.length ? tags.join(" ") : "-", true],
			["id", s.id, true],
		];
	}

	factsGrid(rows, t) {
		return h_flex()
			.flex_none()
			.flex_wrap()
			.px(14)
			.pb(8)
			.text_size(12)
			.children(
				rows.map(([label, value, wide], i) =>
					h_flex()
						.id(`jd-fact-${i}`)
						.w(wide ? "100%" : "50%")
						.min_w(0)
						.py(1)
						.pr(8)
						.gap(8)
						.child(
							div()
								.w(76)
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
			);
	}

	/** A section label inside the detail body. */
	label(text, t, n) {
		return h_flex()
			.pt(10)
			.pb(3)
			.gap(6)
			.items_center()
			.text_size(11)
			.font_semibold()
			.text_color(t.dim)
			.child(text)
			.when(n > 0, (el) =>
				el.child(
					div().px(5).rounded(4).bg(t.raised).text_size(10).child(String(n)),
				),
			);
	}

	/** Wrapped text, one element per line so line breaks hold. */
	para(text, t, color) {
		return v_flex()
			.min_w(0)
			.text_size(12)
			.text_color(color ?? t.foreground)
			.children(
				String(text)
					.split("\n")
					.map((l) => div().min_w(0).child(l || " ")),
			);
	}

	/** A labeled prompt (job prompt, pre-run, post-run); none says so. */
	textSection(label, text, t) {
		return v_flex()
			.child(this.label(label, t))
			.child(text ? this.para(text, t) : this.para("(none)", t, t.dim));
	}

	/** A timestamped line: time column, then the text. */
	timeRow(ts, text, color, t) {
		return h_flex()
			.py(1)
			.gap(8)
			.items_start()
			.child(
				div()
					.w(84)
					.flex_none()
					.text_size(11)
					.text_color(t.dim)
					.child(stamp(ts)),
			)
			.child(
				div()
					.flex_1()
					.min_w(0)
					.text_size(12)
					.text_color(color)
					.child(text),
			);
	}

	/** A job's prompts, the hook in flight, its result, the trail of what
	 * happened to it, and the messages about it. */
	jobBody(j, t) {
		const d = this.detail?.task?.id === j.id ? this.detail : null;
		const n = this.names();
		const hook = hookText(j.hook);
		const gist = resultSummary(j.result);
		const trail = d?.trail ?? [];
		const messages = d?.messages ?? [];
		const kindColor = (k) =>
			k === "task.failed" || k === "task.leased_out"
				? t.ansi1
				: k === "task.completed"
					? t.done
					: t.foreground;
		return v_flex()
			.px(14)
			.pb(12)
			.child(this.textSection("PROMPT", j.description, t))
			.when(Boolean(j.pre_run), (el) =>
				el.child(this.textSection("PRE-RUN", j.pre_run, t)),
			)
			.when(Boolean(j.post_run), (el) =>
				el.child(this.textSection("POST-RUN", j.post_run, t)),
			)
			.when(Boolean(hook), (el) =>
				el.child(div().pt(8).text_size(12).text_color(t.blocked).child(hook)),
			)
			.when(j.result != null, (el) =>
				el
					.child(this.label("RESULT", t))
					.when(Boolean(gist), (el2) =>
						el2.child(
							this.para(gist, t, j.state === "failed" ? t.ansi1 : t.foreground),
						),
					)
					.children(
						resultFields(j.result).map(([k, v]) =>
							h_flex()
								.pt(3)
								.gap(8)
								.items_start()
								.child(
									div()
										.w(84)
										.flex_none()
										.text_size(11)
										.text_color(t.dim)
										.child(k),
								)
								.child(div().flex_1().min_w(0).child(this.para(v, t))),
						),
					),
			)
			.child(this.label("TRAIL", t, trail.length))
			.when(!d, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("Loading…")),
			)
			.when(Boolean(d) && trail.length === 0, (el) =>
				el.child(
					div()
						.text_size(12)
						.text_color(t.dim)
						.child("No events (older than the event log keeps)."),
				),
			)
			.children(
				trail.map((e) =>
					this.timeRow(e.ts, trailLine(e, n), kindColor(e.kind), t),
				),
			)
			.when(messages.length > 0, (el) =>
				el
					.child(this.label("MESSAGES", t, messages.length))
					.children(messages.map((m) => this.messageRow(m, n, t))),
			);
	}

	/** One message: when, who to whom, kind and status, then its first
	 * lines (at most 3, about 240 characters); a click shows it whole. */
	messageRow(m, n, t) {
		const text = messageText(m) || "(no text)";
		const short = text.split("\n").slice(0, 3).join("\n").slice(0, 240);
		const long = short.length < text.length;
		const full = this.expanded.has(m.id);
		return v_flex()
			.id(`msg-${m.id}`)
			.py(3)
			.gap(1)
			.when(long, (el) =>
				el.cursor_pointer().on_click((_e, cx) => {
					if (full) this.expanded.delete(m.id);
					else this.expanded.add(m.id);
					cx.notify();
				}),
			)
			.child(
				this.timeRow(
					m.created_at,
					`${who(m.from_id, n)} -> ${who(m.to_id, n)} · ${m.kind} · ${m.status}`,
					t.muted,
					t,
				),
			)
			.child(
				div()
					.pl(92)
					.child(this.para(full ? text : `${short}${long ? "…" : ""}`, t)),
			)
			.when(long, (el) =>
				el.child(
					div()
						.pl(92)
						.text_size(11)
						.text_color(t.accent)
						.child(full ? "show less" : "show all"),
				),
			);
	}

	/** A schedule's prompts and its runs, newest first; a run opens as a
	 * job with a way back here. */
	scheduleBody(s, t) {
		const d = this.detail?.schedule?.id === s.id ? this.detail : null;
		const runs = d?.jobs ?? [];
		return v_flex()
			.px(14)
			.pb(12)
			.child(this.textSection("PROMPT", s.description, t))
			.when(Boolean(s.pre_run), (el) =>
				el.child(this.textSection("PRE-RUN", s.pre_run, t)),
			)
			.when(Boolean(s.post_run), (el) =>
				el.child(this.textSection("POST-RUN", s.post_run, t)),
			)
			.child(this.label("RUNS", t, runs.length))
			.when(!d, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("Loading…")),
			)
			.when(Boolean(d) && runs.length === 0, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("Not fired yet.")),
			)
			.children(
				runs.map((j) => {
					const c = stateColor(j.state);
					const gist = resultSummary(j.result).split("\n")[0];
					return h_flex()
						.id(`run-${j.id}`)
						.mx(-6)
						.px(6)
						.py(3)
						.gap(8)
						.rounded(6)
						.items_center()
						.cursor_pointer()
						.hover((s2) => s2.bg(t.hover))
						.on_click((_e, cx) => {
							this.select("job", j.id, cx, s.id);
							cx.notify();
						})
						.child(div().size(7).rounded(4).flex_none().bg(t[c]))
						.child(
							div()
								.w(84)
								.flex_none()
								.text_size(11)
								.text_color(t.dim)
								.child(stamp(j.occurrence_at ?? j.created_at)),
						)
						.child(
							div()
								.w(96)
								.flex_none()
								.text_size(11)
								.text_color(t[c])
								.child(j.state),
						)
						.child(
							div()
								.flex_1()
								.min_w(0)
								.truncate()
								.text_size(12)
								.text_color(t.muted)
								.child(gist || (CLOSED.includes(j.state) ? "" : "in progress")),
						);
				}),
			);
	}

	/** The edit form: each field prefilled, locked ones dim; Save sends the
	 * changed ones, an emptied field clears it. */
	editForm(obj, t) {
		const e = this.edit;
		const note =
			e.kind === "schedule"
				? "Edits apply to the jobs it fires next."
				: obj.state === "queued"
					? "An edit reaches the agent with the job."
					: "Delivered: only post-run can change.";
		const box = (f) =>
			div()
				.id(`ef-${f.key}`)
				.px(9)
				.py(f.multi ? 5 : 0)
				.when(!f.multi, (el) => el.h(30).flex().items_center())
				.rounded(7)
				.border_1()
				.border_color(t.input)
				.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
				.child(
					f.multi
						? Textarea.new(e.states[f.key]).w_full().text_size(12)
						: Input.new(e.states[f.key]).h_full().w_full().text_size(12),
				);
		return v_flex()
			.px(14)
			.pt(8)
			.pb(12)
			.gap(2)
			.child(div().text_size(11).text_color(t.dim).child(note))
			.children(
				e.fields.flatMap((f) => [
					this.label(f.label, t),
					f.locked
						? div()
								.min_w(0)
								.truncate()
								.text_size(12)
								.text_color(t.dim)
								.child(f.value.split("\n")[0] || "(none)")
						: box(f),
				]),
			)
			.child(
				h_flex()
					.pt(10)
					.gap(8)
					.justify_end()
					.child(
						this.button("ef-cancel", "Cancel", t, (cx) => {
							this.closeEdit();
							cx.notify();
						}),
					)
					.child(
						this.button(
							"ef-save",
							this.busy === e.id ? "Saving…" : "Save",
							t,
							(cx) => this.saveEdit(cx),
							{ solid: true },
						),
					),
			);
	}
}
