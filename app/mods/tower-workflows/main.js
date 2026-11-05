// Workflows: every workflow (state, version, triggers, running runs), and,
// clicking one, its graph as a top-down list of nodes, the way a PR
// timeline reads. Branches indent under the node they leave, labeled with
// the outcome that takes them; a loop is a `↺` row back to its node; a join
// shows once its branches are done. Pick a run and the graph shows where
// it is: each node's latest step (done, failed, running, waiting) and, on
// a click, every visit's summary.
// Reads: `tower workflow list|show`, `tower run show`, `tower ps`.
// Acts: run (a form of the declared inputs), enable / disable, remove and
// cancel a run (two presses), edit the TOML source, and create a workflow
// from a form of steps (or from TOML). Definitions reach the server through
// `tower workflow apply <file>`; locally the file is this mod's draft in the
// app's data folder. Everything goes to the machine tower.js targets.

import {
	Editor,
	EditorState,
	h_flex,
	Input,
	InputState,
	Textarea,
	TextareaState,
	v_flex,
} from "gpui-base";
import { div, svg, View } from "gpui-kit";
import { appHome, settings, theme } from "herdr";
import {
	ago,
	applyWorkflow,
	followMachine,
	machineLabel,
	stateColor,
	towerOn,
} from "./tower.js";
import {
	agentText,
	draftError,
	draftToml,
	graphRows,
	inputArgs,
	latestStep,
	missingInputs,
	routeKeys,
	runGist,
	stamp,
	starterToml,
	stepsByNode,
	stepTone,
	triggerOf,
	triggersText,
} from "./workflows.js";

const REFRESH_MS = 10000;
const RUN_MS = 3000; // a running run on screen re-reads this often
const RUNS_SHOWN = 5; // runs listed before "show all"
const LEVEL = 20; // px per indent level: one rail column
const ICONS = "mods/tower-workflows";

export default class TowerWorkflows extends View {
	init(_props, cx) {
		this.workflows = null;
		this.agents = null;
		this.agentMap = {};
		this.error = null; // the last action's failure, kept until the next action
		this.loadError = null; // the last poll's failure
		this.notice = null; // what the last apply did
		this.busy = null; // key of the action in flight
		this.confirm = null; // key awaiting a second press
		this.sel = null; // the open workflow's name
		this.detail = null; // its `workflow show` answer
		this.detailError = null;
		this.autoRun = false; // open the newest run if it is running
		this.runId = null; // the run laid over the graph
		this.run = null; // its `run show` answer
		this.opened = new Set(); // expanded graph rows
		this.allRuns = false;
		this.starting = null; // the run form: { [input]: InputState }
		this.editor = null; // { state, name (null: a new one), back }
		this.create = null; // the create form
		this.gen = 0; // bumped when the machine changes; late answers are dropped
		this.machine = followMachine(cx, (m, cx) => {
			// Everything shown came from the old server: start over on the list.
			this.machine = m;
			this.gen++;
			this.closeForms();
			this.workflows = null;
			this.agents = null;
			this.agentMap = {};
			this.error = null;
			this.loadError = null;
			this.notice = null;
			this.busy = null;
			this.confirm = null;
			this.sel = null;
			this.detail = null;
			this.detailError = null;
			this.autoRun = false;
			this.runId = null;
			this.run = null;
			this.opened = new Set();
			this.allRuns = false;
			cx.spawn(async (cx) => this.refresh(cx));
			cx.notify();
		});
		cx.timer.every(REFRESH_MS, (cx) => this.refresh(cx));
		cx.timer.every(RUN_MS, (cx) => {
			if (this.run?.run?.state !== "running") return;
			cx.spawn(async (cx) => {
				await this.loadRun();
				cx.notify();
			});
		});
		this.refresh(cx);
	}

	async refresh(cx) {
		const on = this.machine;
		const gen = this.gen;
		try {
			const [w, ps] = await Promise.all([
				towerOn(on, "workflow", "list"),
				towerOn(on, "ps"),
			]);
			if (this.gen !== gen) return;
			this.workflows = w.workflows ?? [];
			this.agents = ps.agents ?? [];
			this.agentMap = Object.fromEntries(
				this.agents.map((a) => [a.id, a.name]),
			);
			this.loadError = null;
		} catch (e) {
			if (this.gen !== gen) return;
			this.loadError = String(e.message ?? e);
		}
		if (this.sel) await this.loadDetail();
		if (this.runId) await this.loadRun();
		if (this.gen === gen) cx.notify();
	}

	/** `tower workflow show` for the open workflow; a late answer for an
	 * earlier one is dropped. The first answer may open its running run. */
	async loadDetail() {
		const name = this.sel;
		const on = this.machine;
		const gen = this.gen;
		try {
			const d = await towerOn(on, "workflow", "show", name);
			if (this.sel !== name || this.gen !== gen) return;
			this.detail = d;
			this.detailError = null;
			if (this.autoRun) {
				this.autoRun = false;
				const newest = d.runs?.[0];
				if (newest?.state === "running" && !this.runId) {
					this.runId = newest.id;
					await this.loadRun();
				}
			}
		} catch (e) {
			if (this.sel !== name || this.gen !== gen) return;
			this.detailError = String(e.message ?? e);
		}
	}

	async loadRun() {
		const id = this.runId;
		if (!id) return;
		const on = this.machine;
		const gen = this.gen;
		try {
			const r = await towerOn(on, "run", "show", id);
			if (this.runId === id && this.gen === gen) this.run = r;
		} catch (e) {
			if (this.runId === id && this.gen === gen)
				this.loadError = String(e.message ?? e);
		}
	}

