// Spike, outside lint and review: what a writer of a WebTransport stream fails
// with in Chromium when the peer stops the stream, when the session ends, and
// when the connection is lost, and whether that comes before `closed` of the
// session settles (docs/experiments/2026-10-11-write-abort-in-chromium.md).
//
// Needs `cargo build --locked` in this directory and the certificate that an
// interop run creates under interop/.cert.
// Run: bun spikes/wt-write-abort/run.ts [--rounds 40] [--port 47330]
import { type ChildProcess, spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: options } = parseArgs({
	options: {
		rounds: { type: "string", default: "40" },
		port: { type: "string", default: "47330" },
		out: { type: "string", default: "" },
	},
});
const rounds = Number(options.rounds);
const here = import.meta.dir;
const root = join(here, "..", "..");
const cert = join(root, "interop/.cert/cert.pem");
const key = join(root, "interop/.cert/key.pem");
const certSha256 = createHash("sha256")
	.update(new X509Certificate(readFileSync(cert)).raw)
	.digest("hex");

/** Starts the server and resolves once it listens. */
async function startServer(): Promise<ChildProcess> {
	const server = spawn(
		join(here, "target/debug/xmoq-wt-write-abort-spike"),
		[options.port, cert, key],
		{ stdio: ["ignore", "pipe", "inherit"] },
	);
	await new Promise<void>((resolve, reject) => {
		server.stdout?.setEncoding("utf8");
		server.stdout?.on("data", (text: string) => {
			if (text.includes("LISTENING")) resolve();
		});
		server.once("exit", (code) => reject(new Error(`the server exited: ${code}`)));
	});
	return server;
}

type Scenario = "stop" | "stop-now" | "peer-close" | "local-close" | "killed";
type Kind = "uni" | "bidi";
/**
 * What the writer does when it happens: nothing; one large write that waits
 * for the peer's window; or small writes one after the other, each awaited,
 * as an application that sends Objects does.
 */
type Mode = "idle" | "waiting" | "writing";

/** How a promise of the page was rejected, or "resolved", or "pending". */
type Outcome = string;

interface Sample {
	scenario: Scenario;
	kind: Kind;
	mode: Mode;
	/** The write of the first byte, which opens the stream for the peer. */
	firstWrite: Outcome;
	/** `writer.closed`. */
	writerClosed: Outcome;
	/** The write that was waiting, or the first of the small writes that failed. */
	pendingWrite: Outcome;
	/** A write made 300 ms after `writer.closed` settled. */
	lateWrite: Outcome;
	/** `closed` of the session, at the end of the sample. */
	sessionClosed: Outcome;
	/** Milliseconds from `writer.closed` settling to `closed` of the session settling; negative when the session was first, null when it had not settled 300 ms later. */
	sessionAfterMs: number | null;
}

// Runs in the page, on raw `WebTransport`. `go` resolves when the driver has
// done its part of the scenario (killed the server); the others need none.
async function sample([url, hashHex, scenario, kind, mode]: [
	string,
	string,
	Scenario,
	Kind,
	Mode,
]): Promise<Sample> {
	const describe = (error: unknown): string => {
		// biome-ignore lint/suspicious/noExplicitAny: a value of the browser, looked at field by field
		const e = error as any;
		const ctor = Object.getPrototypeOf(e)?.constructor?.name;
		return `${ctor} name=${e?.name} source=${e?.source} code=${e?.streamErrorCode} "${e?.message}"`;
	};
	const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
	const hash = new Uint8Array(
		(hashHex.match(/../g) ?? []).map((pair) => Number.parseInt(pair, 16)),
	);
	const path = {
		stop: "/stop",
		"stop-now": "/stop-now",
		"peer-close": "/close",
		"local-close": "/hold",
		killed: "/hold",
	}[scenario];
	// biome-ignore lint/suspicious/noExplicitAny: `WebTransport` is not in the types of this script
	const transport = new (globalThis as any).WebTransport(url + path, {
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
	});
	await transport.ready;
	let sessionClosed = "pending";
	let sessionClosedAt: number | undefined;
	transport.closed.then(
		(info: { closeCode: number; reason: string }) => {
			sessionClosed = `resolved code=${info.closeCode} "${info.reason}"`;
			sessionClosedAt = performance.now();
		},
		(error: unknown) => {
			sessionClosed = describe(error);
			sessionClosedAt = performance.now();
		},
	);
	const writable: WritableStream<Uint8Array> =
		kind === "uni"
			? await transport.createUnidirectionalStream()
			: (await transport.createBidirectionalStream()).writable;
	const writer = writable.getWriter();
	// The stream exists for the peer from its first byte on. A peer that
	// stops the stream at once can make this write fail already.
	const firstWrite: string = await writer.write(new Uint8Array([1])).then(
		() => "resolved",
		describe,
	);
	let writerClosed = "pending";
	let writerClosedAt: number | undefined;
	writer.closed.then(
		() => {
			writerClosed = "resolved";
			writerClosedAt = performance.now();
		},
		(error: unknown) => {
			writerClosed = describe(error);
			writerClosedAt = performance.now();
		},
	);
	let pendingWrite = mode === "idle" ? "none" : "pending";
	if (mode === "waiting") {
		// More than the peer's window takes, and the peer reads nothing.
		writer.write(new Uint8Array(8 * 1024 * 1024)).then(
			() => {
				pendingWrite = "resolved";
			},
			(error: unknown) => {
				pendingWrite = describe(error);
			},
		);
	}
	if (mode === "writing") {
		// Not awaited here: the loop goes on beside the scenario. 100 bytes a
		// write and a pause, so that the peer's window is not what stops it.
		void (async () => {
			try {
				for (let sent = 0; sent < 2000; sent++) {
					await writer.write(new Uint8Array(100));
					await sleep(0);
				}
				pendingWrite = "resolved, 2000 times";
			} catch (error) {
				pendingWrite = describe(error);
			}
		})();
	}
	if (scenario === "local-close") {
		await sleep(200);
		transport.close({ closeCode: 4, reason: "bye from the page" });
	}
	if (scenario === "killed") {
		// biome-ignore lint/suspicious/noExplicitAny: a function the driver exposes
		await (globalThis as any).serverKilled();
	}
	for (let waited = 0; writerClosed === "pending" && waited < 15_000; waited += 5) {
		await sleep(5);
	}
	await sleep(300);
	const lateWrite: string = await writer.write(new Uint8Array([2])).then(
		() => "resolved",
		describe,
	);
	const sessionAfterMs =
		sessionClosedAt === undefined || writerClosedAt === undefined
			? null
			: Math.round((sessionClosedAt - writerClosedAt) * 10) / 10;
	if (sessionClosed === "pending") transport.close();
	return {
		scenario,
		kind,
		mode,
		firstWrite,
		writerClosed,
		pendingWrite,
		lateWrite,
		sessionClosed,
		sessionAfterMs,
	};
}

