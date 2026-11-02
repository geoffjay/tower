/**
 * tower-tools — makes an omp session tower-aware.
 *
 * Registers the tower_status, tower_agent, tower_inbox, tower_task, and
 * tower_schedule tools (top-level, not behind tool discovery) and adds the
 * tower operator block to the main session's system prompt. The tools speak
 * REST to the tower server the `tower` CLI would reach (lib/client.ts).
 *
 * Each tool declares its parameter schema in a const before registerTool:
 * written inline, TypeScript loses the inferred parameter type.
 */

import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import { TowerClient } from "../lib/client";
import { registerAgentTool } from "./agent";
import { registerInboxTool } from "./inbox";
import { withOperatorPrompt } from "./prompt";
import { registerScheduleTool } from "./schedule";
import { registerStatusTool } from "./status";
import { registerTaskTool } from "./task";

export default function towerTools(pi: ExtensionAPI): void {
	pi.setLabel("tower tools");
	const client = new TowerClient();

	registerStatusTool(pi, client);
	registerAgentTool(pi, client);
	registerInboxTool(pi, client);
	registerTaskTool(pi, client);
	registerScheduleTool(pi, client);

	// Re-applied on every prompt: an override governs only until the next preparation.
	pi.on("before_agent_start", async (event, ctx) => {
		if (ctx.agent.kind !== "main") return undefined;
		return { systemPrompt: withOperatorPrompt(event.systemPrompt) };
	});
}
