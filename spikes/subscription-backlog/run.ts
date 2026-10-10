// Spike, outside lint and review: what a page that subscribes with @xmoq/moqt and never
// reads does to the own relay and to the browser
// (docs/experiments/2026-10-10-subscription-backlog-in-chromium.md).
// Needs `cargo build --locked -p xmoq-relay` and the certificate that an interop run
// creates under interop/.cert.
// Run: bun spikes/subscription-backlog/run.ts --objects-per-group 1 --object-bytes 57 --seconds 20
//   --max-streams N: `maxPeerUniStreams` of the page that does not read (default: the
//     library's). A number no run reaches shows the library as it was without the limit.
import { execFileSync, spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: o } = parseArgs({ options: {
	port: { type: "string", default: "47300" },
	"objects-per-group": { type: "string", default: "1" },
	"object-bytes": { type: "string", default: "57" },
	seconds: { type: "string", default: "20" },
	"max-streams": { type: "string", default: "0" },
	// When the relay has ended the first subscription, or the seconds are over: subscribe
	// to a second track from the page that does not read, and report whether the answer
	// arrives.
	"then-subscribe": { type: "boolean", default: false },
} });
const W = join(import.meta.dir, "..", "..");
const cert = join(W, "interop/.cert/cert.pem"), key = join(W, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const lines: string[] = [];
const relay = spawn(join(W, "target/debug/xmoq-relay"), ["--port", o.port, "--cert-file", cert, "--key-file", key]);
for (const s of [relay.stdout, relay.stderr]) { s.setEncoding("utf8"); let b = ""; s.on("data", (d: string) => { b += d; const p = b.split("\n"); b = p.pop() ?? ""; lines.push(...p); }); }
const until = async (found: () => boolean, ms: number) => { const end = Date.now() + ms; while (Date.now() < end) { if (found()) return true; await new Promise((r) => setTimeout(r, 50)); } return false; };
await until(() => lines.some((l) => l.startsWith("LISTENING")), 5000);
const build = await Bun.build({ entrypoints: [join(import.meta.dir, "page.ts")], target: "browser" });
const js = await build.outputs[0].text();
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: (q) => new URL(q.url).pathname === "/page.js" ? new Response(js, { headers: { "content-type": "text/javascript" } }) : new Response("<script type=module src=/page.js></script>", { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
/** Resident memory of every process of the browser, in MiB. */
const browserMiB = (): number => {
	const out = execFileSync("ps", ["-axo", "rss=,command="]).toString();
	const kib = out.split("\n").filter((l) => /ms-playwright.*[Cc]hrom/.test(l)).reduce((sum, l) => sum + Number(l.trim().split(/\s+/)[0]), 0);
	return Math.round(kib / 1024);
};
const open = async (maxStreams: number) => { const page = await (await browser.newContext()).newPage(); await page.goto(`http://localhost:${server.port}/`); await page.waitForFunction(() => "backlog" in globalThis); await page.evaluate(([u, h, m]) => (globalThis as any).backlog.connect(u, h, m), [`https://127.0.0.1:${o.port}`, sha, maxStreams] as const); return page; };
const publisher = await open(0);
await publisher.evaluate(([n, b]) => (globalThis as any).backlog.publish(n, b), [Number(o["objects-per-group"]), Number(o["object-bytes"])]);
// The publisher runs before anyone subscribes; the relay drops what nobody takes.
const slow = await open(Number(o["max-streams"]));
const before = browserMiB();
await slow.evaluate(() => (globalThis as any).backlog.subscribeAndNeverRead());
const subscribedAt = Date.now();
const sentAtSubscribe = await publisher.evaluate(() => (globalThis as any).backlog.sent());
const cutOff = /^SEND session=1 stream=\d+ PUBLISH_DONE status_code=0x5 /;
const wasCut = await until(() => lines.some((l) => cutOff.test(l)), Number(o.seconds) * 1000);
const seconds = ((Date.now() - subscribedAt) / 1000).toFixed(1);
const sent = await publisher.evaluate(() => (globalThis as any).backlog.sent());
// The page learns of PUBLISH_DONE at once and ends the subscription when its wait for
// late streams is over (5 seconds in the library).
let stats = await slow.evaluate(() => (globalThis as any).backlog.stats());
const after = browserMiB();
if (wasCut) {
	await until(() => false, Number(process.env.WAIT_MS ?? "6500"));
	stats = { ...stats, ended: (await slow.evaluate(() => (globalThis as any).backlog.stats())).ended };
}
const secondSubscribe = o["then-subscribe"]
	? await slow.evaluate(() => (globalThis as any).backlog.subscribeToSecond(10000))
	: null;
// Where the page was not told, it starts reading: does it learn of the end then?
let afterReading: unknown = null;
if (wasCut && stats.ended === null) {
	const began = Date.now();
	const read = await Promise.race([
		slow.evaluate(() => (globalThis as any).backlog.readAll()),
		new Promise((r) => setTimeout(() => r("still reading after 15s"), 15000)),
	]);
	afterReading = { read, seconds: ((Date.now() - began) / 1000).toFixed(1), ended: (await slow.evaluate(() => (globalThis as any).backlog.stats())).ended };
}
const bytes = Number(o["object-bytes"]);
console.log(JSON.stringify({
	objectsPerGroup: Number(o["objects-per-group"]), objectBytes: bytes,
	cutOffByRelay: wasCut, secondsWatched: seconds,
	sentWhileSubscribed: { groups: sent.groups - sentAtSubscribe.groups, objects: sent.objects - sentAtSubscribe.objects, MiB: Math.round(((sent.objects - sentAtSubscribe.objects) * bytes) / 1048576) },
	handedOverToTheQueue: stats.handedOver, ended: stats.ended,
	maxPeerUniStreams: Number(o["max-streams"]) || "default (64)",
	browserMiB: { before, after },
	streamsOpenedForSubscriber: lines.filter((l) => /^OPEN_DATA session=1 /.test(l)).length,
	secondSubscribe,
	relaySentSubscribeOkToSubscriber: lines.filter((l) => /^SEND session=1 stream=\d+ SUBSCRIBE_OK/.test(l)).length,
	afterReading,
	relayClosedASession: lines.filter((l) => /^CLOSE_SESSION/.test(l)),
	publisher: sent.stopped || "still sending",
}));
await publisher.evaluate(() => (globalThis as any).backlog.stop());
await browser.close(); server.stop(true); relay.kill("SIGINT");
