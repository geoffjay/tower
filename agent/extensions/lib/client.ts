/**
 * Tower REST + SSE client (design D§7).
 *
 * Server discovery mirrors the `tower` CLI (crates/tower-client): `TOWER_URL`,
 * else `[server] bind_tcp` from the tower home's config.toml, else
 * 127.0.0.1:8266; the bearer token comes from `TOWER_TOKEN`, else the token
 * file. The tower home is `TOWER_HOME`, else the platform dirs
 * (`~/Library/Application Support/tower` on macOS; `$XDG_CONFIG_HOME/tower`
 * and `$XDG_DATA_HOME/tower` on Linux).
 *
 * REST callers are always the operator ("me"): the server reads agent
 * identity only on `/mcp`.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { TowerEvent } from "./model";

const DEFAULT_BIND = "127.0.0.1:8266";
const REQUEST_TIMEOUT_MS = 30_000;
/** The server sends an SSE keep-alive every 15s; silence past this means a dead peer. */
const STREAM_IDLE_MS = 45_000;

export interface TowerEndpoint {
	baseUrl: string;
	token: string;
	tokenSource: string;
}

/** A failed tower call: the server's error envelope, or a transport failure (`code: "unreachable"`). */
export class TowerError extends Error {
	constructor(
		message: string,
		readonly code: string,
		readonly status?: number,
		readonly detail?: unknown,
	) {
		super(message);
		this.name = "TowerError";
	}
}

function homeFiles(env: NodeJS.ProcessEnv): {
	configFile: string;
	tokenFile: string;
} {
	if (env.TOWER_HOME) {
		return {
			configFile: path.join(env.TOWER_HOME, "config.toml"),
			tokenFile: path.join(env.TOWER_HOME, "token"),
		};
	}
	if (process.platform === "darwin") {
		const dir = path.join(
			os.homedir(),
			"Library",
			"Application Support",
			"tower",
		);
		return {
			configFile: path.join(dir, "config.toml"),
			tokenFile: path.join(dir, "token"),
		};
	}
	const configHome = env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
	const dataHome =
		env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
	return {
		configFile: path.join(configHome, "tower", "config.toml"),
		tokenFile: path.join(dataHome, "tower", "token"),
	};
}

/** `[server] bind_tcp` from config.toml; a wildcard bind is reached on loopback. */
function configuredBind(configFile: string): string {
	let text: string;
	try {
		text = fs.readFileSync(configFile, "utf8");
	} catch {
		return DEFAULT_BIND;
	}
	let parsed: unknown;
	try {
		parsed = Bun.TOML.parse(text);
	} catch (err) {
		throw new TowerError(
			`cannot parse ${configFile}: ${err instanceof Error ? err.message : String(err)}`,
			"config",
		);
	}
	const server =
		typeof parsed === "object" && parsed !== null && "server" in parsed
			? parsed.server
			: undefined;
	const bind =
		typeof server === "object" && server !== null && "bind_tcp" in server
			? server.bind_tcp
			: undefined;
	const value =
		typeof bind === "string" && bind.trim() ? bind.trim() : DEFAULT_BIND;
	return value.replace("0.0.0.0:", "127.0.0.1:").replace("[::]:", "127.0.0.1:");
}

/** Resolve the server address and token the same way the `tower` CLI does. */
export function resolveEndpoint(
	env: NodeJS.ProcessEnv = process.env,
): TowerEndpoint {
	const { configFile, tokenFile } = homeFiles(env);
	const baseUrl = (
		env.TOWER_URL || `http://${configuredBind(configFile)}`
	).replace(/\/+$/, "");
	if (env.TOWER_TOKEN)
		return {
			baseUrl,
			token: env.TOWER_TOKEN.trim(),
			tokenSource: "TOWER_TOKEN",
		};
	let token: string;
	try {
		token = fs.readFileSync(tokenFile, "utf8").trim();
	} catch {
		throw new TowerError(
			`cannot read the tower token file ${tokenFile} (is the server started? otherwise set TOWER_TOKEN or TOWER_HOME)`,
			"unauthorized",
		);
	}
	if (!token)
		throw new TowerError(
			`the tower token file ${tokenFile} is empty`,
			"unauthorized",
		);
	return { baseUrl, token, tokenSource: tokenFile };
}

