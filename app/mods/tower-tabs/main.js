import { h_flex } from "gpui-base";
import { div, image, svg, View } from "gpui-kit";
import {
	closeOverlay,
	layout,
	machines,
	openOverlay,
	panes,
	props,
	run,
	settings,
	spaces,
	theme,
	window,
} from "herdr";
import {
	followMachine,
	machineLabel,
	pin,
	pinned,
	target,
	towerOn,
} from "./tower.js";

const INBOX_MS = 10000; // the Inbox panel's own refresh period

function find(node, id) {
	if (node?.id === id) return node;
	for (const child of node?.children ?? []) {
		const found = find(child, id);
		if (found) return found;
	}
	return null;
}

export default class Tabs extends View {
	init(_props, cx) {
		// Opened as the machine picker (the Machine button below): only the
		// menu, no polling.
		this.picker = Boolean(props()?.machines);
		if (this.picker) return;
		this.pressed = null;
		this.panels = JSON.parse(localStorage.getItem("panels") ?? "{}");
		// Pending inbox items on the machine the panels follow, polled here so
		// the Inbox button shows them while the panel is closed. A failed call
		// keeps the last count; a machine change starts from zero.
		this.inboxCount = 0;
		const poll = (cx) => {
			const on = this.machine;
			cx.spawn(async (cx) => {
				try {
					const v = await towerOn(on, "inbox");
					if (on === this.machine) this.inboxCount = (v.messages ?? []).length;
				} catch {
					// no tower: the Inbox panel itself says so
				}
				cx.notify();
			});
		};
		this.machine = followMachine(cx, (m, cx) => {
			this.machine = m;
			this.inboxCount = 0;
			poll(cx);
			cx.notify();
		});
		cx.timer.every(INBOX_MS, poll);
		poll(cx);
	}

	/** The picker: follow the shown space, or pin this computer or a machine
	 * (`hc machine ls`). Choosing one closes it. */
	renderPicker() {
		const t = theme();
		const current = pinned();
		const space = spaces().find((p) => p.active)?.machine ?? null;
		const choose = (value) => () => {
			pin(value);
			closeOverlay(props().overlay);
		};
		const entry = (id, label, note, chosen, value) =>
			h_flex()
				.id(id)
				.h(28)
				.px(10)
				.gap(8)
				.items_center()
				.rounded(6)
				.text_size(12.8)
				.text_color(chosen ? t.foreground : t.muted)
				.bg(chosen ? t.selected : t.popover)
				.hover((s) => s.bg(t.hover))
				.on_click(choose(value))
				.child(div().flex_1().min_w(0).truncate().child(label))
				.when(Boolean(note), (el) =>
					el.child(div().flex_none().text_color(t.dim).child(note)),
				);
		const others = machines().filter((m) => m.name !== null);
		return div()
			.id("tower-machines")
			.size_full()
			.p(4)
			.flex()
			.flex_col()
			.gap(1)
			.child(
				entry(
					"machine-follow",
					"Follow space",
					machineLabel(space),
					current === null,
					null,
				),
			)
			.child(div().h(1).my(3).bg(t.border))
			.child(entry("machine-local", "this computer", null, current === "", ""))
			.children(
				others.map((m) =>
					entry(
						`machine-${m.name}`,
						m.name,
						m.layout === "unavailable" ? "offline" : null,
						current === m.name,
						m.name,
					),
				),
			);
	}

	// The side panel is remembered per space: switching shows that space's choice
	// (closed if it never had one), and opening or closing it records it.
	syncPanel(space, panel, body, enabled) {
		if (!space || !panel) return;
		const now = panel.hidden ? null : (body?.active ?? null);
		if (space.id !== this.space) {
			const first = this.space === undefined;
			this.space = space.id;
			this.pending = null;
			if (!(first && !(space.id in this.panels))) {
				const stored = this.panels[space.id] ?? null,
					want = enabled.includes(stored) ? stored : null;
				if (want !== now) {
					this.pending = { want, until: Date.now() + 1000 };
					if (want) {
						run("layout.select", { stack: "panel-body", child: want });
						run("layout.show", { id: "panel" });
					} else run("layout.hide", { id: "panel" });
					return;
				}
			}
		}
		// Until the restore lands (or something else won), the old state isn't a choice.
		if (this.pending) {
			if (this.pending.want !== now && Date.now() < this.pending.until) return;
			this.pending = null;
		}
		if (this.panels[space.id] === now) return;
		this.panels[space.id] = now;
		localStorage.setItem("panels", JSON.stringify(this.panels));
	}

