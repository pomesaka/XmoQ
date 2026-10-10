// What every interop driver needs: the certificate, the relay, the browser
// with its pages, the Rust clients, and the teardown of all of them. `run.ts`
// and `disconnect.ts` are the drivers.

import {
	type ChildProcessWithoutNullStreams,
	execFileSync,
	spawn,
} from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
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
 * WHY the uncaught failures: Bun 1.3.12 ends the process on them without
 * running any `finally`, and the relay stayed behind with its port.
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

/** Spawns a child, echoes its output, and lets callers wait for a matching line. */
function spawnLogged(
	source: string,
	command: string,
	args: string[],
	env: Record<string, string> = {},
): LoggedProcess {
	const child = spawn(command, args, { env: { ...process.env, ...env } });
	const lines: string[] = [];
	const waiters: {
		pattern: RegExp;
		resolve: (match: RegExpMatchArray) => void;
	}[] = [];
	const exited = new Promise<number | null>((resolve) =>
		child.once("exit", resolve),
	);

	const onLine = (line: string): void => {
		// Strip the ANSI colors `tracing` adds, so the patterns can match.
		// biome-ignore lint/suspicious/noControlCharactersInRegex: ESC starts every color sequence
		const clean = line.replace(/\u001b\[[0-9;]*m/g, "");
		lines.push(clean);
		log(source, clean);
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
			const died = exited.then((code): never => {
				throw new Error(
					`${source} exited with code ${code} before printing ${pattern}`,
				);
			});
			return withTimeout(
				Promise.race([matched, died]),
				`${source} to print ${pattern}`,
				timeoutMs,
			);
		},
		matches: (pattern) =>
			lines.flatMap((line) => {
				const match = line.match(pattern);
				return match === null ? [] : [match];
			}),
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
			// WHY: Bun 1.3.12 otherwise throws an error that says only "Bundle
			// failed"; the reasons are in `build.logs`.
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
	relay: LoggedProcess;
	relayVersion: string;
	certSha256Hex: string;
	/** Absent in a run between Rust clients, which starts no browser. */
	pages: { origin: string; browser: Browser } | undefined;
	/** Makes `own-rust` print the bytes of every stream. */
	verbose: boolean;
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
	const relay = spawnLogged(
		"relay",
		config.binary,
		config.args(setup.port),
		setup.verbose ? config.verboseEnv : {},
	);
	defer(() => stopProcess(relay, "SIGINT"));
	const relayVersion = captureGroup(await relay.waitForLine(config.ready), 1);
	const pages = await startBrowser([
		...new Set(setup.clients.filter(isBrowserClient)),
	]);
	return {
		relayUrl: `https://127.0.0.1:${setup.port}`,
		relay,
		relayVersion,
		certSha256Hex: certificate.sha256Hex,
		pages,
		verbose: setup.verbose,
	};
}

export async function openConnectedPage(
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
	await withTimeout(
		page.evaluate(([url, hash]) => window.xmoq.connect(url, hash), [
			context.relayUrl,
			context.certSha256Hex,
		] satisfies [string, string]),
		`${label} to connect`,
	);
	return page;
}

export function spawnRustClient(
	context: RunContext,
	label: string,
	client: RustClient,
	// "publish-namespace" is a mode of the own Rust client alone: it announces
	// the namespace and serves the track when it is asked for it.
	mode: "publish" | "publish-namespace" | "subscribe",
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