	/** Open a workflow (null: back to the list). */
	open(name, cx) {
		this.closeForms();
		this.sel = name;
		this.detail = null;
		this.detailError = null;
		this.runId = null;
		this.run = null;
		this.opened = new Set();
		this.allRuns = false;
		this.confirm = null;
		this.error = null;
		this.autoRun = Boolean(name);
		if (name) {
			cx.spawn(async (cx) => {
				await this.loadDetail();
				cx.notify();
			});
		}
		cx.notify();
	}

	/** Lay a run over the graph; the same run again takes it off. */
	pickRun(id, cx) {
		this.opened = new Set();
		this.confirm = null;
		if (this.runId === id) {
			this.runId = null;
			this.run = null;
			cx.notify();
			return;
		}
		this.runId = id;
		this.run = null;
		cx.spawn(async (cx) => {
			await this.loadRun();
			cx.notify();
		});
		cx.notify();
	}

	/** Run one tower verb. `confirmFirst`: the first press only arms it.
	 * `after(answer)` runs on success, before the refresh. */
	act(cx, key, args, confirmFirst, after) {
		if (confirmFirst && this.confirm !== key) {
			this.confirm = key;
			cx.notify();
			return;
		}
		this.confirm = null;
		this.notice = null;
		this.busy = key;
		cx.notify();
		const on = this.machine;
		const gen = this.gen;
		cx.spawn(async (cx) => {
			try {
				const v = await towerOn(on, ...args);
				if (this.gen !== gen) return;
				this.error = null;
				after?.(v);
			} catch (e) {
				if (this.gen !== gen) return;
				this.error = e.message;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	/** Apply `source`; on success say what changed and open the workflow. */
	applyAndOpen(cx, source, done) {
		this.busy = "apply";
		this.error = null;
		cx.notify();
		const on = this.machine;
		const gen = this.gen;
		cx.spawn(async (cx) => {
			try {
				const v = await applyWorkflow(
					source,
					`${appHome()}/data/tower-workflows/draft.toml`,
					on,
				);
				if (this.gen !== gen) return;
				const name = v.workflow?.name;
				this.notice = `${name} v${v.version} ${
					v.changed ? (v.version === 1 ? "created" : "saved") : "unchanged"
				}`;
				done();
				this.busy = null;
				this.open(name, cx);
			} catch (e) {
				if (this.gen !== gen) return;
				this.error = String(e.message ?? e);
				this.busy = null;
			}
			await this.refresh(cx);
		});
	}

	closeForms() {
		this.closeStart();
		this.closeEditor();
		this.closeCreate();
	}

	// --- run form ---------------------------------------------------------

	openStart(cx) {
		const inputs = this.detail?.definition?.inputs ?? {};
		this.starting = Object.fromEntries(
			Object.entries(inputs).map(([k, spec]) => [
				k,
				InputState.new({
					placeholder:
						spec.description ??
						(spec.default != null ? `default: ${spec.default}` : k),
				}),
			]),
		);
		cx.notify();
	}

	closeStart() {
		for (const s of Object.values(this.starting ?? {})) s.release();
		this.starting = null;
	}

	submitStart(cx) {
		const values = Object.fromEntries(
			Object.entries(this.starting).map(([k, s]) => [k, s.value()]),
		);
		const missing = missingInputs(this.detail?.definition, values);
		if (missing.length) {
			this.error = `needs ${missing.join(", ")}`;
			cx.notify();
			return;
		}
		const name = this.sel;
		this.act(
			cx,
			"start",
			["workflow", "run", name, ...inputArgs(values)],
			false,
			(v) => {
				this.closeStart();
				if (v.run?.id && this.sel === name) {
					this.runId = v.run.id;
					this.run = null;
					this.opened = new Set();
				}
			},
		);
	}

	// --- TOML editor ------------------------------------------------------

	/** `name`: the workflow being edited, null for a new one; `back`:
	 * "form" returns to the create form on Cancel. */
	openEditor(source, name, back, cx) {
		this.closeEditor();
		this.editor = {
			state: EditorState.new({
				language: "toml",
				value: source,
				font_family: settings().font_family,
				font_size: 12,
				line_numbers: true,
				folding: false,
				soft_wrap: true,
			}),
			name,
			back,
		};
		cx.notify();
	}

	closeEditor() {
		this.editor?.state.release();
		this.editor = null;
	}

	// --- create form ------------------------------------------------------

	openCreate(cx) {
		this.closeForms();
		this.error = null;
		this.notice = null;
		this.create = {
			name: InputState.new({ placeholder: "fix-issue" }),
			description: InputState.new({ placeholder: "What it does" }),
			inputs: InputState.new({ placeholder: "issue, repo (optional)" }),
			steps: [],
		};
		this.addStep(cx);
	}

	addStep(cx) {
		const prompt = TextareaState.new({
			placeholder:
				"What this step does. {{ input.issue }} inserts an input; the agent gets the earlier steps' summaries.",
		});
		prompt.set_auto_grow(2, 8);
		this.create.steps.push({
			key: `${Date.now()}-${this.create.steps.length}`,
			id: InputState.new({ placeholder: "triage" }),
			agent: null,
			prompt,
			outcomes: InputState.new({ placeholder: "build, design (optional)" }),
			routes: {},
		});
		cx.notify();
	}

	removeStep(i, cx) {
		const [s] = this.create.steps.splice(i, 1);
		for (const st of [s.id, s.prompt, s.outcomes]) st.release();
		cx.notify();
	}

	closeCreate() {
		const f = this.create;
		if (!f) return;
		for (const st of [f.name, f.description, f.inputs]) st.release();
		for (const s of f.steps)
			for (const st of [s.id, s.prompt, s.outcomes]) st.release();
		this.create = null;
	}

	/** The create form's values, as `draftToml` reads them. */
	draft() {
		const f = this.create;
		return {
			name: f.name.value().trim(),
			description: f.description.value(),
			inputs: f.inputs.value(),
			steps: f.steps.map((s) => ({
				id: s.id.value().trim(),
				agent: s.agent,
				prompt: s.prompt.value(),
				outcomes: s.outcomes.value(),
				routes: s.routes,
			})),
		};
	}

	submitCreate(cx) {
		const d = this.draft();
		const err = draftError(d);
		if (err) {
			this.error = err;
			cx.notify();
			return;
		}
		this.applyAndOpen(cx, draftToml(d), () => this.closeCreate());
	}

	/** Continue in TOML: the form so far, else a starter definition. */
	createAsToml(cx) {
		const d = this.draft();
		const source =
			d.name || d.steps.some((s) => s.id || s.prompt.trim())
				? draftToml(d)
				: starterToml(this.agents?.[0]?.name);
		this.openEditor(source, null, "form", cx);
	}

	// --- shared pieces ----------------------------------------------------

	button(id, label, t, onClick, opts = {}) {
		return h_flex()
			.id(id)
			.h(opts.h ?? 22)
			.px(opts.px ?? 6)
			.flex_none()
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

	pill(id, label, on, t, onClick, dot) {
		return h_flex()
			.id(id)
			.h(22)
			.px(7)
			.gap(5)
			.items_center()
			.rounded(6)
			.text_size(11.5)
			.cursor_pointer()
			.bg(on ? t.selected : t.raised)
			.text_color(on ? t.foreground : t.muted)
			.border_1()
			.border_color(on ? t.accent : t.border)
			.hover((s) => s.bg(t.hover))
			.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
			.on_click((_e, cx) => {
				onClick();
				cx.notify();
			})
			.when(Boolean(dot), (el) => el.child(div().size(6).rounded(3).bg(dot)))
			.child(label);
	}

	field(id, state, t) {
		return div()
			.id(id)
			.h(28)
			.px(8)
			.flex()
			.items_center()
			.rounded(7)
			.border_1()
			.border_color(t.input)
			.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
			.child(Input.new(state).h_full().w_full().text_size(12));
	}

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
	para(text, t, color, max) {
		let lines = String(text).split("\n");
		const cut = max && lines.length > max;
		if (cut) lines = [...lines.slice(0, max), "…"];
		return v_flex()
			.min_w(0)
			.text_size(12)
			.text_color(color ?? t.foreground)
			.children(
				lines.map((l) =>
					div()
						.min_w(0)
						.child(l || " "),
				),
			);
	}

	facts(rows, t) {
		return v_flex()
			.flex_none()
			.gap(1)
			.text_size(12)
			.children(
				rows.map(([label, value], i) =>
					h_flex()
						.id(`wf-fact-${i}`)
						.min_w(0)
						.gap(8)
						.child(
							div()
								.w(64)
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
								.child(String(value ?? "-")),
						),
				),
			);
	}

	strip(text, color) {
		return div().px(14).py(6).text_size(12).text_color(color).child(text);
	}

	// --- render -----------------------------------------------------------

	render() {
		const t = theme();
		const list = this.workflows ?? [];
		const running = list.reduce((n, w) => n + (w.open_runs ?? 0), 0);
		const body = this.create
			? this.editor
				? this.editorScreen(t)
				: this.createForm(t)
			: this.editor
				? this.editorScreen(t)
				: this.sel
					? this.detailScreen(t)
					: this.listScreen(t);
		return v_flex()
			.id("tower-workflows")
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
					.child(div().font_semibold().child("Workflows"))
					.child(
						div()
							.text_size(11)
							.text_color(t.dim)
							.child(`on ${machineLabel(this.machine)}`),
					)
					.child(div().flex_1())
					.child(
						div()
							.text_size(11)
							.text_color(t.dim)
							.child(`${list.length} · ${running} running`),
					),
			)
			.when(Boolean(this.error ?? this.loadError), (el) =>
				el.child(this.strip(this.error ?? this.loadError, t.ansi1)),
			)
			.when(Boolean(this.notice) && !(this.error ?? this.loadError), (el) =>
				el.child(this.strip(this.notice, t.muted)),
			)
			.child(body)
			.when(!this.sel && !this.create && !this.editor, (el) =>
				el.child(
					h_flex()
						.h(38)
						.flex_none()
						.px(14)
						.items_center()
						.border_t(1)
						.border_color(t.border)
						.child(
							this.button(
								"wf-new",
								"＋ New workflow…",
								t,
								(cx) => this.openCreate(cx),
								{ solid: true, h: 24, px: 8 },
							),
						),
				),
			);
	}

	listScreen(t) {
		const list = this.workflows;
		return v_flex()
			.id("wf-list")
			.flex_1()
			.min_h(0)
			.overflow_y_scrollbar()
			.pb(10)
			.when(list === null, (el) => el.child(this.strip("Loading…", t.dim)))
			.when(list?.length === 0, (el) =>
				el.child(
					this.strip(
						"No workflows yet. A workflow chains jobs: each step is a job for an agent, and its outcome picks the next.",
						t.dim,
					),
				),
			)
			.children((list ?? []).map((w) => this.workflowRow(w, t)));
	}

	workflowRow(w, t) {
		const live = w.open_runs > 0;
		const color = !w.enabled ? t.dim : live ? t.working : t.idle;
		const state = !w.enabled
			? "disabled"
			: live
				? `${w.open_runs} running`
				: "enabled";
		return v_flex()
			.id(`wf-${w.name}`)
			.mx(8)
			.my(3)
			.px(8)
			.py(6)
			.gap(3)
			.rounded(7)
			.cursor_pointer()
			.hover((s) => s.bg(t.hover))
			.on_click((_e, cx) => this.open(w.name, cx))
			.child(
				h_flex()
					.gap(7)
					.items_center()
					.child(div().size(8).rounded(4).flex_none().bg(color))
					.child(div().min_w(0).truncate().font_semibold().child(w.name))
					.child(
						div()
							.flex_none()
							.text_size(11)
							.text_color(t.dim)
							.child(`v${w.current_version}`),
					)
					.child(div().flex_1())
					.child(
						div().flex_none().text_size(11).text_color(color).child(state),
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
						[
							`${w.nodes} ${w.nodes === 1 ? "step" : "steps"}`,
							triggersText(w.triggers),
							w.description,
						]
							.filter(Boolean)
							.join(" · "),
					),
			);
	}

	// --- detail -----------------------------------------------------------

	detailScreen(t) {
		const d = this.detail?.workflow?.name === this.sel ? this.detail : null;
		const w = d?.workflow;
		const top = h_flex()
			.h(34)
			.flex_none()
			.px(8)
			.gap(6)
			.items_center()
			.border_b(1)
			.border_color(t.border)
			.child(
				h_flex()
					.id("wf-back")
					.h(24)
					.px(6)
					.items_center()
					.rounded(6)
					.cursor_pointer()
					.text_color(t.muted)
					.hover((s) => s.bg(t.hover))
					.on_click((_e, cx) => this.open(null, cx))
					.child("‹ Workflows"),
			)
			.child(div().flex_1())
			.when(Boolean(w), (el) =>
				el
					.child(
						div()
							.text_size(11)
							.text_color(t.dim)
							.child(`v${w.current_version}`),
					)
					.child(
						div()
							.text_size(11)
							.text_color(w.enabled ? t.muted : t.dim)
							.child(w.enabled ? "enabled" : "disabled"),
					),
			);
		if (!d) {
			return v_flex()
				.flex_1()
				.min_h(0)
				.child(top)
				.child(
					this.strip(
						this.detailError ?? "Loading…",
						this.detailError ? t.ansi1 : t.dim,
					),
				);
		}
		const def = d.definition ?? {};
		const inputs = Object.entries(def.inputs ?? {});
		const schedules = d.schedules ?? [];
		const facts = [
			["triggers", triggersText(def.triggers)],
			[
				"inputs",
				inputs.length
					? inputs
							.map(
								([k, s]) => `${k}${s.required && s.default == null ? "*" : ""}`,
							)
							.join(", ")
					: "none",
			],
		];
		if (def.result) facts.push(["result", `from ${def.result}`]);
		for (const s of schedules) {
			facts.push([
				"next run",
				s.enabled && s.next_run_at
					? `${stamp(s.next_run_at)} (${s.cron})`
					: "paused",
			]);
		}
		facts.push(["updated", `${stamp(w.updated_at)} (${ago(w.updated_at)})`]);
		return v_flex()
			.flex_1()
			.min_h(0)
			.child(top)
			.child(
				v_flex()
					.id("wf-detail")
					.flex_1()
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						v_flex()
							.px(14)
							.pt(12)
							.gap(4)
							.child(div().text_size(15).font_semibold().child(w.name))
							.when(Boolean(w.description), (el) =>
								el.child(this.para(w.description, t, t.muted)),
							),
					)
					.child(this.detailActions(w, t))
					.when(Boolean(this.starting), (el) => el.child(this.startForm(t)))
					.child(div().px(14).child(this.facts(facts, t)))
					.child(div().px(14).child(this.runsList(d, t)))
					.child(div().px(14).pb(16).child(this.graph(d, t))),
			);
	}

	detailActions(w, t) {
		const name = w.name;
		const rm = `rm-${name}`;
		return h_flex()
			.flex_none()
			.flex_wrap()
			.px(14)
			.py(8)
			.gap(6)
			.when(w.enabled && !this.starting, (el) =>
				el.child(
					this.button(
						"wf-run",
						this.busy === "start" ? "Starting…" : "Run…",
						t,
						(cx) => this.openStart(cx),
						{ solid: true },
					),
				),
			)
			.child(
				this.button("wf-edit", "Edit", t, (cx) =>
					this.openEditor(this.detail.source ?? "", name, "detail", cx),
				),
			)
			.child(
				this.button(
					"wf-toggle",
					this.busy === "toggle" ? "…" : w.enabled ? "Disable" : "Enable",
					t,
					(cx) =>
						this.act(cx, "toggle", [
							"workflow",
							w.enabled ? "disable" : "enable",
							name,
						]),
				),
			)
			.child(
				this.button(
					"wf-rm",
					this.confirm === rm ? "Remove?" : "Remove",
					t,
					(cx) =>
						this.act(cx, rm, ["workflow", "rm", name], true, () => {
							this.notice = `${name} removed`;
							this.sel = null;
							this.detail = null;
							this.runId = null;
							this.run = null;
						}),
					{ danger: true },
				),
			);
	}

	startForm(t) {
		const inputs = Object.entries(this.detail?.definition?.inputs ?? {});
		return v_flex()
			.mx(14)
			.mb(8)
			.p(10)
			.gap(2)
			.rounded(8)
			.bg(t.raised)
			.child(
				div()
					.text_size(12)
					.font_semibold()
					.child(
						inputs.length
							? "Start a run"
							: "Start a run (this workflow takes no input)",
					),
			)
			.children(
				inputs.flatMap(([k, spec]) => [
					this.label(
						`${k.toUpperCase()}${spec.required && spec.default == null ? " *" : ""}`,
						t,
					),
					this.field(`wf-in-${k}`, this.starting[k], t),
				]),
			)
			.child(
				h_flex()
					.pt(8)
					.gap(8)
					.justify_end()
					.child(
						this.button("wf-start-cancel", "Cancel", t, (cx) => {
							this.closeStart();
							cx.notify();
						}),
					)
					.child(
						this.button(
							"wf-start-go",
							this.busy === "start" ? "Starting…" : "Start",
							t,
							(cx) => this.submitStart(cx),
							{ solid: true },
						),
					),
			);
	}

	runsList(d, t) {
		const runs = d.runs ?? [];
		const shown = this.allRuns ? runs : runs.slice(0, RUNS_SHOWN);
		return v_flex()
			.child(this.label("RUNS", t, runs.length))
			.when(runs.length === 0, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("No runs yet.")),
			)
			.children(
				shown.map((r) => {
					const c = t[stateColor(r.state)];
					const on = r.id === this.runId;
					return h_flex()
						.id(`wf-run-${r.id}`)
						.mx(-6)
						.px(6)
						.py(3)
						.gap(8)
						.rounded(6)
						.items_center()
						.cursor_pointer()
						.when(on, (el) => el.bg(t.selected))
						.hover((s) => s.bg(t.hover))
						.on_click((_e, cx) => this.pickRun(r.id, cx))
						.child(div().size(7).rounded(4).flex_none().bg(c))
						.child(
							div()
								.w(78)
								.flex_none()
								.text_size(11)
								.text_color(t.dim)
								.child(stamp(r.created_at)),
						)
						.child(
							div()
								.w(66)
								.flex_none()
								.text_size(11)
								.text_color(c)
								.child(r.state),
						)
						.child(
							div()
								.flex_1()
								.min_w(0)
								.truncate()
								.text_size(12)
								.text_color(t.muted)
								.child(runGist(r) || triggerOf(r)),
						);
				}),
			)
			.when(runs.length > RUNS_SHOWN, (el) =>
				el.child(
					div()
						.id("wf-runs-all")
						.pt(2)
						.text_size(11)
						.text_color(t.accent)
						.cursor_pointer()
						.on_click((_e, cx) => {
							this.allRuns = !this.allRuns;
							cx.notify();
						})
						.child(this.allRuns ? "show fewer" : `show all ${runs.length}`),
				),
			);
	}

	// --- the graph --------------------------------------------------------

	graph(d, t) {
		const def = d.definition ?? {};
		const rows = graphRows(def);
		const run = this.run?.run?.id === this.runId ? this.run : null;
		const steps = run ? stepsByNode(run.steps) : null;
		const stray = run
			? [...steps.keys()].filter(
					(id) => !(def.nodes ?? []).some((n) => n.id === id),
				)
			: [];
		const cancel = `cancel-${this.runId}`;
		return v_flex()
			.child(
				h_flex()
					.pt(12)
					.pb(6)
					.gap(6)
					.items_center()
					.text_size(11)
					.font_semibold()
					.text_color(t.dim)
					.child(
						this.runId
							? `RUN ${stamp(run?.run.created_at)}${run ? ` · ${run.run.state}` : ""}`
							: "GRAPH",
					)
					.child(div().flex_1())
					.when(run?.run.state === "running", (el) =>
						el.child(
							this.button(
								"wf-cancel-run",
								this.confirm === cancel ? "Cancel run?" : "Cancel run",
								t,
								(cx) =>
									this.act(cx, cancel, ["run", "cancel", this.runId], true),
								{ danger: true },
							),
						),
					)
					.when(Boolean(this.runId), (el) =>
						el.child(
							this.button("wf-run-close", "× Graph", t, (cx) =>
								this.pickRun(this.runId, cx),
							),
						),
					),
			)
			.when(Boolean(run), (el) => el.child(this.runFacts(run.run, d, t)))
			.when(Boolean(this.runId) && !run, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("Loading…")),
			)
			.when(rows.length === 0, (el) =>
				el.child(div().text_size(12).text_color(t.dim).child("No nodes.")),
			)
			.children(rows.map((r, i) => this.graphRow(r, i, steps, t)))
			.when(stray.length > 0, (el) =>
				el
					.child(this.label("STEPS OF NODES THIS VERSION LACKS", t))
					.children(
						stray.flatMap((id) =>
							steps.get(id).map((s) => this.visitLine(s, true, t)),
						),
					),
			);
	}

	runFacts(r, d, t) {
		const input = Object.entries(r.input ?? {})
			.map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
			.join(" ");
		const rows = [
			["trigger", triggerOf(r)],
			["started", `${stamp(r.created_at)} (${ago(r.created_at)})`],
		];
		if (input) rows.push(["input", input]);
		if (r.ended_at) rows.push(["ended", stamp(r.ended_at)]);
		const gist = runGist(r);
		return v_flex()
			.pb(8)
			.child(this.facts(rows, t))
			.when(Boolean(gist), (el) =>
				el.child(
					div()
						.pt(4)
						.child(
							this.para(
								gist,
								t,
								r.state === "failed" ? t.ansi1 : t.foreground,
								4,
							),
						),
				),
			)
			.when(r.version !== d.workflow.current_version, (el) =>
				el.child(
					div()
						.pt(4)
						.text_size(11)
						.text_color(t.blocked)
						.child(
							`This run used v${r.version}; the graph is v${d.workflow.current_version}.`,
						),
				),
			);
	}

	/** A node's mark: a filled check / cross when done / failed, a spinner
	 * while running, a ring while waiting, small and dim before it starts. */
	mark(tone, t) {
		const box = h_flex().size(18).flex_none().items_center().justify_center();
		const filled = (c, icon) =>
			box.child(
				h_flex()
					.size(18)
					.rounded(9)
					.bg(c)
					.items_center()
					.justify_center()
					.child(svg(`${ICONS}/${icon}.svg`).size(11).text_color(t.background)),
			);
		switch (tone) {
			case "ok":
				return filled(t.done, "check");
			case "bad":
				return filled(t.ansi1, "cross");
			case "run":
				return box.child(
					svg("mods/workspace-chrome/spinner.svg")
						.size(14)
						.text_color(t.working)
						.spin(900),
				);
			case "wait":
				return box.child(
					div().size(11).rounded(6).border_2().border_color(t.accent),
				);
			case "skip":
				return box.child(div().size(8).rounded(4).bg(t.dim));
			case "node":
				return box.child(
					div().size(10).rounded(5).border_2().border_color(t.accent),
				);
			case "loop":
				return box.child(div().text_size(13).text_color(t.muted).child("↺"));
			case "start":
				return box.child(div().size(8).rounded(4).bg(t.muted));
			default:
				return box.child(
					div().size(9).rounded(5).border_1().border_color(t.dim),
				);
		}
	}

	/** One row: the rails of the levels above it, its mark (with the line
	 * down to what follows), then its text. */
	graphRow(r, i, steps, t) {
		const key = `${r.kind}-${r.id}-${i}`;
		const node = r.node;
		const list = steps && r.kind === "node" ? steps.get(r.id) : null;
		const latest = latestStep(list);
		const tone =
			r.kind === "node" ? (steps ? stepTone(latest?.state) : "node") : r.kind;
		const open = this.opened.has(key);
		const openable = r.kind === "node" && (steps ? Boolean(latest) : true);
		const rail = (on) =>
			v_flex()
				.w(LEVEL)
				.flex_none()
				.h_full()
				.items_center()
				.when(on, (el) => el.child(div().w(1).flex_1().bg(t.border)));
		const down = !r.last || r.parent;
		const title =
			r.kind === "start"
				? "Start"
				: r.kind === "loop"
					? `back to ${r.id}`
					: r.id;
		const dim = tone === "todo" || tone === "skip";
		const badges = [];
		if (node && r.kind === "node") {
			if (node.join === "all") badges.push("join all");
			if (node.max_visits) badges.push(`≤${node.max_visits} visits`);
		}
		let note = null;
		if (r.kind === "start") {
			note = "these steps start together";
		} else if (r.kind === "loop") {
			note = node?.max_visits
				? `runs ${r.id} again, at most ${node.max_visits} visits`
				: `runs ${r.id} again`;
		} else if (steps) {
			note = latest
				? [
						list.length > 1 ? `visit ${latest.visit}` : null,
						latest.agent_id
							? (this.agentMap[latest.agent_id] ?? latest.agent_id)
							: agentText(node?.agent),
						latest.state,
						latest.outcome ? `→ ${latest.outcome}` : null,
						latest.ended_at
							? ago(latest.ended_at)
							: latest.started_at
								? `since ${ago(latest.started_at)}`
								: null,
					]
						.filter(Boolean)
						.join(" · ")
				: `not reached · ${agentText(node?.agent)}`;
		} else {
			note = agentText(node?.agent);
		}
		const outcomes = !steps && node?.outcomes?.length ? node.outcomes : null;
		return h_flex()
			.id(`wf-row-${key}`)
			.w_full()
			.gap(0)
			.items_start()
			.children(r.guides.map((on) => rail(on)))
			.child(
				v_flex()
					.w(LEVEL)
					.flex_none()
					.h_full()
					.items_center()
					.child(this.mark(tone, t))
					.when(down, (el) =>
						el.child(div().w(1).flex_1().min_h(8).bg(t.border)),
					),
			)
			.child(
				v_flex()
					.flex_1()
					.min_w(0)
					.pl(6)
					.pb(10)
					.gap(1)
					.child(
						h_flex()
							.id(`wf-head-${key}`)
							.w_full()
							.gap(6)
							.items_center()
							.when(openable, (el) =>
								el.cursor_pointer().on_click((_e, cx) => {
									if (open) this.opened.delete(key);
									else this.opened.add(key);
									cx.notify();
								}),
							)
							.when(Boolean(r.via), (el) =>
								el.child(
									div()
										.flex_none()
										.px(5)
										.rounded(4)
										.bg(t.raised)
										.text_size(10.5)
										.text_color(t.accent)
										.child(r.via),
								),
							)
							.child(
								div()
									.min_w(0)
									.truncate()
									.font_medium()
									.text_color(
										r.kind === "loop" ? t.muted : dim ? t.dim : t.foreground,
									)
									.child(title),
							)
							.when(openable, (el) =>
								el.child(
									div()
										.flex_none()
										.text_size(10)
										.text_color(t.dim)
										.child(open ? "▾" : "▸"),
								),
							)
							.child(div().flex_1())
							.children(
								badges.map((b) =>
									div().flex_none().text_size(10.5).text_color(t.dim).child(b),
								),
							),
					)
					.when(Boolean(note), (el) =>
						el.child(
							div()
								.w_full()
								.truncate()
								.text_size(11.5)
								.text_color(latest?.state === "failed" ? t.ansi1 : t.dim)
								.child(note),
						),
					)
					.when(Boolean(r.after), (el) =>
						el.child(
							div()
								.w_full()
								.truncate()
								.text_size(11.5)
								.text_color(t.dim)
								.child(`after ${r.after.join(", ")}`),
						),
					)
					.when(Boolean(outcomes), (el) =>
						el.child(
							h_flex()
								.pt(2)
								.gap(4)
								.flex_wrap()
								.children(
									outcomes.map((o) =>
										div()
											.px(5)
											.rounded(4)
											.border_1()
											.border_color(t.border)
											.text_size(10.5)
											.text_color(t.muted)
											.child(o),
									),
								),
						),
					)
					.when(steps && latest && !open, (el) => {
						const sum = latest.result?.summary;
						return typeof sum === "string" && sum
							? el.child(
									div()
										.w_full()
										.truncate()
										.text_size(11.5)
										.text_color(t.muted)
										.child(sum.split("\n")[0]),
								)
							: el;
					})
					.when(open, (el) =>
						el.child(steps ? this.visits(list, t) : this.nodeBody(node, t)),
					),
			);
	}

	/** A definition node, expanded: its prompt and what it reads. */
	nodeBody(node, t) {
		return v_flex()
			.pt(4)
			.gap(2)
			.child(this.para(node.prompt ?? "", t, t.muted, 12))
			.when(Boolean(node.context?.length), (el) =>
				el.child(
					div()
						.text_size(11)
						.text_color(t.dim)
						.child(`reads ${node.context.join(", ")}`),
				),
			)
			.when(Boolean(node.title), (el) =>
				el.child(
					div()
						.text_size(11)
						.text_color(t.dim)
						.child(`job title: ${node.title}`),
				),
			);
	}

	/** A run's node, expanded: every visit, newest last. */
	visits(list, t) {
		return v_flex()
			.pt(4)
			.gap(4)
			.children(list.map((s) => this.visitLine(s, false, t)));
	}

	visitLine(s, withNode, t) {
		const c = t[stateColor(s.state)];
		const sum = s.result?.summary;
		return v_flex()
			.gap(1)
			.child(
				h_flex()
					.gap(6)
					.items_center()
					.text_size(11.5)
					.child(div().size(6).rounded(3).flex_none().bg(c))
					.child(
						div()
							.min_w(0)
							.truncate()
							.text_color(t.muted)
							.child(
								[
									withNode ? `${s.node_id} #${s.visit}` : `#${s.visit}`,
									s.state,
									s.outcome ? `→ ${s.outcome}` : null,
									s.agent_id ? (this.agentMap[s.agent_id] ?? s.agent_id) : null,
									stamp(s.ended_at ?? s.started_at ?? s.created_at),
								]
									.filter(Boolean)
									.join(" · "),
							),
					),
			)
			.when(typeof sum === "string" && sum !== "", (el) =>
				el.child(
					div()
						.pl(12)
						.child(this.para(sum, t, t.foreground, 20)),
				),
			)
			.when(Boolean(s.error), (el) =>
				el.child(
					div()
						.pl(12)
						.child(this.para(s.error, t, t.ansi1)),
				),
			)
			.when(Boolean(s.task_id) && s.state !== "pending", (el) =>
				el.child(
					div()
						.pl(12)
						.text_size(11)
						.text_color(t.dim)
						.child(`job ${s.task_id}`),
				),
			);
	}

	// --- TOML editor screen ---------------------------------------------

	editorScreen(t) {
		const e = this.editor;
		const back = () => {
			this.closeEditor();
		};
		return v_flex()
			.flex_1()
			.min_h(0)
			.child(
				h_flex()
					.h(34)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.border_b(1)
					.border_color(t.border)
					.child(
						div()
							.flex_1()
							.min_w(0)
							.truncate()
							.font_semibold()
							.child(e.name ? `Edit ${e.name}` : "New workflow (TOML)"),
					)
					.child(
						div()
							.flex_none()
							.text_size(11)
							.text_color(t.dim)
							.child(
								e.name ? "a change saves a new version" : "Create stores v1",
							),
					),
			)
			.child(
				div()
					.id("wf-editor")
					.flex_1()
					.min_h(0)
					.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
					.child(Editor.new(e.state).size_full()),
			)
			.child(
				h_flex()
					.h(38)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.justify_end()
					.border_t(1)
					.border_color(t.border)
					.child(
						this.button(
							"wf-ed-cancel",
							e.back === "form" ? "‹ Form" : "Cancel",
							t,
							(cx) => {
								back();
								cx.notify();
							},
						),
					)
					.child(
						this.button(
							"wf-ed-apply",
							this.busy === "apply" ? "Saving…" : e.name ? "Save" : "Create",
							t,
							(cx) =>
								this.applyAndOpen(cx, e.state.value(), () => {
									this.closeEditor();
									this.closeCreate();
								}),
							{ solid: true },
						),
					),
			);
	}

	// --- create form screen ---------------------------------------------

	createForm(t) {
		const f = this.create;
		return v_flex()
			.flex_1()
			.min_h(0)
			.child(
				v_flex()
					.id("wf-create")
					.flex_1()
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						v_flex()
							.p(14)
							.gap(2)
							.child(div().text_size(15).font_semibold().child("New workflow"))
							.child(
								div()
									.text_size(11.5)
									.text_color(t.dim)
									.child(
										"Each step is a job for an agent. A step's outcomes (words the agent picks from) choose where the run goes next.",
									),
							)
							.child(this.label("NAME", t))
							.child(this.field("wf-c-name", f.name, t))
							.child(this.label("DESCRIPTION", t))
							.child(this.field("wf-c-desc", f.description, t))
							.child(this.label("INPUTS (asked for at each run)", t))
							.child(this.field("wf-c-inputs", f.inputs, t))
							.children(f.steps.map((s, i) => this.stepCard(s, i, t)))
							.child(
								h_flex()
									.pt(8)
									.child(
										this.button("wf-c-add", "＋ Step", t, (cx) =>
											this.addStep(cx),
										),
									),
							),
					),
			)
			.child(
				h_flex()
					.h(38)
					.flex_none()
					.px(14)
					.gap(8)
					.items_center()
					.border_t(1)
					.border_color(t.border)
					.child(
						this.button("wf-c-toml", "TOML…", t, (cx) => this.createAsToml(cx)),
					)
					.child(div().flex_1())
					.child(
						this.button("wf-c-cancel", "Cancel", t, (cx) => {
							this.closeCreate();
							this.error = null;
							cx.notify();
						}),
					)
					.child(
						this.button(
							"wf-c-go",
							this.busy === "apply" ? "Creating…" : "Create",
							t,
							(cx) => this.submitCreate(cx),
							{ solid: true },
						),
					),
			);
	}

