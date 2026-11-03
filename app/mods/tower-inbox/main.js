// The operator inbox: questions and approvals addressed to you, each
// answerable in place. Reads: `tower inbox --json`. Acts: approve / deny
// (approvals), a free-text answer (questions), dismiss (stale). Approve
// and deny need a second press — an approval lets an agent act.
//
// A pending item also shows the asking agent's screen tail when the agent
// is alive (`tower read <name>`), so "what is it asking about" is one
// click away.

import { h_flex, Input, InputState, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { theme } from "herdr";
import { ago, messageText, tower, towerRead } from "./tower.js";

const PENDING = new Set(["pending"]);

export default class TowerInbox extends View {
	init(_props, cx) {
		this.items = null;
		this.error = null;
		this.busy = null; // message id with an action in flight
		this.confirm = null; // message id awaiting a second press
		this.screen = null; // {from, text} the asking agent's screen tail
		this.screenFor = null;
		this.answer = null; // {id, state} the answer form open on an item
		cx.timer.every(10000, (cx) => this.refresh(cx));
		this.refresh(cx);
	}

	async refresh(cx) {
		try {
			const v = await tower("inbox");
			this.items = v.messages ?? [];
			this.error = null;
		} catch (e) {
			this.error = String(e.message ?? e);
		}
		cx.notify();
	}

	button(id, label, t, onClick, opts = {}) {
		return h_flex()
			.id(id)
			.h(opts.h ?? 24)
			.px(opts.px ?? 8)
			.flex_none()
			.flex()
			.items_center()
			.rounded(6)
			.text_size(12)
			.bg(opts.solid ? t.solid : opts.raised)
			.text_color(
				opts.solid
					? t.solid_foreground
					: opts.danger
						? t.ansi1
						: opts.foreground,
			)
			.border_1()
			.border_color(opts.solid ? t.solid : opts.border)
			.cursor_pointer()
			.hover((s) => s.opacity(0.8))
			.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
			.on_click((_e, cx) => onClick(cx))
			.child(label);
	}

	act(cx, id, args, confirmFirst) {
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
				this.answer = null;
			} catch (e) {
				this.error = `${e.message}`;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	/** Answer a question: a text field opens on the item; Enter sends. */
	openAnswer(m, cx) {
		if (this.answer?.id === m.id) return;
		const state = InputState.new({
			placeholder: `Answer ${m.from_id}…`,
			value: "",
		});
		state.on("submit", (_e, cx2) => this.sendAnswer(m, state, cx2));
		this.answer = { id: m.id, state };
		state.focus();
		cx.notify();
	}

	sendAnswer(m, state, cx) {
		const text = state.value().trim();
		if (!text) return;
		this.busy = m.id;
		this.answer = null;
		cx.notify();
		cx.spawn(async (cx) => {
			try {
				await tower("approve", m.id, "--answer", text);
				this.error = null;
			} catch (e) {
				this.error = e.message;
			}
			this.busy = null;
			await this.refresh(cx);
		});
	}

	/** The asking agent's recent screen, when it is still alive. */
	peek(m, cx) {
		if (this.screenFor === m.id) {
			this.screen = null;
			this.screenFor = null;
			cx.notify();
			return;
		}
		this.screen = { from: m.from_id, text: "…" };
		this.screenFor = m.id;
		cx.notify();
		cx.spawn(async (cx) => {
			let text = null;
			try {
				text = (await towerRead(m.from_id)).trim().split("\n").slice(-24).join("\n");
			} catch (e) {
				text = `cannot read ${m.from_id}: ${e.message}`;
			}
			this.screen = { from: m.from_id, text: text || "(no output)" };
			cx.notify();
		});
	}

	item(m, t) {
		const isApproval = m.kind === "approval";
		const answering = this.answer?.id === m.id;
		const deadline = m.deadline_at
			? ago(m.deadline_at).replace(" ago", " left")
			: null;
		const expired = m.deadline_at && m.deadline_at < Date.now();
		return v_flex()
			.id(`inbox-${m.id}`)
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
					.child(
						div()
							.size(7)
							.rounded(4)
							.flex_none()
							.bg(isApproval ? t.blocked : t.accent),
					)
					.child(
						div().flex_1().min_w(0).truncate().font_semibold().child(m.from_id),
					)
					.child(div().text_size(11).text_color(t.muted).child(m.kind))
					.when(Boolean(deadline), (el) =>
						el.child(
							div()
								.text_size(11)
								.text_color(expired ? t.ansi1 : t.dim)
								.child(deadline),
						),
					),
			)
			.child(
				div()
					.pl(14)
					.text_size(12)
					.text_color(t.foreground)
					.child(messageText(m) || "(no text)"),
			)
			.child(
				h_flex()
					.pl(14)
					.gap(6)
					.items_center()
					.child(
						this.button(
							`peek-${m.id}`,
							this.screenFor === m.id ? "Hide screen" : "Peek agent",
							t,
							(cx) => this.peek(m, cx),
						),
					)
					.when(isApproval, (el) =>
						el
							.child(
								this.button(
									`yes-${m.id}`,
									this.confirm === m.id ? "Approve?" : "Approve",
									t,
									(cx) => this.act(cx, m.id, ["approve", m.id], true),
									{ solid: true },
								),
							)
							.child(
								this.button(
									`no-${m.id}`,
									"Deny",
									t,
									(cx) => this.act(cx, m.id, ["approve", m.id, "--deny"]),
									{ danger: true },
								),
							),
					)
					.when(!isApproval, (el) =>
						el
							.child(
								this.button(
									`ans-${m.id}`,
									"Answer…",
									t,
									(cx) => this.openAnswer(m, cx),
									{ solid: true },
								),
							)
							.child(
								this.button(
									`dis-${m.id}`,
									"Dismiss",
									t,
									(cx) => this.act(cx, m.id, ["dismiss", m.id]),
									{ danger: true },
								),
							),
					),
			)
			.when(answering, (el) =>
				el.child(
					h_flex()
						.id(`answer-bar-${m.id}`)
						.pl(14)
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
								.child(
									Input.new(this.answer.state).w_full().h_full().text_size(12),
								),
						)
						.child(
							this.button(
								`send-${m.id}`,
								"Send",
								t,
								(cx) => this.sendAnswer(m, this.answer.state, cx),
								{ solid: true },
							),
						),
				),
			)
			.when(this.screenFor === m.id && this.screen, (el) =>
				el.child(
					v_flex()
						.id(`screen-${m.id}`)
						.mx(6)
						.mt(2)
						.p(8)
						.rounded(6)
						.bg(t.raised)
						.child(
							div()
								.text_size(11)
								.text_color(t.dim)
								.child(`screen of ${this.screen.from}`),
						)
						.child(
							div()
								.text_size(11)
								.text_color(t.muted)
								.font_family("JetBrains Mono")
								.child(this.screen.text),
						),
				),
			);
	}

	render() {
		const t = theme();
		const items = (this.items ?? []).filter((m) => PENDING.has(m.status));
		return v_flex()
			.id("tower-inbox")
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
					.child(div().font_semibold().child("Inbox"))
					.child(div().flex_1())
					.when(items.length > 0, (el) =>
						el.child(
							div()
								.h(18)
								.px(6)
								.flex()
								.items_center()
								.rounded(9)
								.bg(t.blocked)
								.text_size(11)
								.child(String(items.length)),
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
					.id("tower-inbox-body")
					.flex_1()
					.min_h(0)
					.overflow_y_scrollbar()
					.child(
						v_flex()
							.pb(10)
							.when(this.items === null, (el) =>
								el.child(
									div()
										.px(14)
										.py(8)
										.text_size(12)
										.text_color(t.dim)
										.child("Loading…"),
								),
							)
							.when(this.items !== null && items.length === 0, (el) =>
								el.child(
									div()
										.px(14)
										.py(8)
										.text_size(12)
										.text_color(t.dim)
										.child("Nothing needs you."),
								),
							)
							.children(items.map((m) => this.item(m, t))),
					),
			);
	}
}
