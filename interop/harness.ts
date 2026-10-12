// What every interop driver needs: the certificate, the relay, the browser
// with its pages, the Rust clients, and the teardown of all of them. `run.ts`
// and `disconnect.ts` are the drivers.

import {
	type ChildProcessWithoutNullStreams,
	execFileSync,
	spawn,
} from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { type Browser, chromium, type Page } from "playwright-core";
import type { XmoqPage } from "./xmoq-page.ts";

// The functions passed to `page.evaluate` run in the page, where this global
// exists. This file is checked without the DOM lib, so it is declared here.
declare const window: { readonly xmoq: XmoqPage };

const here = import.meta.dir;
const moqtailRoot = join(here, "peers", "moqtail");
const certDir = join(here, ".cert");
const certFile = join(certDir, "cert.pem");
const keyFile = join(certDir, "key.pem");
const logDir = join(here, ".logs");

// Provisional names, the same as the Phase 0 spike.
export const NAMESPACE = "xmoq/phase0";
export const TRACK = "object";
export const STEP_TIMEOUT_MS = 20_000;
/**
 * How long a subscriber may take to learn that its publisher is gone.
 * Provisional, like the idle timeout of the own relay that it has to cover:
 * a client that was killed, or that exits before its close is sent, says
 * nothing, and the relay notices 6 to 9 seconds later (measured).
 */
export const DISCONNECT_TIMEOUT_MS = 30_000;
const KILL_GRACE_MS = 5_000;

export type BrowserClient = "own-browser" | "moqtail-browser";
export type RustClient = "own-rust" | "moqtail-rust";
export type Client = BrowserClient | RustClient;

/** Path each browser client is served under, and the entry point bundled for it. */
const BROWSER_PAGES: Record<
	BrowserClient,
	{ path: string; entrypoint: string }
> = {
	"own-browser": { path: "/own/", entrypoint: join(here, "page.ts") },
	"moqtail-browser": {
		path: "/moqtail/",
		entrypoint: join(moqtailRoot, "browser", "page.ts"),
	},
};

/**
 * Every binary here takes the flags and prints the `PUBLISHED`, `SENT`,
 * `SUBSCRIBED` and `OBJECT` lines of crates/moqt-client/src/report.rs, and
 * sends its Object when it reads a line on stdin. Closing stdin makes a
 * publisher leave: the own client ends its Track with PUBLISH_DONE first, the
 * moqtail one closes the session.
 */
const RUST_CLIENT_BINARIES: Record<RustClient, string> = {
	"own-rust": join(here, "..", "target", "debug", "xmoq-moqt-client"),
	"moqtail-rust": join(
		moqtailRoot,
		"rust-client",
		"target",
		"debug",
		"xmoq-interop-moqtail-peer",
	),
};

const OWN_CLIENTS: ReadonlySet<Client> = new Set<Client>([
	"own-browser",
	"own-rust",
]);

export function isOwnClient(client: Client): boolean {
	return OWN_CLIENTS.has(client);
}

/** How a relay is started, and the line it prints once it accepts sessions. */
interface Relay {
	binary: string;
	args(port: string): string[];
	/** Capture group 1 is the version the relay reports. */
	ready: RegExp;
	verboseEnv: Record<string, string>;
}

const RELAYS: Record<string, Relay> = {
	moqtail: {
		binary: join(moqtailRoot, ".tools", "bin", "relay"),
		args: (port) => [
			"--port",
			port,
			"--host",
			"127.0.0.1",
			"--cert-file",
			certFile,
			"--key-file",
			keyFile,
			"--log-folder",
			logDir,
		],
		ready: /(moqtail\/\S+) is running on/,
		verboseEnv: { RUST_LOG: "debug" },
	},
	own: {
		binary: join(here, "..", "target", "debug", "xmoq-relay"),
		args: (port) => [
			"--port",
			port,
			"--cert-file",
			certFile,
			"--key-file",
			keyFile,
		],
		ready: /^LISTENING (\S+) port=/,
		// It always prints one line for each thing it does.
		verboseEnv: {},
	},
};

