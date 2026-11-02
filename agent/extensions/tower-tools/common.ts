/** Helpers shared by the tower tools: results, argument checks, approval tiers, and TUI renderers. */

import type {
	AgentToolResult,
	ToolApprovalDecision,
} from "@oh-my-pi/pi-agent-core";
import type { ToolRenderResultOptions } from "@oh-my-pi/pi-coding-agent";
import { type Component, Text, type Theme } from "@oh-my-pi/pi-tui";
import type { TowerClient } from "../lib/client";
import type { Agent } from "../lib/model";
import { type AgentNames, agentNames } from "../lib/snapshot";

export function textResult<T>(text: string, details?: T): AgentToolResult<T> {
	return { content: [{ type: "text", text }], details };
}

/** Throw a model-readable error when an action's required argument is missing. */
export function need<T>(
	value: T | undefined,
	field: string,
	action: string,
): T {
	if (value === undefined || value === null || value === "")
		throw new Error(`${action} needs \`${field}\``);
	return value;
}

/** Agent id → name lookup from a fresh agent list. */
export async function fetchNames(
	client: TowerClient,
	signal?: AbortSignal,
): Promise<AgentNames> {
	const { agents } = await client.get<{ agents: Agent[] }>("/v1/agents", {
		signal,
	});
	return agentNames(agents);
}

/**
 * Approval by action: listed read actions run unprompted; `confirm` actions
 * always ask, whatever the approval mode, because they are irreversible or
 * grant an agent permission; everything else is a write.
 */
export function approvalByAction(
	read: readonly string[],
	confirm: Record<string, string> = {},
) {
	return (args: unknown): ToolApprovalDecision => {
		const action =
			typeof args === "object" && args !== null && "action" in args
				? args.action
				: undefined;
		if (typeof action !== "string") return "write";
		if (read.includes(action)) return "read";
		const reason = confirm[action];
		return reason ? { tier: "write", policy: "prompt", reason } : "write";
	};
}
/** One-line call header: `tower_task create "title" → agent`. */
export function callLine(
	label: string,
	detail: string,
	theme: Theme,
): Component {
	return new Text(
		`${theme.fg("toolTitle", theme.bold(label))} ${theme.fg("muted", detail)}`,
		0,
		0,
	);
}

const COLLAPSED_LINES = 16;

/** Text result body, cut to the first lines until the user expands it; errors in red. */
export function textBody(
	result: { content: { type: string; text?: string }[]; isError?: boolean },
	options: ToolRenderResultOptions,
	theme: Theme,
): Component {
	const text = result.content
		.map((block) => (block.type === "text" ? (block.text ?? "") : ""))
		.join("\n")
		.trimEnd();
	const lines = text.split("\n");
	const shown = options.expanded ? lines : lines.slice(0, COLLAPSED_LINES);
	const hidden = lines.length - shown.length;
	const body =
		shown.join("\n") + (hidden > 0 ? `\n… ${hidden} more lines` : "");
	return new Text(
		theme.fg(result.isError ? "error" : "toolOutput", body),
		0,
		0,
	);
}

/** Quote a short string argument for a call header. */
export function quoted(text: string | undefined, max = 60): string {
	if (!text) return "";
	const flat = text.replace(/\s+/g, " ").trim();
	return `"${flat.length > max ? `${flat.slice(0, max - 1)}…` : flat}"`;
}
