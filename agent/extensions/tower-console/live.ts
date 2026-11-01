/**
 * Live fleet state for the console. The `/v1/events` stream is the change
 * trigger: any event other than an output chunk schedules a debounced
 * snapshot refresh, and each refresh is diffed against the previous one to
 * produce the activity digest and notifications. A slow periodic refresh
 * heals a lagging or silent stream and keeps relative times fresh.
 */

import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { type TowerClient, TowerError } from "../lib/client";
import { isOpenTask, type Task } from "../lib/model";
import type { LinkState } from "../lib/render";
import {
	agentNames,
	buildView,
	type FleetView,
	fetchSnapshot,
	offlineView,
	type Snapshot,
} from "../lib/snapshot";
import { messageText, oneLine, resultText, trunc } from "../lib/text";

const DEBOUNCE_MS = 400;
const PERIODIC_MS = 60_000;
const BACKOFF_MAX_MS = 30_000;
/** Output chunks dominate the event log and never change fleet state. */
const SKIPPED_KINDS = ["agent.output"];

export interface Change {
	level: "info" | "warn" | "error";
	text: string;
	/** Worth a toast: something that needs the operator. */
	notify: boolean;
}

export interface LiveListener {
	/** The view or the link state changed: re-render. */
	update(): void;
	/** Fleet changes between two successful refreshes. */
	changes(changes: Change[]): void;
}

export class LiveFleet {
	link: LinkState = "connecting";
	snapshot: Snapshot | undefined;
	view: FleetView | undefined;
	/** Settles after the first refresh attempt, successful or not. */
	readonly ready: Promise<void>;

	#client: TowerClient;
	#ctx: ExtensionContext;
	#listener: LiveListener;
	#abort = new AbortController();
	#streamOpen = false;
	#debounce: Timer | undefined;
	#periodic: Timer | undefined;
	#refreshing = false;
	#again = false;
	#markReady: () => void;

	constructor(
		client: TowerClient,
		ctx: ExtensionContext,
		listener: LiveListener,
	) {
		this.#client = client;
		this.#ctx = ctx;
		this.#listener = listener;
		const { promise, resolve } = Promise.withResolvers<void>();
		this.ready = promise;
		this.#markReady = resolve;
	}

	start(): void {
		void this.refresh();
		this.#periodic = this.#ctx.setInterval(
			() => void this.refresh(),
			PERIODIC_MS,
		);
		this.#follow().catch(() => {});
	}

	stop(): void {
		this.#abort.abort();
		if (this.#debounce) this.#ctx.clearTimer(this.#debounce);
		if (this.#periodic) this.#ctx.clearTimer(this.#periodic);
		this.#debounce = undefined;
		this.#periodic = undefined;
	}

	/** Re-read the fleet now; concurrent calls coalesce into one follow-up read. Never throws. */
	async refresh(): Promise<void> {
		if (this.#abort.signal.aborted) return;
		if (this.#refreshing) {
			this.#again = true;
			return;
		}
		this.#refreshing = true;
		try {
			const next = await fetchSnapshot(this.#client, {
				signal: this.#abort.signal,
				timeoutMs: 10_000,
			});
			const previous = this.snapshot;
			this.snapshot = next;
			this.view = buildView(next);
			if (this.link === "offline")
				this.link = this.#streamOpen ? "live" : "connecting";
			if (previous) {
				const changes = diff(previous, next);
				if (changes.length) this.#listener.changes(changes);
			}
		} catch (err) {
			if (this.#abort.signal.aborted) return;
			this.link = "offline";
			const reason =
				err instanceof TowerError || err instanceof Error
					? err.message
					: String(err);
			this.view = offlineView(this.#client.describe(), reason);
		} finally {
			this.#refreshing = false;
			this.#markReady();
		}
		this.#listener.update();
		if (this.#again) {
			this.#again = false;
			void this.refresh();
		}
	}

	#schedule(): void {
		if (this.#debounce || this.#abort.signal.aborted) return;
		this.#debounce = this.#ctx.setTimeout(() => {
			this.#debounce = undefined;
			void this.refresh();
		}, DEBOUNCE_MS);
	}

	async #follow(): Promise<void> {
		let backoff = 1_000;
		const signal = this.#abort.signal;
		while (!signal.aborted) {
			try {
				const stream = this.#client.events({
					signal,
					skipKinds: SKIPPED_KINDS,
					onOpen: () => {
						this.#streamOpen = true;
						backoff = 1_000;
						if (this.link !== "offline") this.link = "live";
						this.#listener.update();
						// The stream starts at the head of the log: re-read what changed while it was down.
						this.#schedule();
					},
				});
				for await (const _batch of stream) this.#schedule();
			} catch {
				// Transport errors fall through to the reconnect below.
			}
			if (signal.aborted) return;
			this.#streamOpen = false;
			if (this.link === "live") {
				this.link = "connecting";
				this.#listener.update();
			}
			const pause = Promise.withResolvers<void>();
			const timer = this.#ctx.setTimeout(() => pause.resolve(), backoff);
			const wake = () => {
				this.#ctx.clearTimer(timer);
				pause.resolve();
			};
			signal.addEventListener("abort", wake, { once: true });
			await pause.promise;
			signal.removeEventListener("abort", wake);
			backoff = Math.min(backoff * 2, BACKOFF_MAX_MS);
		}
	}
}

