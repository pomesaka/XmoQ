// Spike, outside lint and review: does a browser open a WebTransport session to the own
// relay with a pinned certificate and the protocol "moqt-18"?
// Needs `cargo build --locked -p xmoq-relay` and the certificate of an interop run.
// Run from the repository root: bun spikes/browser-support/connect.ts <chromium|firefox> [port]
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, firefox } from "../../interop/node_modules/playwright-core/index.mjs";

const name = process.argv[2] ?? "";
if (name !== "chromium" && name !== "firefox") throw new Error("usage: connect.ts <chromium|firefox> [port]");
const port = process.argv[3] ?? "47400";
const root = join(import.meta.dir, "..", "..");
const cert = join(root, "interop/.cert/cert.pem");
const key = join(root, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");

const relay = spawn(join(root, "target/debug/xmoq-relay"), ["--port", port, "--cert-file", cert, "--key-file", key]);
const lines: string[] = [];
for (const stream of [relay.stdout, relay.stderr]) {
	stream.setEncoding("utf8");
	let rest = "";
	stream.on("data", (data: string) => {
		rest += data;
		const parts = rest.split("\n");
		rest = parts.pop() ?? "";
		for (const line of parts) { lines.push(line); console.log(`[relay] ${line}`); }
	});
}
const giveUp = setTimeout(() => { console.log("gave up after 60 s"); relay.kill(); process.exit(2); }, 60_000);
while (!lines.some((line) => line.startsWith("LISTENING"))) await new Promise((r) => setTimeout(r, 50));

const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }) });
const browser = await (name === "chromium" ? chromium.launch({ channel: "chromium" }) : firefox.launch());
try {
	const page = await browser.newPage();
	page.on("console", (message) => console.log(`[page] ${message.text()}`));
	await page.goto(`http://localhost:${server.port}/`);
	const result = await page.evaluate(async ([url, hashHex]) => {
		const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((h) => Number.parseInt(h, 16)));
		const describe = (error: unknown) => {
			const e = error as { name?: string; message?: string; source?: string; streamErrorCode?: number | null };
			return `${e?.name}: ${e?.message} source=${e?.source} streamErrorCode=${e?.streamErrorCode}`;
		};
		let wt: any;
		try {
			wt = new (globalThis as any).WebTransport(url, { protocols: ["moqt-18"], serverCertificateHashes: [{ algorithm: "sha-256", value: hash }] });
		} catch (error) {
			return `constructor threw ${describe(error)}`;
		}
		const closed = wt.closed.then((info: any) => `closed resolved ${JSON.stringify(info)}`, (error: unknown) => `closed rejected ${describe(error)}`);
		const ready = wt.ready.then(() => `ready; protocol=${JSON.stringify(wt.protocol)}`, (error: unknown) => `ready rejected ${describe(error)}`);
		const first = await Promise.race([ready, new Promise<string>((r) => setTimeout(() => r("ready did not settle in 15 s"), 15_000))]);
		if (!first.startsWith("ready;")) return `${first} | ${await Promise.race([closed, new Promise<string>((r) => setTimeout(() => r("closed did not settle"), 2000))])}`;
		wt.close({ closeCode: 0, reason: "probe done" });
		return `${first} | ${await closed}`;
	}, [`https://127.0.0.1:${port}`, sha] as const);
	console.log(`${name} ${browser.version()}: ${result}`);
	await new Promise((r) => setTimeout(r, 500));
} finally {
	await browser.close();
	server.stop(true);
	relay.kill();
	clearTimeout(giveUp);
}