let server = await startServer();
// A run that fails must not leave the server on its port.
process.on("exit", () => server.kill("SIGKILL"));
process.on("unhandledRejection", (error) => {
	console.error(error);
	process.exit(1);
});
const pageServer = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }),
});
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
await page.goto(`http://localhost:${pageServer.port}/`);
let killed: () => void = () => {};
let whenKilled = new Promise<void>((resolve) => {
	killed = resolve;
});
await page.exposeFunction("serverKilled", () => whenKilled);

const url = `https://127.0.0.1:${options.port}`;
const samples: Sample[] = [];
const combos: [Kind, Mode][] = [
	["uni", "idle"],
	["uni", "waiting"],
	["uni", "writing"],
	["bidi", "idle"],
	["bidi", "waiting"],
	["bidi", "writing"],
];
const run = (scenario: Scenario, kind: Kind, mode: Mode): Promise<Sample> =>
	page.evaluate(sample, [url, certSha256, scenario, kind, mode] satisfies [
		string,
		string,
		Scenario,
		Kind,
		Mode,
	]);

for (const scenario of [
	"stop",
	"stop-now",
	"peer-close",
	"local-close",
] satisfies Scenario[]) {
	for (const [kind, pending] of combos) {
		// Eight at a time: one after the other would take minutes.
		for (let done = 0; done < rounds; done += 8) {
			const batch = Math.min(8, rounds - done);
			samples.push(
				...(await Promise.all(
					Array.from({ length: batch }, () => run(scenario, kind, pending)),
				)),
			);
		}
		console.log(`done ${scenario} ${kind} ${pending}`);
	}
}

// The server is killed while every session of a batch is open, so one kill
// gives a sample for each: ten of each kind and mode a kill, with a server
// started anew in between. WHY not more at once: the server gives a session
// 3 seconds without traffic, and a batch has to be open before that.
for (let done = 0; done < rounds; done += 10) {
	const open = combos.flatMap(([kind, mode]) =>
		Array.from({ length: Math.min(10, rounds - done) }, () =>
			run("killed", kind, mode),
		),
	);
	await new Promise((resolve) => setTimeout(resolve, 1000));
	server.kill("SIGKILL");
	killed();
	samples.push(...(await Promise.all(open)));
	console.log(`done killed, ${done + 10} of ${rounds}`);
	whenKilled = new Promise<void>((resolve) => {
		killed = resolve;
	});
	server = await startServer();
}

/** Counts the samples of each scenario by what was seen. */
const table = new Map<string, number>();
const gaps = new Map<string, number[]>();
for (const each of samples) {
	const order =
		each.sessionAfterMs === null
			? "session open 300 ms later"
			: each.sessionAfterMs >= 0
				? "writer first"
				: "session first";
	const row = `${each.scenario} | ${each.kind} | ${each.mode} | first write: ${each.firstWrite} | writer.closed: ${each.writerClosed} | waiting write: ${each.pendingWrite} | later write: ${each.lateWrite} | session: ${each.sessionClosed} | ${order}`;
	table.set(row, (table.get(row) ?? 0) + 1);
	if (each.sessionAfterMs !== null) {
		const group = `${each.scenario} | ${each.kind} | ${each.mode}`;
		gaps.set(group, [...(gaps.get(group) ?? []), each.sessionAfterMs]);
	}
}
const lines = [
	`browser=${browser.version()} samples=${samples.length} rounds=${rounds}`,
	...[...table.entries()].sort().map(([row, count]) => `${count} x ${row}`),
	"ms from writer.closed to the session's closed (min / median / max):",
	...[...gaps.entries()].sort().map(([group, values]) => {
		const sorted = [...values].sort((a, b) => a - b);
		return `${group}: ${sorted[0]} / ${sorted[Math.floor(sorted.length / 2)]} / ${sorted.at(-1)} (n=${sorted.length})`;
	}),
];
console.log(lines.join("\n"));
if (options.out !== "") writeFileSync(options.out, `${lines.join("\n")}\n`);

await browser.close();
pageServer.stop(true);
server.kill("SIGKILL");
process.exit(0);
