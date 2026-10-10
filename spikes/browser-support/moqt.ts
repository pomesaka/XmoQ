// Spike, outside lint and review: the interop page of `@xmoq/moqt` in a browser other
// than Chromium, through the own relay. One page publishes, one subscribes.
// Needs `cargo build --locked -p xmoq-relay` and the certificate of an interop run.
// Run from the repository root:
//   bun spikes/browser-support/moqt.ts <chromium|firefox> [--port 47410] [--objects 1] [--bytes 6] [--close-publisher]
// With --close-publisher the publisher's page is closed after the Objects, without
// PUBLISH_DONE and without close(): what do the relay and the subscriber see?
// With --objects N the publisher sends N Objects, one a stream, and the subscriber is
// asked for the last Group of every thousand: how many streams does a session take?
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium, firefox } from "../../interop/node_modules/playwright-core/index.mjs";

const { values, positionals } = parseArgs({
	allowPositionals: true,
	options: { port: { type: "string", default: "47410" }, "close-publisher": { type: "boolean", default: false }, objects: { type: "string", default: "1" }, bytes: { type: "string", default: "6" } },
});
const name = positionals[0] ?? "";
if (name !== "chromium" && name !== "firefox") throw new Error("usage: moqt.ts <chromium|firefox>");
const objects = Number(values.objects);
const payloadHex = "ab".repeat(Number(values.bytes));
const root = join(import.meta.dir, "..", "..");
const cert = join(root, "interop/.cert/cert.pem");
const key = join(root, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const started = performance.now();
const say = (line: string) => console.log(`${((performance.now() - started) / 1000).toFixed(1)}s ${line}`);

const relay = spawn(join(root, "target/debug/xmoq-relay"), ["--port", values.port, "--cert-file", cert, "--key-file", key]);
const relayLines: string[] = [];
for (const stream of [relay.stdout, relay.stderr]) {
	stream.setEncoding("utf8");
	let rest = "";
	stream.on("data", (data: string) => {
		rest += data;
		const parts = rest.split("\n");
		rest = parts.pop() ?? "";
		relayLines.push(...parts);
	});
}
while (!relayLines.some((line) => line.startsWith("LISTENING"))) await new Promise((r) => setTimeout(r, 50));

const build = await Bun.build({ entrypoints: [join(root, "interop/page.ts")], target: "browser", throw: false });
const [bundle] = build.outputs;
if (!build.success || bundle === undefined) throw new Error(`bundle failed: ${build.logs.join("\n")}`);
const script = await bundle.text();
const html = readFileSync(join(root, "interop/index.html"), "utf8");
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch(request) {
		const path = new URL(request.url).pathname;
		if (path.endsWith("page.js")) return new Response(script, { headers: { "content-type": "text/javascript" } });
		return new Response(html, { headers: { "content-type": "text/html" } });
	},
});

const browser = await (name === "chromium" ? chromium.launch({ channel: "chromium" }) : firefox.launch());
let failed = false;
const within = <T>(what: string, promise: Promise<T>, ms = 20_000): Promise<T> =>
	Promise.race([promise, new Promise<T>((_r, reject) => setTimeout(() => reject(new Error(`timed out: ${what}`)), ms))]);
try {
	say(`${name} ${browser.version()}`);
	const open = async (label: string) => {
		const page = await (await browser.newContext()).newPage();
		page.on("console", (message) => { const text = message.text(); if (!text.startsWith("trace ") && objects <= 1) console.log(`[${label}] ${text.slice(0, 200)}`); });
		page.on("pageerror", (error) => console.log(`[${label}] pageerror: ${error.message}`));
		await page.goto(`http://localhost:${server.port}/`);
		await page.waitForFunction(() => typeof (window as any).xmoq === "object");
		await within(`${label} connect`, page.evaluate(([url, hash]) => (window as any).xmoq.connect(url, hash), [`https://127.0.0.1:${values.port}`, sha]));
		say(`${label} connected`);
		return page;
	};
	const publisher = await open("pub");
	await within("publish", publisher.evaluate(() => (window as any).xmoq.publishTrack("xmoq/phase0", "object")));
	say("published");
	const subscriber = await open("sub");
	await within("subscribe", subscriber.evaluate(() => (window as any).xmoq.subscribe("xmoq/phase0", "object")));
	say("subscribed");

	let received = -1;
	for (let group = 0; group < objects; group += 1) {
		await within(`send Group ${group}`, publisher.evaluate(([g, hex]) => (window as any).xmoqOwn.sendObjectIn(g, hex, true), [String(group), payloadHex]));
		if ((group + 1) % 1000 === 0 || group === objects - 1) {
			try {
				await within(`receive Group ${group}`, subscriber.evaluate((g) => (window as any).xmoqOwn.objectOfGroup(g), String(group)), 15_000);
				received = group;
				say(`subscriber has Group ${group} (${group + 1} streams, ${((group + 1) * payloadHex.length / 2 / 1e6).toFixed(1)} MB of payload)`);
			} catch (error) {
				say(`STOPPED: ${String(error)}; the last Group seen was ${received}`);
				failed = true;
				break;
			}
		}
	}
	if (!failed && values["close-publisher"]) {
		// The publisher's page goes without a word from its application, as a closed tab does.
		const before = relayLines.length;
		await publisher.context().close();
		say("publisher's page closed");
		say(`subscriber: ${(await within("report", subscriber.evaluate(() => (window as any).xmoqOwn.subscriptionReport()), 30_000)).slice(0, 300)}`);
		for (const line of relayLines.slice(before)) console.log(`[relay after the close] ${line.slice(0, 200)}`);
	} else if (!failed) {
		await within("end the track", publisher.evaluate(() => (window as any).xmoqOwn.endTrack()));
		say(`publisher: peer ended its direction as ${await publisher.evaluate(() => (window as any).xmoqOwn.peerStreamEnd())}`);
		say(`subscriber: ${(await within("report", subscriber.evaluate(() => (window as any).xmoqOwn.subscriptionReport()))).slice(0, 300)}`);
	}
} catch (error) {
	failed = true;
	say(`FAILED ${String(error)}`);
} finally {
	await browser.close();
	server.stop(true);
	relay.kill();
	const interesting = relayLines.filter((line) => /CLOSE_SESSION|HANDSHAKE_FAILED|CONNECTION_ENDED|PUBLISH_DONE|RESET_|STOP_/.test(line));
	for (const line of interesting.slice(0, 12)) console.log(`[relay] ${line.slice(0, 200)}`);
	say(`relay lines: OPEN_DATA ${relayLines.filter((l) => l.startsWith("OPEN_DATA")).length}, FINISH_DATA ${relayLines.filter((l) => l.startsWith("FINISH_DATA")).length}, RESET_DATA ${relayLines.filter((l) => l.startsWith("RESET_DATA")).length}`);
}
process.exit(failed ? 1 : 0);
