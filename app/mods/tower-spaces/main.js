import { h_flex, v_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { machines, openOverlay, run, spaces, terminals, theme } from "herdr";

const RANK = {
	blocked: 4,
	done: 3,
	working: 2,
	running: 2,
	idle: 1,
};

function spaceStatus(s, allTerminals) {
	let topRank = 0;
	let topStatus = null;

	const consider = (st) => {
		if (!st) return;
		const norm = st === "running" ? "working" : st;
		const r = RANK[norm] ?? 0;
		if (r > topRank) {
			topRank = r;
			topStatus = norm;
		}
	};

	consider(s.status);
	for (const tab of s.tabs ?? []) {
		consider(tab.status);
	}

	if (allTerminals) {
		const prefix = `${s.name} /`;
		for (const t of allTerminals) {
			if (t.state === "live") {
				const inSpace =
					t.labels?.["herdr.space"] === s.id || t.where?.startsWith(prefix);
				if (inSpace) {
					consider(t.status);
				}
			}
		}
	}

	return topStatus;
}

function dotColor(t, status, active) {
	if (status === "blocked") return t.blocked;
	if (status === "done") return t.done;
	if (status === "working") return t.working;
	if (status === "idle") return t.idle;
	return active ? t.foreground : t.dim;
}

function spaceTooltip(s, status, isOffline) {
	const parts = [s.name];
	if (s.machine) parts.push(`(${s.machine})`);
	if (isOffline) parts.push("· offline");
	else if (status) parts.push(`· ${status}`);
	return parts.join(" ");
}

export default class TowerSpaces extends View {
	init(_props, cx) {
		this.pressed = null;
		this.over = null;
		cx.timer.every(1000, (cx) => cx.notify());
	}

	render() {
		const sp = spaces();
		const ms = machines();
		const ts = terminals();
		const t = theme();

		const remoteMachines = [
			...new Set(sp.map((s) => s.machine).filter(Boolean)),
		].sort();
		const machineList = [null, ...remoteMachines];

		const items = [];
		let machineCount = 0;

		for (const m of machineList) {
			const machineSpaces = sp.filter((s) => (s.machine ?? null) === m);
			if (!machineSpaces.length) continue;

			if (machineCount > 0) {
				items.push(
					div()
						.id(`machine-sep-${m ?? "local"}`)
						.w(20)
						.h(1)
						.flex_none()
						.my(3)
						.bg(t.border)
						.tooltip(m ? `Machine: ${m}` : "This computer"),
				);
			}
			machineCount++;

			for (const s of machineSpaces) {
				const status = spaceStatus(s, ts);
				const isOffline =
					Boolean(s.machine) &&
					ms.some((k) => k.name === s.machine && k.layout === "unavailable");
				const tip = spaceTooltip(s, status, isOffline);
				const dotC = isOffline ? t.dim : dotColor(t, status, s.active);
				const dotSize = status ? 8 : 6;
				const fromIndex =
					this.pressed === null
						? -1
						: sp.findIndex((k) => k.id === this.pressed);
				const currentIndex = sp.findIndex((k) => k.id === s.id);

				items.push(
					h_flex()
						.relative()
						.id(`space-${s.id}`)
						.size(28)
						.flex_none()
						.rounded(6)
						.items_center()
						.justify_center()
						.cursor_pointer()
						.tooltip(tip)
						.when(s.active, (el) =>
							el.bg(t.selected).border_1().border_color(t.accent),
						)
						.when(!s.active, (el) =>
							el
								.border_1()
								.border_color("#00000000")
								.hover((h) => h.bg(t.hover)),
						)
						.when(this.pressed === s.id && this.over !== s.id, (el) =>
							el.opacity(0.5),
						)
						.when(
							this.pressed !== null &&
								this.over === s.id &&
								this.pressed !== s.id,
							(el) =>
								el.child(
									div()
										.absolute()
										.left(2)
										.right(2)
										.h(2)
										.bg(t.accent)
										.when(fromIndex < currentIndex, (line) => line.bottom(0))
										.when(fromIndex > currentIndex, (line) => line.top(0)),
								),
						)
						.on_mouse_down("left", () => {
							this.pressed = s.id;
							this.over = s.id;
						})
						.on_mouse_move((_e, cx) => {
							const source = sp.find((k) => k.id === this.pressed);
							const over =
								source && (source.machine ?? null) === (s.machine ?? null)
									? s.id
									: null;
							if (this.pressed !== null && this.over !== over) {
								this.over = over;
								cx.notify();
							}
						})
						.on_mouse_up("left", (_e, cx) => {
							const from = this.pressed;
							const over = this.over;
							this.pressed = null;
							this.over = null;
							if (from !== null && over === s.id && from !== s.id) {
								const index = sp.findIndex((k) => k.id === s.id);
								if (index !== -1) {
									run("space.move", { id: from, index: index + 1 });
								}
							} else if (from === s.id) {
								run("space.select", { id: s.id });
							}
							cx.notify();
						})
						.on_mouse_down("right", () => {
							openOverlay({
								mod: "menu",
								kind: "popup",
								width: 200,
								height: 260,
								props: { tab: "space", target: { id: s.id } },
							});
						})
						.child(
							div()
								.size(dotSize)
								.rounded(dotSize / 2)
								.bg(dotC),
						),
				);
			}
		}

		return v_flex()
			.id("tower-spaces")
			.size_full()
			.min_h(0)
			.items_center()
			.py(6)
			.px(4)
			.bg(t.chrome)
			.rounded(10)
			.border_1()
			.border_color(t.border)
			.on_mouse_up("left", (_e, cx) => {
				if (this.pressed !== null) {
					this.pressed = null;
					this.over = null;
					cx.notify();
				}
			})
			.child(
				v_flex()
					.flex_1()
					.w_full()
					.min_h(0)
					.items_center()
					.overflow_y_scroll()
					.gap(4)
					.children(items),
			);
	}
}
