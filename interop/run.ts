// Interop driver: moves one MOQT Object through the moqtail relay between two
// clients, at least one of which is an own client, and compares the received
// payload with the sent one.
//
// Usage: bun run.ts --publisher <client> --subscriber <client> [--port N] [--verbose]
//                   [--subscribe-track NAME]
//   <client>: own-browser | moqtail-browser | moqtail-rust
//   --subscribe-track: track the subscriber asks for (default: the published one).
//     Another name makes the relay answer REQUEST_ERROR, and the run fails by design.
//
// The relay and the moqtail clients live under peers/moqtail; build them with
// the steps in README.md.

import {
	type ChildProcessWithoutNullStreams,
	execFileSync,
	spawn,
} from "node:child_process";
import { createHash, randomBytes, X509Certificate } from "node:crypto";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { type Browser, chromium, type Page } from "playwright-core";
import type { XmoqPage } from "./xmoq-page.ts";

// The functions passed to `page.evaluate` run in the page, where this global
// exists. This file is checked without the DOM lib, so it is declared here.
declare const window: { readonly xmoq: XmoqPage };

const here = import.meta.dir;
const moqtailRoot = join(here, "peers", "moqtail");
const relayBin = join(moqtailRoot, ".tools", "bin", "relay");
const certDir = join(here, ".cert");
const certFile = join(certDir, "cert.pem");
const keyFile = join(certDir, "key.pem");
const logDir = join(here, ".logs");
const distDir = join(here, ".dist");

// Provisional names, the same as the Phase 0 spike.
const NAMESPACE = "xmoq/phase0";
const TRACK = "object";
const STEP_TIMEOUT_MS = 20_000;
const KILL_GRACE_MS = 5_000;

type BrowserClient = "own-browser" | "moqtail-browser";
type RustClient = "moqtail-rust";
type Client = BrowserClient | RustClient;

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
 * `SUBSCRIBED` and `OBJECT` lines of peers/moqtail/rust-client.
 */
const RUST_CLIENT_BINARIES: Record<RustClient, string> = {
	"moqtail-rust": join(
		moqtailRoot,
		"rust-client",
		"target",
		"debug",
		"xmoq-interop-moqtail-peer",
	),
};

const OWN_CLIENTS: ReadonlySet<Client> = new Set<Client>(["own-browser"]);

function isBrowserClient(value: string): value is BrowserClient {
	return Object.hasOwn(BROWSER_PAGES, value);
}

function isRustClient(value: string): value is RustClient {
	return Object.hasOwn(RUST_CLIENT_BINARIES, value);
}

function isClient(value: string): value is Client {
	return isBrowserClient(value) || isRustClient(value);
}

function parseClient(option: string, value: string): Client {
	if (!isClient(value))
		throw new Error(`unknown client ${value} for --${option}`);
	return value;
}

const { values: options } = parseArgs({
	options: {
		publisher: { type: "string", default: "own-browser" },
		subscriber: { type: "string", default: "moqtail-rust" },
		port: { type: "string", default: "44330" },
		"subscribe-track": { type: "string", default: TRACK },
		verbose: { type: "boolean", default: false },
	},
});

function log(source: string, line: string): void {
	console.log(`[${source}] ${line}`);
}

