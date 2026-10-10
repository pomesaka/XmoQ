// Spike, outside lint and review: Objects of the size of a video frame, 30 a second, each
// on a stream of its own (as draft-ietf-moq-msf-01 §6 asks) or a Group on one stream,
// through the own relay to three pages on @xmoq/moqt: one that never reads, one that reads
// half as fast as the Objects come, and one that reads everything
// (docs/experiments/2026-10-10-subscription-backlog-in-chromium.md).
// Needs `cargo build --locked -p xmoq-relay` and the certificate of an interop run.
// Run from the repository root:
//   bun spikes/frame-streams/run.ts --one-per-stream [--object-bytes 10000] [--per-second 30]
//     [--objects-per-group 30] [--slow-reads 15] [--seconds 40] [--port 47500]
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: o } = parseArgs({ options: {
	port: { type: "string", default: "47500" },
	"one-per-stream": { type: "boolean", default: false },
	"object-bytes": { type: "string", default: "10000" },
	"per-second": { type: "string", default: "30" },
	"objects-per-group": { type: "string", default: "30" },
	"slow-reads": { type: "string", default: "15" },
	seconds: { type: "string", default: "40" },
} });
const root = join(import.meta.dir, "..", "..");
const cert = join(root, "interop/.cert/cert.pem"), key = join(root, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const started = Date.now();
const lines: { at: number; line: string }[] = [];
const relay = spawn(join(root, "target/debug/xmoq-relay"), ["--port", o.port, "--cert-file", cert, "--key-file", key]);
for (const s of [relay.stdout, relay.stderr]) { s.setEncoding("utf8"); let b = ""; s.on("data", (d: string) => { b += d; const p = b.split("\n"); b = p.pop() ?? ""; for (const line of p) lines.push({ at: Date.now(), line }); }); }
const until = async (found: () => boolean, ms: number) => { const end = Date.now() + ms; while (Date.now() < end) { if (found()) return true; await new Promise((r) => setTimeout(r, 20)); } return found(); };
await until(() => lines.some((l) => l.line.startsWith("LISTENING")), 5000);
const build = await Bun.build({ entrypoints: [join(import.meta.dir, "page.ts")], target: "browser" });
const js = await build.outputs[0]!.text();
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: (q) => new URL(q.url).pathname === "/page.js" ? new Response(js, { headers: { "content-type": "text/javascript" } }) : new Response('<!DOCTYPE html><script type="module" src="/page.js"></script>', { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const open = async () => { const page = await (await browser.newContext()).newPage(); await page.goto(`http://localhost:${server.port}/`); await page.waitForFunction(() => typeof (globalThis as any).frames === "object" && typeof (globalThis as any).frames.connect === "function"); await page.evaluate(([url, hash]) => (globalThis as any).frames.connect(url, hash), [`https://127.0.0.1:${o.port}`, sha]); return page; };
const count = (pattern: RegExp) => lines.filter((l) => pattern.test(l.line)).length;
const firstAt = (pattern: RegExp) => lines.find((l) => pattern.test(l.line))?.at;

try {
	// Sessions in the order they connect: 0 publisher, 1 never reads, 2 reads slowly, 3 reads everything.
	const publisher = await open();
	const never = await open();
	const slow = await open();
	const reader = await open();
	await publisher.evaluate(([b, r, n, one]) => (globalThis as any).frames.publish(b, r, n, one), [Number(o["object-bytes"]), Number(o["per-second"]), Number(o["objects-per-group"]), o["one-per-stream"]] as const);
	await reader.evaluate(() => (globalThis as any).frames.subscribe(-1));
	await slow.evaluate((rate) => (globalThis as any).frames.subscribe(rate), Number(o["slow-reads"]));
	await never.evaluate(() => (globalThis as any).frames.subscribe(0));
	const subscribedAt = Date.now();
	const sentAtSubscribe = await publisher.evaluate(() => (globalThis as any).frames.sent());
	const cut = (session: number) => new RegExp(`^SEND session=${session} stream=\\d+ PUBLISH_DONE status_code=0x5 `);
	await until(() => false, Number(o.seconds) * 1000);
	const sent = await publisher.evaluate(() => (globalThis as any).frames.sent());
	// Does the page that never reads get an answer to another request of the same session?
	const second = await never.evaluate(() => (globalThis as any).frames.subscribeToSecond(10_000));
	const stats = { never: await never.evaluate(() => (globalThis as any).frames.stats()), slow: await slow.evaluate(() => (globalThis as any).frames.stats()), reader: await reader.evaluate(() => (globalThis as any).frames.stats()) };
	const seconds = (at: number | undefined) => (at === undefined ? null : Math.round((at - subscribedAt) / 100) / 10);
	const perSession = (session: number) => ({
		cutOffAfterSeconds: seconds(firstAt(cut(session))),
		publishDoneLine: lines.find((l) => cut(session).test(l.line))?.line.replace(/^SEND session=\d+ stream=\d+ /, "") ?? null,
		streamsBooked: count(new RegExp(`^OPEN_DATA session=${session} `)),
	});
	console.log(JSON.stringify({
		shape: o["one-per-stream"] ? "one Object a stream" : `${o["objects-per-group"]} Objects a stream`,
		objectBytes: Number(o["object-bytes"]), perSecondAsked: Number(o["per-second"]), secondsWatched: Number(o.seconds),
		publisher: { objects: sent.objects, streams: sent.streams, late: sent.late, perSecondMeasured: Math.round((sent.objects / (sent.elapsedMs / 1000)) * 10) / 10, stopped: sent.stopped || "still sending", sentBeforeSubscribers: sentAtSubscribe.objects },
		neverReads: { ...perSession(1), handedOverToTheQueue: stats.never.handedOver, ended: stats.never.ended, publishDoneSeenByPage: stats.never.publishDoneSeenMs !== null, secondSubscribe: second },
		readsSlowly: { ...perSession(2), readPerSecondAsked: Number(o["slow-reads"]), read: stats.slow.read, handedOverToTheQueue: stats.slow.handedOver, ended: stats.slow.ended, readEnd: stats.slow.readEnd },
		readsEverything: { ...perSession(3), read: stats.reader.read, ended: stats.reader.ended, readEnd: stats.reader.readEnd },
		relay: { resets: count(/^RESET_DATA /), resetCodes: [...new Set(lines.map((l) => l.line.match(/^RESET_DATA out=\d+ code=(\S+)/)?.[1]).filter((c) => c !== undefined))], closedASession: lines.filter((l) => /^CLOSE_SESSION/.test(l.line)).map((l) => l.line), subscribeOkToNever: count(/^SEND session=1 stream=\d+ SUBSCRIBE_OK/) },
		// How and when the relay saw a session end, in seconds after the subscribers joined.
		sessionEnds: lines.filter((l) => /^(CONNECTION_ENDED|SESSION_CLOSED|CLOSE_SESSION) /.test(l.line)).map((l) => `${seconds(l.at)}s ${l.line.slice(0, 120)}`),
		wallSeconds: Math.round((Date.now() - started) / 100) / 10,
	}));
	await publisher.evaluate(() => (globalThis as any).frames.stop());
} finally {
	await browser.close(); server.stop(true); relay.kill("SIGINT");
}