function relayNamed(name: string): Relay {
	const relay = RELAYS[name];
	if (relay === undefined) throw new Error(`unknown relay ${name}`);
	return relay;
}

export function isBrowserClient(value: string): value is BrowserClient {
	return Object.hasOwn(BROWSER_PAGES, value);
}

export function isRustClient(value: string): value is RustClient {
	return Object.hasOwn(RUST_CLIENT_BINARIES, value);
}

export function parseClient(option: string, value: string): Client {
	if (!isBrowserClient(value) && !isRustClient(value))
		throw new Error(`unknown client ${value} for --${option}`);
	return value;
}

export function log(source: string, line: string): void {
	console.log(`[${source}] ${line}`);
}

export function withTimeout<T>(
	promise: Promise<T>,
	what: string,
	timeoutMs: number = STEP_TIMEOUT_MS,
): Promise<T> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timer = setTimeout(
			() => reject(new Error(`timed out waiting for ${what}`)),
			timeoutMs,
		);
	});
	return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/**
 * Looks every 20 ms until `look` returns something, and fails like
 * `withTimeout` when it has not by then.
 *
 * WHY it stops looking at the limit: a loop that went on would keep the
 * process alive after the run has failed, and the run would end at the limit
 * of whoever started it and not with its own message.
 */