export interface RequestOptions {
	signal?: AbortSignal;
	timeoutMs?: number;
}

/** Follows the stream live from the head of the log; a reconnect re-reads state rather than replaying it. */
export interface StreamOptions {
	signal: AbortSignal;
	/** Event kinds dropped before JSON parsing (e.g. `agent.output`, the bulk of the log). */
	skipKinds?: readonly string[];
	/** Called once the server accepted the stream. */
	onOpen?: () => void;
}

export class TowerClient {
	#endpoint: TowerEndpoint | undefined;

	/** The resolved endpoint; throws a `TowerError` when the token cannot be found. */
	endpoint(): TowerEndpoint {
		this.#endpoint ??= resolveEndpoint();
		return this.#endpoint;
	}

	/** Base URL for display, without failing when the token is missing. */
	describe(): string {
		try {
			return this.endpoint().baseUrl;
		} catch {
			return process.env.TOWER_URL || `http://${DEFAULT_BIND}`;
		}
	}

	get<T>(route: string, options?: RequestOptions): Promise<T> {
		return this.request<T>("GET", route, undefined, options);
	}

	post<T>(route: string, body?: unknown, options?: RequestOptions): Promise<T> {
		return this.request<T>("POST", route, body ?? {}, options);
	}

	patch<T>(route: string, body: unknown, options?: RequestOptions): Promise<T> {
		return this.request<T>("PATCH", route, body, options);
	}

	delete<T>(route: string, options?: RequestOptions): Promise<T> {
		return this.request<T>("DELETE", route, undefined, options);
	}

	/**
	 * One JSON call. The result is typed by the caller: the server's
	 * documented response shapes (D§7) are this client's contract.
	 */
	async request<T>(
		method: string,
		route: string,
		body?: unknown,
		options: RequestOptions = {},
	): Promise<T> {
		for (let attempt = 0; ; attempt++) {
			const endpoint = this.endpoint();
			const signals = [
				AbortSignal.timeout(options.timeoutMs ?? REQUEST_TIMEOUT_MS),
			];
			if (options.signal) signals.push(options.signal);
			let response: Response;
			try {
				response = await fetch(endpoint.baseUrl + route, {
					method,
					headers: {
						authorization: `Bearer ${endpoint.token}`,
						accept: "application/json",
						...(body === undefined
							? {}
							: { "content-type": "application/json" }),
					},
					body: body === undefined ? undefined : JSON.stringify(body),
					signal: AbortSignal.any(signals),
				});
			} catch (err) {
				throw transportError(err, endpoint.baseUrl, options.signal);
			}
			// A rotated token file: re-read it once before giving up.
			if (
				response.status === 401 &&
				attempt === 0 &&
				endpoint.tokenSource !== "TOWER_TOKEN"
			) {
				this.#endpoint = undefined;
				continue;
			}
			const parsed = await parseResponse(response, method, route);
			return parsed as T;
		}
	}

	/**
	 * Follow `/v1/events` from the head of the log. The server batches events
	 * into one `tower-batch` frame per poll, one JSON event per `data:` line;
	 * this yields each frame's events that survive `skipKinds`, when any do.
	 * Returns when the server closes the stream; throws on transport errors and
	 * when no byte (keep-alives included) arrives for 45s.
	 */
	async *events(options: StreamOptions): AsyncGenerator<TowerEvent[]> {
		const endpoint = this.endpoint();
		const idle = new AbortController();
		const signal = AbortSignal.any([options.signal, idle.signal]);
		const skip = (options.skipKinds ?? []).map((kind) => `"kind":"${kind}"`);
		let response: Response;
		try {
			response = await fetch(`${endpoint.baseUrl}/v1/events`, {
				headers: {
					authorization: `Bearer ${endpoint.token}`,
					accept: "text/event-stream",
				},
				signal,
			});
		} catch (err) {
			throw transportError(err, endpoint.baseUrl, options.signal);
		}
		if (!response.ok || !response.body) {
			if (response.status === 401) this.#endpoint = undefined;
			throw new TowerError(
				`event stream failed: HTTP ${response.status}`,
				"stream",
				response.status,
			);
		}
		options.onOpen?.();
		const reader = response.body.getReader();
		const decoder = new TextDecoder();
		let watchdog = setTimeout(() => idle.abort(), STREAM_IDLE_MS);
		let buffer = "";
		let batch: TowerEvent[] = [];
		try {
			for (;;) {
				const { done, value } = await reader.read();
				if (done) return;
				clearTimeout(watchdog);
				watchdog = setTimeout(() => idle.abort(), STREAM_IDLE_MS);
				buffer += decoder.decode(value, { stream: true });
				let newline = buffer.indexOf("\n");
				while (newline !== -1) {
					const line = buffer.slice(0, newline).replace(/\r$/, "");
					buffer = buffer.slice(newline + 1);
					newline = buffer.indexOf("\n");
					if (line.startsWith("data:")) {
						const data = line.slice(5).trimStart();
						if (data && !skip.some((marker) => data.includes(marker))) {
							const event = parseEvent(data);
							if (event) batch.push(event);
						}
					} else if (line === "") {
						if (batch.length) yield batch;
						batch = [];
					}
					// `id:`, `event:`, and `:` keep-alive lines carry nothing this client keeps.
				}
			}
		} catch (err) {
			if (idle.signal.aborted && !options.signal.aborted) {
				throw new TowerError("event stream went silent", "stream");
			}
			throw err;
		} finally {
			clearTimeout(watchdog);
			reader.cancel().catch(() => {});
		}
	}
}