function withTimeout<T>(promise: Promise<T>, what: string): Promise<T> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timer = setTimeout(
			() => reject(new Error(`timed out waiting for ${what}`)),
			STEP_TIMEOUT_MS,
		);
	});
	return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function captureGroup(match: RegExpMatchArray, index: number): string {
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
 * WHY one shared promise: a signal can arrive while `main` is already cleaning
 * up, and the two callers must not stop the same process twice.
 * WHY a loop instead of a snapshot: after a signal, `main` keeps running until
 * its next await fails and can still register a cleanup in the meantime.
 */
function runCleanups(): Promise<void> {
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

// WHY `once`: a second signal then gets the default action and ends the driver
// at once, which is the way out if a cleanup hangs.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
	process.once(signal, () => {
		log("driver", `received ${signal}, stopping`);
		void runCleanups().finally(() => process.exit(SIGNAL_EXIT_CODES[signal]));
	});
}

/**
 * Chromium accepts a certificate through `serverCertificateHashes` only if it
 * is ECDSA P-256 and valid for less than two weeks, so the certificate is
 * regenerated whenever it is missing or about to expire.
 */
function ensureCertificate(): { sha256Hex: string; validTo: string } {
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

interface LoggedProcess {
	child: ChildProcessWithoutNullStreams;
	exited: Promise<number | null>;
	waitForLine(pattern: RegExp): Promise<RegExpMatchArray>;
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
		waitForLine(pattern) {
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
			);
		},
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

function startRelay(port: string): LoggedProcess {
	const relay = spawnLogged(
		"relay",
		relayBin,
		[
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
		options.verbose ? { RUST_LOG: "debug" } : {},
	);
	defer(() => stopProcess(relay, "SIGINT"));
	return relay;
}

/** Bundles the pages the run needs and serves them from a throwaway HTTP server. */
async function servePages(clients: BrowserClient[]): Promise<string> {
	const html = readFileSync(join(here, "index.html"), "utf8");
	const files = new Map<string, { type: string; body: string }>();
	for (const client of clients) {
		const page = BROWSER_PAGES[client];
		const outdir = join(distDir, client);
		const build = await Bun.build({
			entrypoints: [page.entrypoint],
			target: "browser",
			outdir,
			// WHY: Bun 1.3.12 otherwise throws an error that says only "Bundle
			// failed"; the reasons are in `build.logs`.
			throw: false,
		});
		if (!build.success)
			throw new Error(
				`bundling ${page.entrypoint} failed: ${build.logs.join("\n")}`,
			);
		files.set(page.path, { type: "text/html", body: html });
		files.set(`${page.path}page.js`, {
			type: "text/javascript",
			body: readFileSync(join(outdir, "page.js"), "utf8"),
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

/** What every client needs to reach the relay. */
interface RunContext {
	relayUrl: string;
	certSha256Hex: string;
	pagesOrigin: string;
	browser: Browser;
}

async function openConnectedPage(
	context: RunContext,
	label: string,
	client: BrowserClient,
): Promise<Page> {
	const browserContext = await context.browser.newContext();
	const page = await browserContext.newPage();
	page.on("console", (message) => log(label, message.text()));
	page.on("pageerror", (error) => log(label, `pageerror: ${error.message}`));
	await page.goto(`${context.pagesOrigin}${BROWSER_PAGES[client].path}`);
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

function spawnRustClient(
	context: RunContext,
	label: string,
	client: RustClient,
	mode: "publish" | "subscribe",
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
	]);
	defer(async () => {
		spawned.child.stdin.end();
		await stopProcess(spawned, "SIGTERM");
	});
	return spawned;
}

interface Publisher {
	sendObject(): Promise<void>;
}

interface Subscriber {
	receiveHex(): Promise<string>;
}

/** Connects a publisher and offers the track. `payloadHex` is the Object it sends later. */
async function startPublisher(
	kind: Client,
	context: RunContext,
	payloadHex: string,
): Promise<Publisher> {
	const label = `pub:${kind}`;
	if (isBrowserClient(kind)) {
		const page = await openConnectedPage(context, label, kind);
		await withTimeout(
			page.evaluate(([ns, track]) => window.xmoq.publishTrack(ns, track), [
				NAMESPACE,
				TRACK,
			] satisfies [string, string]),
			`${label} to publish the track`,
		);
		return {
			async sendObject() {
				await page.evaluate((hex) => window.xmoq.sendObject(hex), payloadHex);
			},
		};
	}
	// The Rust client takes the payload when it starts and sends it on a line on stdin.
	const client = spawnRustClient(context, label, kind, "publish", TRACK, [
		"--payload-hex",
		payloadHex,
	]);
	await client.waitForLine(/^PUBLISHED /);
	return {
		async sendObject() {
			client.child.stdin.write("send\n");
			await client.waitForLine(/^SENT /);
		},
	};
}

/** Connects a subscriber and subscribes to `track`. */
async function startSubscriber(
	kind: Client,
	context: RunContext,
	track: string,
): Promise<Subscriber> {
	const label = `sub:${kind}`;
	if (isBrowserClient(kind)) {
		const page = await openConnectedPage(context, label, kind);
		await withTimeout(
			page.evaluate(([ns, name]) => window.xmoq.subscribe(ns, name), [
				NAMESPACE,
				track,
			] satisfies [string, string]),
			`${label} to subscribe`,
		);
		return {
			async receiveHex() {
				const object = await withTimeout(
					page.evaluate(() => window.xmoq.firstObject()),
					`${label} to receive`,
				);
				return object.payloadHex;
			},
		};
	}
	const client = spawnRustClient(context, label, kind, "subscribe", track);
	await client.waitForLine(/^SUBSCRIBED /);
	return {
		async receiveHex() {
			return captureGroup(
				await client.waitForLine(/^OBJECT .*payload_hex=([0-9a-f]*)$/),
				1,
			);
		},
	};
}

interface RunResult {
	publisher: Client;
	subscriber: Client;
	match: boolean;
	sentText: string;
	receivedText: string;
	sentHex: string;
	receivedHex: string;
	relayVersion: string;
	browserVersion: string;
}

async function transferOneObject(
	publisher: Client,
	subscriber: Client,
	subscribeTrack: string,
): Promise<RunResult> {
	const certificate = ensureCertificate();
	log(
		"driver",
		`certificate sha256=${certificate.sha256Hex} validTo=${certificate.validTo}`,
	);
	const payload = Buffer.from(
		`xmoq-phase0b ${new Date().toISOString()} ${randomBytes(8).toString("hex")}`,
	);
	const payloadHex = payload.toString("hex");

	const relay = startRelay(options.port);
	const relayVersion = captureGroup(
		await relay.waitForLine(/(moqtail\/\S+) is running on/),
		1,
	);
	const pagesOrigin = await servePages(
		[publisher, subscriber].filter(isBrowserClient),
	);
	const browser = await chromium.launch({
		headless: true,
		channel: "chromium",
		// WHY off: the handlers above own the teardown. The SIGINT handler of
		// playwright-core 1.64.0 closes the browser and then calls process.exit(130),
		// which would skip stopping the relay and the Rust clients.
		handleSIGINT: false,
		handleSIGTERM: false,
	});
	defer(() => browser.close());
	log("driver", `browser=chromium version=${browser.version()}`);
	const context: RunContext = {
		relayUrl: `https://127.0.0.1:${options.port}`,
		certSha256Hex: certificate.sha256Hex,
		pagesOrigin,
		browser,
	};

	// The publisher offers the track first: the relay rejects a SUBSCRIBE for
	// a track nobody has published.
	const publishing = await startPublisher(publisher, context, payloadHex);
	// The subscriber is live before the Object is sent: the Largest Object
	// filter does not replay what was published earlier.
	const subscribing = await startSubscriber(
		subscriber,
		context,
		subscribeTrack,
	);
	await withTimeout(publishing.sendObject(), "the publisher to send");
	const receivedHex = await subscribing.receiveHex();

	return {
		publisher,
		subscriber,
		match: receivedHex === payloadHex,
		sentText: payload.toString(),
		receivedText: Buffer.from(receivedHex, "hex").toString(),
		sentHex: payloadHex,
		receivedHex,
		relayVersion,
		browserVersion: browser.version(),
	};
}

async function main(): Promise<void> {
	const publisher = parseClient("publisher", options.publisher);
	const subscriber = parseClient("subscriber", options.subscriber);
	if (!OWN_CLIENTS.has(publisher) && !OWN_CLIENTS.has(subscriber)) {
		throw new Error(
			"one side has to be an own client; a moqtail pair checks nothing of this repository",
		);
	}
	const rustBinaries = [publisher, subscriber]
		.filter(isRustClient)
		.map((client) => RUST_CLIENT_BINARIES[client]);
	for (const binary of [relayBin, ...rustBinaries]) {
		if (!existsSync(binary)) {
			throw new Error(
				`${binary} is missing; see ${join(here, "README.md")} for the build steps`,
			);
		}
	}

	let result: RunResult;
	try {
		result = await transferOneObject(
			publisher,
			subscriber,
			options["subscribe-track"],
		);
	} finally {
		await runCleanups();
	}

	log("driver", `RESULT ${JSON.stringify(result)}`);
	if (!result.match)
		throw new Error("received payload differs from the sent payload");
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