	stepCard(s, i, t) {
		const steps = this.create.steps;
		const ids = steps.map((x, j) => x.id.value().trim() || `step ${j + 1}`);
		const keys = routeKeys({ outcomes: s.outcomes.value() });
		const next = ids[i + 1];
		const routeRow = (k) => {
			const cur = s.routes[k] ?? "next";
			const set = (v) => {
				s.routes[k] = v;
			};
			return h_flex()
				.pt(4)
				.gap(4)
				.flex_wrap()
				.items_center()
				.child(
					div()
						.w(76)
						.flex_none()
						.truncate()
						.text_size(11)
						.text_color(t.muted)
						.child(k === "completed" ? "when done" : `on ${k}`),
				)
				.child(
					this.pill(
						`wf-c-${s.key}-${k}-next`,
						next ? `next (${next})` : "end",
						cur === "next",
						t,
						() => set("next"),
					),
				)
				.when(Boolean(next), (el) =>
					el.child(
						this.pill(`wf-c-${s.key}-${k}-end`, "end", cur === "end", t, () =>
							set("end"),
						),
					),
				)
				.children(
					steps
						.map((x, j) => [x, j])
						// Step 1 stays the start: a start node has no incoming edge.
						.filter(([x, j]) => j > 0 && j !== i + 1 && x.id.value().trim())
						.map(([x, j]) =>
							this.pill(
								`wf-c-${s.key}-${k}-${x.key}`,
								j <= i ? `↺ ${ids[j]}` : ids[j],
								cur === x.id.value().trim(),
								t,
								() => set(x.id.value().trim()),
							),
						),
				);
		};
		return v_flex()
			.id(`wf-c-step-${s.key}`)
			.mt(12)
			.p(10)
			.gap(2)
			.rounded(8)
			.border_1()
			.border_color(t.border)
			.child(
				h_flex()
					.items_center()
					.child(
						div()
							.flex_1()
							.text_size(12)
							.font_semibold()
							.child(`STEP ${i + 1}`),
					)
					.when(steps.length > 1, (el) =>
						el.child(
							this.button(`wf-c-rm-${s.key}`, "Remove", t, (cx) =>
								this.removeStep(i, cx),
							),
						),
					),
			)
			.child(this.label("ID", t))
			.child(this.field(`wf-c-id-${s.key}`, s.id, t))
			.child(this.label("AGENT", t))
			.child(
				h_flex()
					.gap(4)
					.flex_wrap()
					.children(
						(this.agents ?? []).map((a) =>
							this.pill(
								`wf-c-ag-${s.key}-${a.id}`,
								a.name,
								s.agent === a.name,
								t,
								() => {
									s.agent = s.agent === a.name ? null : a.name;
								},
								t[stateColor(a.state)],
							),
						),
					),
			)
			.child(this.label("PROMPT", t))
			.child(
				div()
					.id(`wf-c-pr-${s.key}`)
					.px(8)
					.py(5)
					.rounded(7)
					.border_1()
					.border_color(t.input)
					.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
					.child(Textarea.new(s.prompt).w_full().text_size(12)),
			)
			.child(this.label("OUTCOMES", t))
			.child(this.field(`wf-c-out-${s.key}`, s.outcomes, t))
			.child(this.label("THEN", t))
			.children(keys.map(routeRow))
			.when(
				keys.some((k) => {
					const v = s.routes[k];
					const j = steps.findIndex((x) => x.id.value().trim() === v);
					return j >= 0 && j <= i;
				}),
				(el) =>
					el.child(
						div()
							.pt(4)
							.text_size(11)
							.text_color(t.dim)
							.child("A loop: its target step runs at most 3 times per run."),
					),
			);
	}
}
