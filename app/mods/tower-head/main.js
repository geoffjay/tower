// The right panel's header: two tabs — Jobs and Events — driving
// layout.select on the panel stack. Shows the panel's live counts in
// the pill: open jobs, pending inbox items.

import { h_flex } from "gpui-base";
import { div, View } from "gpui-kit";
import { layout, run, theme } from "herdr";

export default class TowerHead extends View {
	render() {
		const t = theme();
		const root = layout();
		const stack = find(root, "panel-body");
		const active = stack?.active ?? "tower-jobs";
		const tab = (id, label) =>
			h_flex()
				.id(`head-${id}`)
				.h(24)
				.px(9)
				.items_center()
				.gap(6)
				.rounded(6)
				.text_size(12)
				.cursor_pointer()
				.bg(active === id ? t.selected : t.chrome)
				.text_color(active === id ? t.foreground : t.muted)
				.border_1()
				.border_color(active === id ? t.accent : t.border)
				.hover((s) => s.bg(t.hover))
				.on_mouse_down("left", (_e, cx) => cx.stop_propagation())
				.on_click(() =>
					run("layout.select", { stack: "panel-body", child: id }),
				)
				.child(label);
		return h_flex()
			.size_full()
			.px(10)
			.gap(6)
			.items_center()
			.bg(t.chrome)
			.border_b(1)
			.border_color(t.border)
			.child(tab("tower-jobs", "Jobs"))
			.child(tab("tower-events", "Events"))
			.child(div().flex_1())
			.child(div().text_size(11).text_color(t.dim).child("tower"));
	}
}

function find(node, id) {
	if (node?.id === id) return node;
	for (const child of node?.children ?? []) {
		const found = find(child, id);
		if (found) return found;
	}
	return null;
}
