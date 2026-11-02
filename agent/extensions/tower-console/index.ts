/**
 * tower-console — the operator console around an omp session.
 *
 *   - welcome: a fleet card at session start (it doubles as the model's first
 *     snapshot of the fleet)
 *   - a live strip above the editor: stacks and agent states, inbox, open jobs,
 *     and the most urgent item, fed by the tower event stream
 *   - toasts for new inbox items, dead agents, and failed jobs
 *   - an activity digest of what changed since the model's last turn
 *   - /fleet, /inbox, /peek (commands.ts)
 *
 * Only the main session runs any of this; subagents see the tools alone.
 */

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { Text } from "@oh-my-pi/pi-tui";
import { TowerClient } from "../lib/client";
import { fleetText } from "../lib/format";
import { fleetCard, fleetStrip, Lines } from "../lib/render";
import { type FleetView, offlineView } from "../lib/snapshot";
import { clock } from "../lib/time";
import { FLEET_MESSAGE, NOTICE_MESSAGE, registerCommands } from "./commands";
import { type Change, LiveFleet } from "./live";

const ACTIVITY_MESSAGE = "tower.activity";
const DIGEST_LIMIT = 30;
const WELCOME_WAIT_MS = 5_000;

export default function towerConsole(pi: ExtensionAPI): void {
	pi.setLabel("tower console");
	const client = new TowerClient();

	let live: LiveFleet | undefined;
	let widget: { component: Lines; requestRender: () => void } | undefined;
	let title = "";
	/** The model has a fleet snapshot in this session's context. */
	let primed = false;
	let digest: { at: number; change: Change }[] = [];

	pi.registerMessageRenderer<FleetView>(
		FLEET_MESSAGE,
		(message, _options, theme) => {
			const view = message.details;
			return view
				? new Lines((width) => fleetCard(view, theme, width))
				: undefined;
		},
	);
	pi.registerMessageRenderer(NOTICE_MESSAGE, (message, _options, theme) => {
		const content = typeof message.content === "string" ? message.content : "";
		return new Text(theme.fg("muted", content), 0, 0);
	});
	registerCommands(pi, client, () => live);

	function render(ctx: ExtensionContext): void {
		widget?.component.invalidate();
		widget?.requestRender();
		const view = live?.view;
		if (!view || ctx.mode !== "tui") return;
		const waiting =
			view.inbox.length +
			view.agents.filter(
				(agent) => agent.state === "blocked" || agent.state === "dead",
			).length;
		const next = waiting ? `tower ⚠${waiting}` : "tower";
		if (next !== title) {
			title = next;
			ctx.ui.setTitle(next);
		}
	}

	function onChanges(ctx: ExtensionContext, changes: Change[]): void {
		const at = Date.now();
		digest.push(...changes.map((change) => ({ at, change })));
		if (digest.length > DIGEST_LIMIT) digest = digest.slice(-DIGEST_LIMIT);
		if (!ctx.hasUI) return;
		for (const change of changes.filter((item) => item.notify)) {
			ctx.ui.notify(
				`tower: ${change.text}`,
				change.level === "error" ? "error" : "warning",
			);
		}
	}

	/** The fleet as of its first refresh, waiting at most WELCOME_WAIT_MS for it. */
	async function firstView(
		ctx: ExtensionContext,
		fleet: LiveFleet,
	): Promise<FleetView> {
		const timeout = Promise.withResolvers<void>();
		const timer = ctx.setTimeout(() => timeout.resolve(), WELCOME_WAIT_MS);
		await Promise.race([fleet.ready, timeout.promise]);
		ctx.clearTimer(timer);
		return fleet.view ?? offlineView(client.describe(), "no answer yet");
	}

	async function welcome(
		ctx: ExtensionContext,
		fleet: LiveFleet,
	): Promise<void> {
		const view = await firstView(ctx, fleet);
		// A prompt sent meanwhile gets the snapshot through before_agent_start instead.
		if (fleet !== live || primed || !ctx.hasUI || !ctx.isIdle()) return;
		primed = true;
		pi.sendMessage({
			customType: FLEET_MESSAGE,
			content: fleetText(view),
			display: true,
			details: view,
			attribution: "agent",
		});
	}

	function start(ctx: ExtensionContext): void {
		if (ctx.agent.kind !== "main") return;
		live?.stop();
		primed = false;
		digest = [];
		const fleet = new LiveFleet(client, ctx, {
			update: () => render(ctx),
			changes: (changes) => onChanges(ctx, changes),
		});
		live = fleet;
		if (ctx.mode === "tui") {
			ctx.ui.setWidget(
				"tower",
				(tui) => {
					const component = new Lines((width) =>
						fleetStrip(fleet.view, fleet.link, ctx.ui.theme, width),
					);
					widget = { component, requestRender: () => tui.requestRender() };
					return component;
				},
				{ placement: "aboveEditor" },
			);
			title = "tower";
			ctx.ui.setTitle(title);
		}
		fleet.start();
		welcome(ctx, fleet).catch(() => {});
	}

	pi.on("session_start", async (_event, ctx) => start(ctx));
	pi.on("session_switch", async (_event, ctx) => start(ctx));
	pi.on("session_shutdown", async () => {
		live?.stop();
		live = undefined;
		widget = undefined;
	});

	pi.on("before_agent_start", async (_event, ctx) => {
		if (ctx.agent.kind !== "main" || !live) return undefined;
		if (!primed) {
			primed = true;
			const view = await firstView(ctx, live);
			digest = [];
			return {
				message: {
					customType: FLEET_MESSAGE,
					content: fleetText(view),
					display: false,
					details: view,
					attribution: "agent",
				},
			};
		}
		if (!digest.length) return undefined;
		const entries = digest;
		digest = [];
		const span = `${clock(entries[0].at)}–${clock(entries[entries.length - 1].at)}`;
		const lines = entries.map(
			({ change }) => `- [${change.level}] ${change.text}`,
		);
		return {
			message: {
				customType: ACTIVITY_MESSAGE,
				content: `Tower activity since your last turn (${span}):\n${lines.join("\n")}`,
				display: false,
				attribution: "agent",
			},
		};
	});
}
