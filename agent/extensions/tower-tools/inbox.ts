import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { TowerClient } from "../lib/client";
import { relative } from "../lib/format";
import type { Message } from "../lib/model";
import { contextTail, messageText } from "../lib/text";
import {
	approvalByAction,
	callLine,
	need,
	quoted,
	textBody,
	textResult,
} from "./common";

const ACTIONS = [
	"list",
	"answer",
	"approve",
	"deny",
	"dismiss",
	"ask",
] as const;

export function registerInboxTool(pi: ExtensionAPI, client: TowerClient): void {
	const z = pi.zod;
	const parameters = z.object({
		action: z.enum(ACTIONS),
		id: z
			.string()
			.optional()
			.describe("Inbox item (message) id: answer, approve, deny, dismiss"),
		text: z
			.string()
			.optional()
			.describe("answer: the reply. ask: the question"),
		agent: z.string().optional().describe("ask: the agent to ask"),
		deadline_s: z
			.number()
			.int()
			.min(1)
			.optional()
			.describe("ask: seconds before the question expires (default 300)"),
	});
	pi.registerTool({
		name: "tower_inbox",
		label: "Tower Inbox",
		description: [
			"The operator inbox: questions and approvals agents sent to the user. A blocked agent gets one automatically, with its screen attached.",
			"list: pending items with sender, age, deadline, text, and screen context.",
			"answer: type `text` into the agent as the reply (questions; works on approvals too).",
			"approve: press Yes on the agent's permission dialog. deny: refuse it.",
			"dismiss: close a stale item without telling the agent.",
			"ask: send question `text` to `agent`; its answer comes back to the inbox.",
			"Answer, approve, deny, or dismiss only what the user decided: an approval lets an agent act. Unanswered items expire at their deadline (an expired approval is denied).",
		].join(" "),
		parameters,
		loadMode: "essential",
		approval: approvalByAction(["list"], {
			approve:
				"Approving presses Yes on the agent's permission dialog and lets it act.",
		}),
		async execute(_toolCallId, params, signal) {
			const { action } = params;
			const respond = (body: unknown) =>
				client.post<{ message: Message }>(
					`/v1/messages/${encodeURIComponent(need(params.id, "id", action))}/respond`,
					body,
					{ signal, timeoutMs: 60_000 },
				);
			switch (action) {
				case "list": {
					const { messages } = await client.get<{ messages: Message[] }>(
						"/v1/messages?to=me&status=pending",
						{
							signal,
						},
					);
					if (!messages.length)
						return textResult("Inbox empty: nothing waits for the user.");
					const now = Date.now();
					const blocks = messages.map((message) => {
						const head = `[${message.id}] ${message.from_id} ${message.kind}, ${relative(message.created_at, now)}${message.deadline_at ? `, expires ${relative(message.deadline_at, now)}` : ""}${message.task_id ? `, job ${message.task_id}` : ""}`;
						const context = contextTail(message, 15);
						return [
							head,
							messageText(message),
							context ? `screen:\n${context}` : "",
						]
							.filter(Boolean)
							.join("\n");
					});
					return textResult(blocks.join("\n\n"), { pending: messages.length });
				}
				case "answer": {
					const { message } = await respond({
						approve: true,
						parts: [{ text: need(params.text, "text", action) }],
					});
					return textResult(
						`answered ${message.id} → delivered to ${message.from_id}`,
					);
				}
				case "approve":
				case "deny": {
					const { message } = await respond({ approve: action === "approve" });
					return textResult(
						`${action === "approve" ? "approved" : "denied"} ${message.id} → delivered to ${message.from_id}`,
					);
				}
				case "dismiss": {
					const { message } = await client.post<{ message: Message }>(
						`/v1/messages/${encodeURIComponent(need(params.id, "id", action))}/dismiss`,
						undefined,
						{ signal },
					);
					return textResult(
						`dismissed ${message.id} from ${message.from_id} (the agent is not told)`,
					);
				}
				case "ask": {
					const { message } = await client.post<{ message: Message }>(
						"/v1/messages",
						{
							to: need(params.agent, "agent", action),
							to_kind: "agent",
							kind: "question",
							parts: [{ text: need(params.text, "text", action) }],
							deadline_s: params.deadline_s,
						},
						{ signal },
					);
					return textResult(
						`asked ${params.agent} (${message.id}, ${message.status}); the answer arrives in the inbox`,
					);
				}
			}
		},
		renderCall(args, _options, theme) {
			const detail = [
				args.action,
				args.id ?? args.agent ?? "",
				quoted(args.text),
			];
			return callLine("tower inbox", detail.filter(Boolean).join(" "), theme);
		},
		renderResult(result, options, theme) {
			return textBody(result, options, theme);
		},
	});
}