/** What changed between two snapshots, in operator terms. */
export function diff(previous: Snapshot, next: Snapshot): Change[] {
	const changes: Change[] = [];
	const name = agentNames([...previous.agents, ...next.agents]);

	const pending = new Set(previous.inbox.map((message) => message.id));
	for (const message of next.inbox) {
		if (pending.has(message.id)) continue;
		changes.push({
			level: "warn",
			text: `inbox: ${message.from_id} ${message.kind} [${message.id}]: "${trunc(oneLine(messageText(message)), 160)}"`,
			notify: true,
		});
	}

	const before = new Map(previous.agents.map((agent) => [agent.id, agent]));
	for (const agent of next.agents) {
		const was = before.get(agent.id);
		if (!was) {
			changes.push({
				level: "info",
				text: `agent ${agent.name} added (${agent.kind}, ${agent.state})`,
				notify: false,
			});
			continue;
		}
		if (was.state === agent.state) continue;
		const notable =
			agent.state === "blocked" ||
			agent.state === "dead" ||
			was.state === "dead" ||
			was.state === "launching";
		if (!notable) continue;
		changes.push({
			level:
				agent.state === "dead"
					? "error"
					: agent.state === "blocked"
						? "warn"
						: "info",
			text: `${agent.name}: ${was.state} → ${agent.state}`,
			notify: agent.state === "dead",
		});
	}
	const remaining = new Set(next.agents.map((agent) => agent.id));
	for (const agent of previous.agents) {
		if (!remaining.has(agent.id))
			changes.push({
				level: "info",
				text: `agent ${agent.name} removed`,
				notify: false,
			});
	}

	const known = new Map(
		[...previous.recent, ...previous.open].map((task) => [task.id, task.state]),
	);
	const current = new Map<string, Task>(
		[...next.recent, ...next.open].map((task) => [task.id, task]),
	);
	for (const task of current.values()) {
		const was = known.get(task.id);
		if (was === task.state) continue;
		const title = `"${trunc(task.title, 60)}"`;
		const owner = name(task.owner_id);
		if (was === undefined && isOpenTask(task.state)) {
			const where = owner
				? `assigned to ${owner}`
				: task.target_agent_id
					? `reserved for ${name(task.target_agent_id)}`
					: "queued without an owner";
			changes.push({
				level: "info",
				text: `job ${task.id} ${title} created, ${where}`,
				notify: false,
			});
			continue;
		}
		switch (task.state) {
			case "completed":
				changes.push({
					level: "info",
					text: `job ${task.id} ${title} completed by ${owner ?? "?"}: ${trunc(oneLine(resultText(task.result)), 200)}`,
					notify: false,
				});
				break;
			case "failed":
				changes.push({
					level: "error",
					text: `job ${task.id} ${title} failed (${owner ?? "no owner"}): ${trunc(oneLine(resultText(task.result)), 200)}`,
					notify: true,
				});
				break;
			case "canceled":
			case "rejected":
				changes.push({
					level: "info",
					text: `job ${task.id} ${title} ${task.state}`,
					notify: false,
				});
				break;
			case "assigned":
				changes.push({
					level: "info",
					text: `job ${task.id} ${title} assigned to ${owner ?? "?"}`,
					notify: false,
				});
				break;
			case "input-required":
				changes.push({
					level: "warn",
					text: `job ${task.id} ${title} waits for input (${owner ?? "?"})`,
					notify: false,
				});
				break;
			case "queued":
				changes.push({
					level: "warn",
					text: `job ${task.id} ${title} is back in the queue (attempt ${task.attempt_count}/${task.max_attempts})`,
					notify: false,
				});
				break;
		}
	}
	return changes;
}