	/** The machine the panels talk to; a pin shows as a dot. Opens the picker. */
	machineButton(t, stop) {
		const machine = target();
		return h_flex()
			.id("tower-machine")
			.h(26)
			.px(8)
			.gap(5)
			.flex_none()
			.items_center()
			.rounded(6)
			.text_size(12)
			.bg(t.chrome)
			.text_color(t.muted)
			.hover((s) => s.bg(t.hover))
			.on_mouse_down("left", stop)
			.on_click(() =>
				openOverlay({
					mod: "tower-tabs",
					kind: "popup",
					width: 220,
					height: 64 + 29 * machines().length,
					props: { machines: true },
				}),
			)
			.child(div().max_w(120).truncate().child(machineLabel(machine)))
			.when(pinned() !== null, (el) =>
				el.child(div().size(5).rounded(3).bg(t.accent)),
			);
	}

	render() {
		if (this.picker) return this.renderPicker();
		const t = theme();
		const space = spaces().find((p) => p.active);
		const shown = space?.tabs ?? [];
		const stop = (_e, cx) => cx.stop_propagation();
		const button = (id, label, action) =>
			div()
				.id(id)
				.size(26)
				.flex_none()
				.flex()
				.items_center()
				.justify_center()
				.rounded(6)
				.text_color(t.dim)
				.hover((s) => s.bg(t.hover).text_color(t.foreground))
				.on_mouse_down("left", stop)
				.on_click(action)
				.child(label);
		const item = (c, i) => {
			const program =
				panes().find((p) => p.tab === c.id && p.program?.agent)?.program ??
				panes().find((p) => p.tab === c.id)?.program;
			const name = /^tb\d+$/.test(c.name)
				? (program?.name ?? "Terminal")
				: c.name;
			return h_flex()
				.id(`tab-${c.id}`)
				.h(29)
				.px(10)
				.gap(7)
				.rounded(7)
				.items_center()
				.flex_none()
				.text_size(12.8)
				.border(1)
				.border_color(c.active ? t.border : t.chrome)
				.bg(c.active ? t.raised : t.chrome)
				.text_color(c.active ? t.foreground : t.muted)
				.hover((s) => s.bg(t.hover))
				.on_mouse_down("left", (e, cx) => {
					cx.stop_propagation();
					this.pressed = i;
					if (e.click_count === 2) run("tab.rename", { id: c.id });
				})
				.on_mouse_up("left", () => {
					const from = this.pressed;
					this.pressed = null;
					if (from !== null && from !== i)
						run("tab.move", {
							id: shown[from].id,
							index: space.tabs.findIndex((x) => x.id === c.id) + 1,
						});
					else run("tab.select", { id: c.id });
				})
				.on_mouse_down("right", (_e, cx) => {
					cx.stop_propagation();
					openOverlay({
						mod: "menu",
						kind: "popup",
						width: 200,
						props: { tab: "tab", target: { id: c.id, space: space.id } },
					});
				})
				.when(Boolean(program?.icon), (el) =>
					el.child(
						program.tint
							? svg(program.icon).size(14).text_color(t.muted)
							: image(program.icon).size(14),
					),
				)
				.child(
					div()
						.max_w(170)
						.truncate()
						.child(c.zoomed ? `${name} Z` : name),
				)
				.when(c.status === "working", (el) =>
					el.child(
						svg("mods/workspace-chrome/spinner.svg")
							.size(12)
							.flex_none()
							.text_color(t.working)
							.spin(900),
					),
				)
				.when(c.status === "blocked" || c.status === "done", (el) =>
					el.child(div().size(6).rounded(3).bg(t[c.status])),
				)
				.when(c.active, (el) =>
					el.child(
						div()
							.id(`close-${c.id}`)
							.text_color(t.dim)
							.on_mouse_down("left", stop)
							.on_click((_e, cx) => {
								cx.stop_propagation();
								run("tab.close", { id: c.id });
							})
							.child(svg("mods/pane/close.svg").size(12)),
					),
				);
		};
		const root = layout(),
			sidebar = find(root, "sidebar"),
			w = window();
		// Old saved rails and a dragged-narrow sidebar can be smaller than the window buttons.
		const lightsPadding =
			w?.platform === "macos" &&
			w.titlebar === "none" &&
			!w.fullscreen &&
			root?.traffic_lights
				? Math.max(
						10,
						(root.traffic_lights[0] + 64) / settings().zoom -
							(sidebar?.hidden ? 0 : (sidebar?.size?.px ?? 0)),
					)
				: 10;
		const panel = find(root, "panel"),
			body = find(root, "panel-body");
		// This layout's panel children are tower's, always offered; the
		// per-space restore below keeps a space's last open tower panel
		// (or closed) instead of closing what shared("workspace.panels")
		// doesn't name.
		const enabled = [
			"tower-agents",
			"tower-workflows",
			"tower-jobs",
			"tower-events",
			"tower-inbox",
		];
		this.syncPanel(space, panel, body, enabled);
		const panelButton = (id, label, icon, attention = false) => {
			const on = panel && !panel.hidden && body?.active === id;
			return h_flex()
				.id(`panel-${id}`)
				.h(26)
				.px(8)
				.gap(5)
				.flex_none()
				.items_center()
				.rounded(6)
				.text_size(12)
				.bg(on ? t.selected : t.chrome)
				.text_color(attention ? t.blocked : on ? t.foreground : t.muted)
				.hover((s) => s.bg(t.hover))
				.on_mouse_down("left", stop)
				.on_click(() => {
					if (on) run("layout.hide", { id: "panel" });
					else {
						run("layout.select", { stack: "panel-body", child: id });
						run("layout.show", { id: "panel" });
					}
				})
				.child(svg(icon).size(14))
				.child(label);
		};
		return h_flex()
			.size_full()
			.min_w(0)
			.px(10)
			.pl(lightsPadding)
			.gap(3)
			.items_center()
			.bg(t.chrome)
			.border_b(1)
			.border_color(t.border)
			.on_mouse_up("left", () => {
				this.pressed = null;
			})
			.on_scroll_wheel((e) => {
				if (Math.abs(e.delta.y) >= 1)
					run(e.delta.y > 0 ? "tab.prev" : "tab.next");
			})
			.child(
				h_flex()
					.id("tab-scroll")
					.flex_1()
					.min_w(0)
					.h_full()
					.items_center()
					.gap(3)
					.overflow_x_scrollbar()
					.children(shown.map(item)),
			)
			.child(
				button("tab-new", "+", () =>
					openOverlay({
						mod: "new-menu",
						kind: "popup",
						width: 250,
						height: 512,
					}),
				),
			)
			.child(this.machineButton(t, stop))
			.child(
				panelButton("tower-agents", "Agents", "mods/tower-tabs/agents.svg"),
			)
			.child(
				panelButton(
					"tower-workflows",
					"Workflows",
					"mods/tower-tabs/workflows.svg",
				),
			)
			.child(panelButton("tower-jobs", "Jobs", "mods/tower-tabs/jobs.svg"))
			.child(
				panelButton("tower-events", "Events", "mods/tower-tabs/events.svg"),
			)
			.child(
				panelButton(
					"tower-inbox",
					"Inbox",
					"mods/tower-tabs/inbox.svg",
					this.inboxCount > 0,
				),
			)
			.when(w?.buttons, (el) =>
				el
					.child(
						button(
							"minimize",
							w.platform === "windows"
								? svg("mods/tabs/win-min.svg").size(16)
								: "−",
							() => run("window.minimize"),
						),
					)
					.child(
						button(
							"maximize",
							w.platform === "windows"
								? svg(
										`mods/tabs/${w.maximized ? "win-restore" : "win-max"}.svg`,
									).size(16)
								: "□",
							() => run("window.maximize"),
						),
					)
					.child(
						button(
							"close-window",
							w.platform === "windows"
								? svg("mods/tabs/win-close.svg").size(16)
								: svg("mods/pane/close.svg").size(13),
							() => run("window.close"),
						),
					),
			);
	}
}
