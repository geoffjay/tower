/** Plain-text helpers for tool output and session context (no ANSI). */

import type { Message } from "./model";

export function trunc(text: string, max: number): string {
	return text.length <= max ? text : `${text.slice(0, Math.max(0, max - 1))}…`;
}

export function oneLine(text: string): string {
	return text.replace(/\s+/g, " ").trim();
}

/** The message's text parts, joined. */
export function messageText(message: Message): string {
	return message.parts
		.map((part) => part.text)
		.filter(
			(text): text is string => typeof text === "string" && text.length > 0,
		)
		.join("\n");
}

const BORDER = /^[\s│┃║|]+|[\s│┃║|]+$/g;

/**
 * Last `lines` non-blank lines of the pane context tower attaches to a
 * blocked-agent inbox item (`parts[].data.context`), with box borders and
 * wide padding squeezed out.
 */
export function contextTail(
	message: Message,
	lines: number,
): string | undefined {
	for (const part of message.parts) {
		const data = part.data;
		if (typeof data !== "object" || data === null || !("context" in data))
			continue;
		const context = data.context;
		if (typeof context !== "string") continue;
		const kept = context
			.split("\n")
			.map((line) => line.replace(BORDER, "").replace(/ {3,}/g, "  "))
			.filter((line) => line.trim().length > 0);
		return kept.slice(-lines).join("\n") || undefined;
	}
	return undefined;
}

/** A job result as one readable string: `{summary}` and `{error}` unwrap; anything else is JSON. */
export function resultText(result: unknown): string {
	if (result === undefined || result === null) return "";
	if (typeof result === "string") return result;
	if (typeof result === "object") {
		if ("summary" in result && typeof result.summary === "string")
			return result.summary;
		if ("error" in result && typeof result.error === "string")
			return `error: ${result.error}`;
	}
	return JSON.stringify(result);
}

/** Left-aligned columns separated by two spaces; the last column is not padded. */
export function table(header: string[], rows: string[][]): string {
	const all = [header, ...rows];
	const widths = header.map((_, column) =>
		Math.max(...all.map((row) => (row[column] ?? "").length)),
	);
	return all
		.map((row) =>
			row
				.map((cell, column) =>
					column === row.length - 1 ? cell : cell.padEnd(widths[column]),
				)
				.join("  ")
				.trimEnd(),
		)
		.join("\n");
}