/** One `data:` line as an event; a torn or foreign line yields nothing (the next snapshot refresh heals any gap). */
function parseEvent(line: string): TowerEvent | undefined {
	let event: unknown;
	try {
		event = JSON.parse(line);
	} catch {
		return undefined;
	}
	if (
		typeof event !== "object" ||
		event === null ||
		!("kind" in event) ||
		typeof event.kind !== "string"
	) {
		return undefined;
	}
	const checked: TowerEvent = event as TowerEvent; // the event log row shape (D§5.4)
	return checked;
}

function transportError(
	err: unknown,
	baseUrl: string,
	caller?: AbortSignal,
): Error {
	if (caller?.aborted)
		return err instanceof Error ? err : new Error(String(err));
	if (err instanceof Error && err.name === "TimeoutError") {
		return new TowerError(
			`tower server at ${baseUrl} did not answer in time`,
			"timeout",
		);
	}
	const reason = err instanceof Error ? err.message : String(err);
	return new TowerError(
		`tower server unreachable at ${baseUrl} (${reason}). Start it with \`tower serve\` or \`tower service install\`.`,
		"unreachable",
	);
}

async function parseResponse(
	response: Response,
	method: string,
	route: string,
): Promise<unknown> {
	const text = await response.text();
	let parsed: unknown;
	try {
		parsed = text ? JSON.parse(text) : {};
	} catch {
		if (response.status === 404 || response.status === 405)
			throw unsupported(method, route, response.status);
		throw new TowerError(
			`non-JSON response (${response.status}) from ${method} ${route}: ${text.slice(0, 200)}`,
			"invalid",
			response.status,
		);
	}
	if (response.ok) return parsed;
	const envelope =
		typeof parsed === "object" && parsed !== null && "error" in parsed
			? parsed.error
			: undefined;
	if (typeof envelope !== "object" || envelope === null) {
		if (response.status === 404 || response.status === 405)
			throw unsupported(method, route, response.status);
		throw new TowerError(
			`HTTP ${response.status} from ${method} ${route}`,
			"error",
			response.status,
		);
	}
	const code =
		"code" in envelope && typeof envelope.code === "string"
			? envelope.code
			: "error";
	const reason =
		"message" in envelope && typeof envelope.message === "string"
			? envelope.message
			: "";
	const detail = "detail" in envelope ? envelope.detail : undefined;
	let message = `${code}: ${reason || `HTTP ${response.status}`}`;
	if (detail !== undefined && detail !== null)
		message += ` (detail: ${JSON.stringify(detail)})`;
	if (code === "unauthorized")
		message += ". Check TOWER_TOKEN or the tower token file.";
	throw new TowerError(message, code, response.status, detail);
}

function unsupported(
	method: string,
	route: string,
	status: number,
): TowerError {
	return new TowerError(
		`this tower server does not support ${method} ${route.split("?")[0]} (HTTP ${status}); upgrade tower`,
		"unsupported",
		status,
	);
}