export async function pollFor<T>(
	// A page is asked with a call that takes time, so `look` may be async.
	look: () => T | undefined | Promise<T | undefined>,
	what: string,
	timeoutMs: number = STEP_TIMEOUT_MS,
): Promise<T> {
	const deadline = performance.now() + timeoutMs;
	for (;;) {
		const found = await look();
		if (found !== undefined) return found;
		if (performance.now() >= deadline) {
			throw new Error(`timed out waiting for ${what}`);
		}
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
}

export function captureGroup(match: RegExpMatchArray, index: number): string {
	const value = match[index];
	if (value === undefined)
		throw new Error(`"${match[0]}" has no capture group ${index}`);
	return value;
}

type Cleanup = () => Promise<unknown>;

const cleanups: Cleanup[] = [];
let cleanedUp: Promise<void> | undefined;

/** Registers what undoes a step. Cleanups run in reverse order of registration. */
function defer(cleanup: Cleanup): void {
	cleanups.push(cleanup);
}

/**
 * WHY one shared promise: a signal can arrive while the driver is already
 * cleaning up, and the two callers must not stop the same process twice.
 * WHY a loop instead of a snapshot: after a signal, the driver keeps running
 * until its next await fails and can still register a cleanup in the meantime.
 */
export function runCleanups(): Promise<void> {
	cleanedUp ??= (async () => {
		for (
			let cleanup = cleanups.pop();
			cleanup !== undefined;
			cleanup = cleanups.pop()
		) {
			await cleanup().catch((error: unknown) =>
				log("driver", `cleanup failed: ${String(error)}`),
			);
		}
	})();
	return cleanedUp;
}

const SIGNAL_EXIT_CODES = { SIGINT: 130, SIGTERM: 143 } as const;

/**
 * Makes a SIGINT or SIGTERM to the driver, and a failure nothing caught, stop
 * everything the driver started.
 *
 * WHY `once` for the signals: a second signal then gets the default action
 * and ends the driver at once, which is the way out if a cleanup hangs.
 * WHY the uncaught failures: Bun ends the process on them without running
 * any `finally` (1.3.12, and 1.3.14 the same), and the relay stayed behind
 * with its port.
 */
export function stopEverythingOnExit(): void {
	for (const signal of ["SIGINT", "SIGTERM"] as const) {
		process.once(signal, () => {
			log("driver", `received ${signal}, stopping`);
			void runCleanups().finally(() => process.exit(SIGNAL_EXIT_CODES[signal]));
		});
	}
	for (const event of ["uncaughtException", "unhandledRejection"] as const) {
		process.once(event, (error: unknown) => {
			log(
				"driver",
				`FAILED ${event}: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`,
			);
			void runCleanups().finally(() => process.exit(1));
		});
	}
}

/**
 * Chromium accepts a certificate through `serverCertificateHashes` only if it
 * is ECDSA P-256 and valid for less than two weeks, so the certificate is
 * regenerated whenever it is missing or about to expire.
 */
export function ensureCertificate(): { sha256Hex: string; validTo: string } {
	const oneDayMs = 24 * 60 * 60 * 1000;
	const usable =
		existsSync(certFile) &&
		existsSync(keyFile) &&
		new Date(new X509Certificate(readFileSync(certFile)).validTo).getTime() -
			Date.now() >
			oneDayMs;
	if (!usable) {
		mkdirSync(certDir, { recursive: true });
		// WHY two steps instead of `req -newkey ec`: with LibreSSL 3.3.6 the
		// one-step form wrote a key the relay rejected (recorded in
		// docs/experiments/2026-10-10-moqtail-object-transfer.md).
		execFileSync(
			"openssl",
			["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", keyFile],
			{ stdio: ["ignore", "ignore", "pipe"] },
		);
		execFileSync(
			"openssl",
			[
				"req",
				"-x509",
				"-new",
				"-key",
				keyFile,
				"-out",
				certFile,
				"-days",
				"13",
				"-subj",
				"/CN=localhost",
				"-addext",
				"subjectAltName=DNS:localhost,IP:127.0.0.1",
			],
			{ stdio: ["ignore", "ignore", "pipe"] },
		);
		log("driver", `generated ${certFile}`);
	}
	const certificate = new X509Certificate(readFileSync(certFile));
	return {
		sha256Hex: createHash("sha256").update(certificate.raw).digest("hex"),
		validTo: certificate.validTo,
	};
}

export interface LoggedProcess {
	child: ChildProcessWithoutNullStreams;
	exited: Promise<number | null>;
	/** The first line that matches, among the lines printed so far and later. */
	waitForLine(pattern: RegExp, timeoutMs?: number): Promise<RegExpMatchArray>;
	/** The matches among the lines printed so far, in the order printed. */
	matches(pattern: RegExp): RegExpMatchArray[];
}

/**
 * A line of the relay that comes after everything a check is about: the
 * `nth` line that matches, the first where `nth` is left out.
 */
export interface Fence {
	line: RegExp;
	nth?: number;
}

/**
 * The relay, and what a driver may ask of its log.
 *
 * WHY there is no `matches` here, as a client's process has: a count of the
 * relay's lines "so far" is a count of the lines the driver has read, and
 * the lines of two processes reach the driver in no order. A check that
 * waited for a client's line and then counted the relay's failed in CI three
 * times, each time for a line that was on its way. The relay writes one
 * stream of lines, so every line before one that was read has been read:
 * `linesBefore` waits for a line the caller names, the fence, and counts
 * what stands before it. What is counted no longer depends on when the
 * driver looks.
 */
export interface RelayLog extends Omit<LoggedProcess, "matches"> {
	/** The first `count` lines that match, once the relay has printed that many. */
	waitForLines(
		pattern: RegExp,
		count: number,
		timeoutMs?: number,
	): Promise<RegExpMatchArray[]>;
	/**
	 * The lines that match `pattern` among those the relay printed before the
	 * fence. Waits for the fence, and fails when it does not come.
	 *
	 * The caller has to name a fence the relay cannot print before a line the
	 * check is about: one that something the driver did afterwards makes it
	 * print, or one the relay prints when it has dealt with what went before.
	 */
	linesBefore(
		fence: Fence,
		pattern: RegExp,
		timeoutMs?: number,
	): Promise<RegExpMatchArray[]>;
	/**
	 * Whether a line that matches has been read. For waiting on the relay
	 * only: true is final, and false says nothing about what the relay did.
	 */
	hasPrinted(pattern: RegExp): boolean;
}

/** Spawns a child, echoes its output, and lets callers wait for a matching line. */
function spawnLogged(
	source: string,
	command: string,
	args: string[],
	env: Record<string, string> = {},
): LoggedProcess & RelayLog {
	const child = spawn(command, args, { env: { ...process.env, ...env } });
	const lines: string[] = [];
	const waiters: {
		pattern: RegExp;
		resolve: (match: RegExpMatchArray) => void;
	}[] = [];
	const exited = new Promise<number | null>((resolve) =>
		child.once("exit", resolve),
	);
	/** Called after every line, by those that wait for more than one line. */
	const lookers: (() => void)[] = [];
	const died = (what: string): Promise<never> =>
		exited.then((code): never => {
			throw new Error(`${source} exited with code ${code} before ${what}`);
		});
	/** The index of the `nth` line that matches, once it is there. */
	const indexOfNth = (
		pattern: RegExp,
		nth: number,
		timeoutMs?: number,
	): Promise<number> => {
		const find = (): number | undefined => {
			let seen = 0;
			for (const [index, line] of lines.entries()) {
				if (!pattern.test(line)) continue;
				seen += 1;
				if (seen === nth) return index;
			}
			return undefined;
		};
		const found = find();
		if (found !== undefined) return Promise.resolve(found);
		const what = `printing ${pattern} ${nth} time(s)`;
		let look: () => void = () => {};
		const arrived = new Promise<number>((resolve) => {
			look = () => {
				const index = find();
				if (index !== undefined) resolve(index);
			};
			lookers.push(look);
		});
		return withTimeout(
			Promise.race([arrived, died(what)]),
			`${source} ${what}`,
			timeoutMs,
		).finally(() => lookers.splice(lookers.indexOf(look), 1));
	};
	const matchesIn = (some: string[], pattern: RegExp): RegExpMatchArray[] =>
		some.flatMap((line) => {
			const match = line.match(pattern);
			return match === null ? [] : [match];
		});

	const onLine = (line: string): void => {
		// Strip the ANSI colors `tracing` adds, so the patterns can match.
		// biome-ignore lint/suspicious/noControlCharactersInRegex: ESC starts every color sequence
		const clean = line.replace(/\u001b\[[0-9;]*m/g, "");
		lines.push(clean);
		log(source, clean);
		for (const look of [...lookers]) look();
		for (const waiter of [...waiters]) {
			const match = clean.match(waiter.pattern);
			if (match) {
				waiters.splice(waiters.indexOf(waiter), 1);
				waiter.resolve(match);
			}
		}
	};
	for (const stream of [child.stdout, child.stderr]) {
		let buffered = "";
		stream.setEncoding("utf8");
		stream.on("data", (chunk: string) => {
			buffered += chunk;
			const parts = buffered.split("\n");
			buffered = parts.pop() ?? "";
			for (const part of parts) if (part.length > 0) onLine(part);
		});
	}

	return {
		child,
		exited,
		waitForLine(pattern, timeoutMs) {
			for (const line of lines) {
				const match = line.match(pattern);
				if (match) return Promise.resolve(match);
			}
			const matched = new Promise<RegExpMatchArray>((resolve) =>
				waiters.push({ pattern, resolve }),
			);
			return withTimeout(
				Promise.race([matched, died(`printing ${pattern}`)]),
				`${source} to print ${pattern}`,
				timeoutMs,
			);
		},
		matches: (pattern) => matchesIn(lines, pattern),
		async waitForLines(pattern, count, timeoutMs) {
			const last = await indexOfNth(pattern, count, timeoutMs);
			return matchesIn(lines.slice(0, last + 1), pattern);
		},
		async linesBefore(fence, pattern, timeoutMs) {
			const at = await indexOfNth(fence.line, fence.nth ?? 1, timeoutMs);
			return matchesIn(lines.slice(0, at), pattern);
		},
		hasPrinted: (pattern) => lines.some((line) => pattern.test(line)),
	};
}

/** Asks a child to stop, and kills it if it has not exited after the grace period. */
async function stopProcess(
	target: LoggedProcess,
	signal: "SIGINT" | "SIGTERM",
): Promise<void> {
	target.child.kill(signal);
	const escalation = setTimeout(
		() => target.child.kill("SIGKILL"),
		KILL_GRACE_MS,
	);
	await target.exited;
	clearTimeout(escalation);
}

/** Bundles the pages the run needs and serves them from a throwaway HTTP server. */
async function servePages(clients: BrowserClient[]): Promise<string> {
	const html = readFileSync(join(here, "index.html"), "utf8");
	const files = new Map<string, { type: string; body: string }>();
	for (const client of clients) {
		const page = BROWSER_PAGES[client];
		// WHY no `outdir`: drivers run at the same time, and a bundle written
		// to a shared path could be read half-written by another one.
		const build = await Bun.build({
			entrypoints: [page.entrypoint],
			target: "browser",
			// WHY: Bun otherwise throws an error whose message is only "Bundle
			// failed" (1.3.12, and 1.3.14 the same); the reasons are in
			// `build.logs`.
			throw: false,
		});
		const [bundle] = build.outputs;
		if (!build.success || bundle === undefined)
			throw new Error(
				`bundling ${page.entrypoint} failed: ${build.logs.join("\n")}`,
			);
		files.set(page.path, { type: "text/html", body: html });
		files.set(`${page.path}page.js`, {
			type: "text/javascript",
			body: await bundle.text(),
		});
	}
	const server = Bun.serve({
		hostname: "127.0.0.1",
		port: 0,
		fetch(request) {
			const file = files.get(new URL(request.url).pathname);
			if (file === undefined) return new Response(null, { status: 404 });
			return new Response(file.body, {
				headers: { "content-type": file.type },
			});
		},
	});
	defer(() => server.stop(true));
	// WHY localhost: it is a secure context, which WebTransport requires.
	return `http://localhost:${server.port}`;
}

/** Serves the pages and launches Chromium, when a client of the run is a page. */
async function startBrowser(
	clients: BrowserClient[],
): Promise<{ origin: string; browser: Browser } | undefined> {
	if (clients.length === 0) return undefined;
	const origin = await servePages(clients);
	const browser = await chromium.launch({
		headless: true,
		channel: "chromium",
		// WHY off: `stopEverythingOnExit` owns the teardown. The SIGINT handler
		// of playwright-core 1.64.0 closes the browser and then calls
		// process.exit(130), which would skip stopping the relay and the Rust
		// clients.
		handleSIGINT: false,
		handleSIGTERM: false,
	});
	defer(() => browser.close());
	log("driver", `browser=chromium version=${browser.version()}`);
	return { origin, browser };
}

/** A started relay and what every client needs to reach it. */
export interface RunContext {
	relayUrl: string;
	relay: RelayLog;
	relayVersion: string;
	certSha256Hex: string;
	/** Absent in a run between Rust clients, which starts no browser. */
	pages: { origin: string; browser: Browser } | undefined;
	/** Makes `own-rust` print the bytes of every stream. */
	verbose: boolean;
	/** Stops the relay. Its sessions end, and its port is free. */
	stopRelay(): Promise<void>;
	/** Starts the relay anew on the same port, with the same certificate; `relay` is the new one from then on. */
	startRelayAnew(): Promise<void>;
}

/** The binaries a run needs and that are not built yet. */
function missingBinaries(relay: string, clients: Client[]): string[] {
	const rustBinaries = clients
		.filter(isRustClient)
		.map((client) => RUST_CLIENT_BINARIES[client]);
	return [relayNamed(relay).binary, ...new Set(rustBinaries)].filter(
		(binary) => !existsSync(binary),
	);
}

/**
 * Creates the certificate, starts the relay, and launches the browser when one
 * of `clients` is a page. Everything started is stopped by `runCleanups`.
 */
export async function startRun(setup: {
	relay: string;
	port: string;
	clients: Client[];
	verbose: boolean;
}): Promise<RunContext> {
	const missing = missingBinaries(setup.relay, setup.clients);
	if (missing.length > 0) {
		throw new Error(
			`${missing.join(", ")} is missing; see ${join(here, "README.md")} for the build steps`,
		);
	}
	const certificate = ensureCertificate();
	log(
		"driver",
		`certificate sha256=${certificate.sha256Hex} validTo=${certificate.validTo}`,
	);
	const config = relayNamed(setup.relay);
	const startRelay = (): ReturnType<typeof spawnLogged> =>
		spawnLogged(
			"relay",
			config.binary,
			config.args(setup.port),
			setup.verbose ? config.verboseEnv : {},
		);
	let relay = startRelay();
	// The one that runs at the end: a run can start it anew.
	defer(() => stopProcess(relay, "SIGINT"));
	const relayVersion = captureGroup(await relay.waitForLine(config.ready), 1);
	const pages = await startBrowser([
		...new Set(setup.clients.filter(isBrowserClient)),
	]);
	const context: RunContext = {
		relayUrl: `https://127.0.0.1:${setup.port}`,
		relay,
		relayVersion,
		certSha256Hex: certificate.sha256Hex,
		pages,
		verbose: setup.verbose,
		stopRelay: () => stopProcess(relay, "SIGINT"),
		async startRelayAnew() {
			relay = startRelay();
			context.relay = relay;
			await relay.waitForLine(config.ready);
		},
	};
	return context;
}

const QUIET_PEER_BINARY = join(
	here,
	"..",
	"target",
	"debug",
	"xmoq-quiet-peer",
);

/**
 * Starts `xmoq-quiet-peer` (`crates/quiet-peer`), a server that takes
 * sessions and leaves out its SETUP (`setup`), its answers to requests
 * (`requests`), or those but for a PUBLISH_NAMESPACE (`publish`). The
 * context has it where a run through a relay has the relay. Everything
 * started is stopped by `runCleanups`.
 */
export async function startQuietPeer(setup: {
	port: string;
	quiet: "setup" | "requests" | "publish";
	clients: Client[];
}): Promise<RunContext> {
	if (!existsSync(QUIET_PEER_BINARY)) {
		throw new Error(
			`${QUIET_PEER_BINARY} is missing: cargo build -p xmoq-quiet-peer`,
		);
	}
	const certificate = ensureCertificate();
	const peer = spawnLogged("quiet-peer", QUIET_PEER_BINARY, [
		"--port",
		setup.port,
		"--cert-file",
		certFile,
		"--key-file",
		keyFile,
		"--quiet",
		setup.quiet,
	]);
	defer(() => stopProcess(peer, "SIGTERM"));
	await peer.waitForLine(/^LISTENING port=/);
	const pages = await startBrowser([
		...new Set(setup.clients.filter(isBrowserClient)),
	]);
	return {
		relayUrl: `https://127.0.0.1:${setup.port}`,
		relay: peer,
		relayVersion: "xmoq-quiet-peer/0.0.0",
		certSha256Hex: certificate.sha256Hex,
		pages,
		verbose: false,
		// Not a relay: no run stops it and starts it anew.
		stopRelay: () => Promise.reject(new Error("the quiet peer is not stopped")),
		startRelayAnew: () =>
			Promise.reject(new Error("the quiet peer is not started anew")),
	};
}

const moqlivemockRoot = join(here, "peers", "moqlivemock", ".tools");

/** `mlmpub` serves the fingerprint of its certificate on its port plus this. */
export const MOQLIVEMOCK_SIDE_PORT_OFFSET = 1000;

/**
 * Starts `mlmpub`, the publisher of moqlivemock, which is a server: clients
 * connect to it, and no relay is in between. The context has it where a run
 * through a relay has the relay, so that a page is opened and connected in
 * the same way. Everything started is stopped by `runCleanups`.
 *
 * `mlmpub` makes its own certificate when it is given none, and serves the
 * SHA-256 of it at `/fingerprint` of a second port
 * (`MOQLIVEMOCK_SIDE_PORT_OFFSET`).
 */
export async function startMoqlivemockPublisher(setup: {
	port: string;
	verbose: boolean;
}): Promise<RunContext> {
	const binary = join(moqlivemockRoot, "bin", "mlmpub");
	if (!existsSync(binary)) {
		throw new Error(
			`${binary} is missing; see ${join(here, "peers", "moqlivemock", "README.md")} for the build steps`,
		);
	}
	const sidePort = String(Number(setup.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET);
	if (Number(sidePort) > 65535) {
		throw new Error(
			`mlmpub on port ${setup.port} needs the port ${sidePort} as well, which is none`,
		);
	}
	const publisher = spawnLogged("mlmpub", binary, [
		"-addr",
		`127.0.0.1:${setup.port}`,
		"-sideport",
		sidePort,
		// The media it serves; its default is a path relative to its source.
		"-asset",
		join(moqlivemockRoot, "assets", "test10s"),
		// It always writes a qlog, by default into the directory it runs in.
		"-qlog",
		join(logDir, `mlmpub-${setup.port}.qlog`),
		...(setup.verbose ? ["-loglevel", "debug"] : []),
	]);
	defer(() => stopProcess(publisher, "SIGINT"));
	await publisher.waitForLine(/Starting MoQ server/);
	const fingerprint = await withTimeout(
		fetch(`http://127.0.0.1:${sidePort}/fingerprint`),
		"mlmpub to serve the fingerprint of the certificate it made",
	);
	const certSha256Hex = (await fingerprint.text()).trim();
	if (!fingerprint.ok || !/^[0-9a-f]{64}$/.test(certSha256Hex)) {
		throw new Error(
			`mlmpub served no SHA-256 at /fingerprint: ${fingerprint.status} ${certSha256Hex.slice(0, 80)}`,
		);
	}
	log("driver", `certificate of mlmpub sha256=${certSha256Hex}`);
	const pages = await startBrowser(["own-browser"]);
	return {
		relayUrl: `https://127.0.0.1:${setup.port}/moq`,
		relay: publisher,
		relayVersion: "moqlivemock/v0.16.1",
		certSha256Hex,
		pages,
		verbose: setup.verbose,
		// Not a relay: no run stops it and starts it anew.
		stopRelay: () => Promise.reject(new Error("mlmpub is not stopped")),
		startRelayAnew: () =>
			Promise.reject(new Error("mlmpub is not started anew")),
	};
}

/**
 * Starts `mlmsub`, the subscriber of moqlivemock, against the relay of the
 * run, to read the catalog track of `namespace`. It writes the catalog it
 * read to `catalogOut`, and is stopped by `runCleanups`: it stays until its
 * `-duration` is over, also when it finds nothing to play.
 */
export function spawnMlmsub(
	context: RunContext,
	namespace: string,
	/** Further flags of `mlmsub`, after the ones every run has. */
	extraArgs: string[] = [],
): { process: LoggedProcess; catalogOut: string } {
	const binary = join(moqlivemockRoot, "bin", "mlmsub");
	if (!existsSync(binary)) {
		throw new Error(
			`${binary} is missing; see ${join(here, "peers", "moqlivemock", "README.md")} for the build steps`,
		);
	}
	// One file a run: the port is the run's own.
	const port = new URL(context.relayUrl).port;
	const catalogOut = join(logDir, `mlmsub-${port}.catalog.json`);
	rmSync(catalogOut, { force: true });
	const spawned = spawnLogged("mlmsub", binary, [
		"-addr",
		context.relayUrl,
		"-namespace",
		namespace,
		"-catalogout",
		catalogOut,
		// Longer than the run; the driver stops it.
		"-duration",
		"60",
		"-qlog",
		join(logDir, `mlmsub-${port}.qlog`),
		...extraArgs,
	]);
	defer(() => stopProcess(spawned, "SIGINT"));
	return { process: spawned, catalogOut };
}

/** Opens the page of a client, which has not connected to anything yet. */
export async function openPage(
	context: RunContext,
	label: string,
	client: BrowserClient,
): Promise<Page> {
	if (context.pages === undefined) {
		throw new Error(`${label} needs the browser, which was not started`);
	}
	const { origin, browser } = context.pages;
	const browserContext = await browser.newContext();
	const page = await browserContext.newPage();
	page.on("console", (message) => log(label, message.text()));
	page.on("pageerror", (error) => log(label, `pageerror: ${error.message}`));
	await page.goto(`${origin}${BROWSER_PAGES[client].path}`);
	await page.waitForFunction(() => typeof window.xmoq === "object");
	return page;
}

export async function openConnectedPage(
	context: RunContext,
	label: string,
	client: BrowserClient,
	/** Own page: the longest payload of an Object it takes; the library's when left out. */
	maxObjectPayloadLength?: number,
	/** Own page: how long the answer to a request is waited for; the library's when left out. */
	answerTimeoutMs?: number,
): Promise<Page> {
	const page = await openPage(context, label, client);
	await withTimeout(
		page.evaluate(
			([url, hash, limit, answer]) =>
				window.xmoq.connect(url, hash, limit, answer),
			[
				context.relayUrl,
				context.certSha256Hex,
				maxObjectPayloadLength,
				answerTimeoutMs,
			] satisfies [string, string, number | undefined, number | undefined],
		),
		`${label} to connect`,
	);
	return page;
}

/**
 * moqtail's own test client 0.15.0 as a publisher: the one publisher here
 * that answers a FETCH. It publishes the track as soon as it is connected
 * and sends its Groups by itself, three Objects of 8 bytes each, 250 ms
 * apart, every Object with the same payload; it takes no commands. It
 * prints `Track published`, and for each Group `Opening stream for group G`
 * and `Stream flushed for group G`; a `Sent object` line it prints for some
 * Objects only. It closes its session two seconds after the last Group.
 *
 * WHY `groups` is the caller's: the run has to be over before the publisher
 * is, and a FETCH is refused once the track is gone.
 */
export function spawnMoqtailTestPublisher(
	context: RunContext,
	label: string,
	groups: number,
): LoggedProcess {
	const spawned = spawnLogged(
		label,
		join(moqtailRoot, ".tools", "bin", "client"),
		[
			"--command",
			"publish",
			// The client makes an https:// URL of it for WebTransport.
			"--server",
			context.relayUrl.replace(/^https:/, "moqt:"),
			"--no-cert-validation",
			"--namespace",
			NAMESPACE,
			"--track-name",
			TRACK,
			"--group-count",
			String(groups),
			"--interval",
			"250",
			"--payload-size",
			"8",
			"--objects-per-group",
			"3",
		],
	);
	defer(() => stopProcess(spawned, "SIGTERM"));
	return spawned;
}

/**
 * The viewer for the terminal, `xmoq-watch` (`crates/watch`).
 *
 * WHY the release build, where the other binaries are debug builds: this
 * one decodes and writes a frame in the time a frame has. Unoptimized it
 * took 32 of the 33 ms on the machine this was written on, and the picture
 * fell a third of a second behind the sound within 8 seconds.
 */
const TERMINAL_VIEWER_BINARY = join(
	here,
	"..",
	"target",
	"release",
	"xmoq-watch",
);

/**
 * Starts `xmoq-watch` towards the relay of the run, or towards `url` and
 * `certSha256Hex` where they are given. It is stopped with the run.
 */
export function spawnTerminalViewer(
	context: RunContext,
	label: string,
	extraArgs: string[],
	server?: { url: string; certSha256Hex: string },
): LoggedProcess {
	if (!existsSync(TERMINAL_VIEWER_BINARY)) {
		throw new Error(
			`${TERMINAL_VIEWER_BINARY} is missing: cargo build --release -p xmoq-watch`,
		);
	}
	const spawned = spawnLogged(label, TERMINAL_VIEWER_BINARY, [
		"--url",
		server?.url ?? context.relayUrl,
		"--cert-sha256",
		server?.certSha256Hex ?? context.certSha256Hex,
		...extraArgs,
	]);
	defer(() => stopProcess(spawned, "SIGTERM"));
	return spawned;
}

export function spawnRustClient(
	context: RunContext,
	label: string,
	client: RustClient,
	// "publish-namespace" is a mode of the own Rust client alone: it announces
	// the namespace and serves the track when it is asked for it.
	// "fetch" is the own Rust client's as well: it sends a Standalone FETCH.
	mode: "publish" | "publish-namespace" | "subscribe" | "fetch",
	track: string,
	extraArgs: string[] = [],
): LoggedProcess {
	const spawned = spawnLogged(label, RUST_CLIENT_BINARIES[client], [
		"--mode",
		mode,
		"--url",
		context.relayUrl,
		"--cert-sha256",
		context.certSha256Hex,
		"--namespace",
		NAMESPACE,
		"--track",
		track,
		...extraArgs,
		// Only the own client can print the bytes of every stream.
		...(context.verbose && client === "own-rust" ? ["--trace"] : []),
	]);
	defer(async () => {
		spawned.child.stdin.end();
		await stopProcess(spawned, "SIGTERM");
	});
	return spawned;
}
