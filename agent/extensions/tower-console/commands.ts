/**
 * Operator slash commands: instant views and actions with no model turn.
 *
 *   /fleet [stack]   the fleet card (into the transcript when idle, an overlay while a turn runs)
 *   /inbox           walk pending questions and approvals: answer, approve, deny, dismiss, peek
 *   /peek <agent>    an agent's recent screen in an overlay
 */

import type {
	ExtensionAPI,
	ExtensionCommandContext,
} from "@oh-my-pi/pi-coding-agent";
import type { AutocompleteItem, Component } from "@oh-my-pi/pi-tui";
import type { TowerClient } from "../lib/client";
import { fleetText, relative } from "../lib/format";
import type { Agent, Message } from "../lib/model";
import { fleetCard, Lines } from "../lib/render";
import {
	buildView,
	type FleetView,
	fetchSnapshot,
	offlineView,
	stackOf,
} from "../lib/snapshot";
import { contextTail, messageText, oneLine, trunc } from "../lib/text";
import type { LiveFleet } from "./live";

export const FLEET_MESSAGE = "tower.fleet";
export const NOTICE_MESSAGE = "tower.notice";

/** A read-only overlay that closes on any key. */
class Panel extends Lines implements Component {
	#close: () => void;

	constructor(build: (width: number) => string[], close: () => void) {
		super(build);
		this.#close = close;
	}

	handleInput(): void {
		this.#close();
	}
}

async function showPanel(
	ctx: ExtensionCommandContext,
	title: string,
	body: string[],
): Promise<void> {
	await ctx.ui.custom<undefined>(
		(_tui, theme, _keybindings, done) =>
			new Panel(
				() => [
					theme.bold(theme.fg("accent", title)),
					"",
					...body,
					"",
					theme.fg("dim", "any key closes"),
				],
				() => done(undefined),
			),
		{ overlay: true },
	);
}

async function screenLines(
	client: TowerClient,
	agent: string,
	max: number,
): Promise<string[]> {
	const { output } = await client.get<{ output: string }>(
		`/v1/agents/${encodeURIComponent(agent)}/read`,
	);
	const lines = output.split("\n").map((line) => line.trimEnd());
	while (lines.length && !lines[lines.length - 1]) lines.pop();
	return lines.slice(-max);
}

/** Record what the operator did outside a model turn, so the agent sees it on its next turn. */
function notice(
	pi: ExtensionAPI,
	ctx: ExtensionCommandContext,
	text: string,
): void {
	pi.sendMessage(
		{
			customType: NOTICE_MESSAGE,
			content: text,
			display: true,
			attribution: "user",
		},
		ctx.isIdle() ? undefined : { deliverAs: "nextTurn" },
	);
}

export function registerCommands(
	pi: ExtensionAPI,
	client: TowerClient,
	live: () => LiveFleet | undefined,
): void {
	pi.registerCommand("fleet", {
		description:
			"Show the tower fleet card: what needs you, agents by stack, open jobs, schedules",
		getArgumentCompletions: (prefix) => stackCompletions(live(), prefix),
		handler: async (args, ctx) => {
			const stack = args.trim() || undefined;
			let view: FleetView;
			try {
				view = buildView(await fetchSnapshot(client), stack);
			} catch (err) {
				view = offlineView(
					client.describe(),
					err instanceof Error ? err.message : String(err),
				);
			}
			if (ctx.isIdle() || !ctx.hasUI) {
				pi.sendMessage({
					customType: FLEET_MESSAGE,
					content: fleetText(view),
					display: true,
					details: view,
					attribution: "user",
				});
				return;
			}
			await ctx.ui.custom<undefined>(
				(_tui, theme, _keybindings, done) =>
					new Panel(
						(width) => fleetCard(view, theme, width),
						() => done(undefined),
					),
				{ overlay: true },
			);
		},
	});

	pi.registerCommand("peek", {
		description: "Show an agent's recent screen: /peek <agent>",
		getArgumentCompletions: (prefix) => agentCompletions(live(), prefix),
		handler: async (args, ctx) => {
			if (!ctx.hasUI) return;
			let agent = args.trim();
			if (!agent) {
				const { agents } = await client.get<{ agents: Agent[] }>("/v1/agents");
				const picked = await ctx.ui.select(
					"Peek at which agent?",
					agents.map((item) => ({
						label: item.name,
						description: `${item.kind} · ${item.state}`,
					})),
				);
				if (!picked) return;
				agent = picked;
			}
			try {
				await showPanel(
					ctx,
					`${agent} — screen`,
					await screenLines(client, agent, 60),
				);
			} catch (err) {
				ctx.ui.notify(
					`peek ${agent}: ${err instanceof Error ? err.message : String(err)}`,
					"error",
				);
			}
		},
	});

	pi.registerCommand("inbox", {
		description:
			"Answer, approve, deny, or dismiss pending tower questions and approvals",
		handler: async (_args, ctx) => {
			if (!ctx.hasUI) return;
			for (;;) {
				let messages: Message[];
				try {
					({ messages } = await client.get<{ messages: Message[] }>(
						"/v1/messages?to=me&status=pending",
					));
				} catch (err) {
					ctx.ui.notify(
						`inbox: ${err instanceof Error ? err.message : String(err)}`,
						"error",
					);
					return;
				}
				if (!messages.length) {
					ctx.ui.notify("tower inbox is empty", "info");
					return;
				}
				const options = messages.map((message) => ({
					label: `${message.from_id} · ${message.kind} · ${relative(message.created_at)}`,
					description: trunc(oneLine(messageText(message)), 90),
				}));
				const picked = await ctx.ui.select(
					`tower inbox — ${messages.length} pending`,
					options,
				);
				if (!picked) return;
				const message =
					messages[options.findIndex((option) => option.label === picked)];
				if (!message) return;
				await handleItem(pi, client, ctx, message);
				void live()?.refresh();
			}
		},
	});
}

