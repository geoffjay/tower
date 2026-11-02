import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TowerClient } from "../lib/client";
import { fleetText } from "../lib/format";
import { fleetCard, Lines } from "../lib/render";
import { buildView, type FleetView, fetchSnapshot } from "../lib/snapshot";
import { callLine, textBody, textResult } from "./common";

export function registerStatusTool(
	pi: ExtensionAPI,
	client: TowerClient,
): void {
	const z = pi.zod;
	const parameters = z.object({
		stack: z
			.string()
			.optional()
			.describe(
				"Only this stack: agents named <stack>-*, jobs and schedules tagged stack:<stack>",
			),
	});
	pi.registerTool({
		name: "tower_status",
		label: "Tower Status",
		description:
			"Fleet overview. Lists what needs the operator first (pending inbox items, blocked or dead agents, failed or stuck jobs, jobs on their last attempt, unowned jobs, paused schedules), then every agent with its state and job, the open jobs, the schedules, and the last 24h of results. Read-only. Call it first when asked what tower or the agents are doing.",
		parameters,
		loadMode: "essential",
		approval: "read",
		async execute(_toolCallId, params, signal) {
			const view = buildView(
				await fetchSnapshot(client, { signal }),
				params.stack || undefined,
			);
			return textResult(fleetText(view), view);
		},
		renderCall(args, _options, theme) {
			return callLine(
				"tower status",
				args.stack ? `stack ${args.stack}` : "",
				theme,
			);
		},
		renderResult(result, options, theme) {
			const view: FleetView | undefined = result.details;
			if (result.isError || !view) return textBody(result, options, theme);
			return new Lines((width) => fleetCard(view, theme, width));
		},
	});
}