async function handleItem(
	pi: ExtensionAPI,
	client: TowerClient,
	ctx: ExtensionCommandContext,
	message: Message,
): Promise<void> {
	const text = messageText(message);
	const context = contextTail(message, 8);
	const expires = message.deadline_at
		? ` · expires ${relative(message.deadline_at)}`
		: "";
	const title = [
		`${message.from_id} · ${message.kind} · ${relative(message.created_at)}${expires}`,
		text,
		context ?? "",
	]
		.filter(Boolean)
		.join("\n");
	const actions =
		message.kind === "approval"
			? [
					"Approve",
					"Deny",
					"Answer with text…",
					"Peek at screen",
					"Dismiss as stale",
					"Back",
				]
			: ["Answer…", "Peek at screen", "Dismiss as stale", "Back"];
	for (;;) {
		const action = await ctx.ui.select(title, actions);
		if (!action || action === "Back") return;
		const route = `/v1/messages/${encodeURIComponent(message.id)}`;
		try {
			switch (action) {
				case "Approve":
				case "Deny": {
					const approve = action === "Approve";
					await client.post(
						`${route}/respond`,
						{ approve },
						{ timeoutMs: 60_000 },
					);
					ctx.ui.notify(
						`${approve ? "approved" : "denied"} → ${message.from_id}`,
						"info",
					);
					notice(
						pi,
						ctx,
						`Operator ${approve ? "approved" : "denied"} ${message.from_id}'s ${message.kind} ${message.id} via /inbox.`,
					);
					return;
				}
				case "Answer…":
				case "Answer with text…": {
					const answer = await ctx.ui.input(
						`Reply to ${message.from_id}`,
						"typed into the agent as its answer",
					);
					if (!answer?.trim()) continue;
					await client.post(
						`${route}/respond`,
						{ approve: true, parts: [{ text: answer }] },
						{ timeoutMs: 60_000 },
					);
					ctx.ui.notify(`answered → ${message.from_id}`, "info");
					notice(
						pi,
						ctx,
						`Operator answered ${message.from_id}'s ${message.kind} ${message.id} via /inbox: "${answer}"`,
					);
					return;
				}
				case "Dismiss as stale": {
					await client.post(`${route}/dismiss`);
					ctx.ui.notify(
						`dismissed ${message.id} (${message.from_id} is not told)`,
						"info",
					);
					notice(
						pi,
						ctx,
						`Operator dismissed ${message.from_id}'s ${message.kind} ${message.id} as stale via /inbox.`,
					);
					return;
				}
				case "Peek at screen":
					await showPanel(
						ctx,
						`${message.from_id} — screen`,
						await screenLines(client, message.from_id, 60),
					);
					break;
			}
		} catch (err) {
			ctx.ui.notify(
				`${action}: ${err instanceof Error ? err.message : String(err)}`,
				"error",
			);
			return;
		}
	}
}

function agentCompletions(
	live: LiveFleet | undefined,
	prefix: string,
): AutocompleteItem[] | null {
	const agents = live?.snapshot?.agents ?? [];
	const matches = agents
		.filter((agent) => agent.name.startsWith(prefix.trim()))
		.map((agent) => ({
			value: agent.name,
			label: agent.name,
			description: `${agent.kind} · ${agent.state}`,
		}));
	return matches.length ? matches : null;
}

function stackCompletions(
	live: LiveFleet | undefined,
	prefix: string,
): AutocompleteItem[] | null {
	const stacks = new Set<string>();
	for (const agent of live?.snapshot?.agents ?? []) {
		const stack = stackOf(agent.name);
		if (stack) stacks.add(stack);
	}
	const matches = [...stacks]
		.filter((stack) => stack.startsWith(prefix.trim()))
		.sort()
		.map((stack) => ({ value: stack, label: stack }));
	return matches.length ? matches : null;
}
