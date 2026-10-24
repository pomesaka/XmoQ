// Spike, outside lint and review: whether Chromium gives the connection credit
// the peer frees to a stream by its `sendOrder`
// (docs/experiments/2026-10-25-send-order-of-data-streams-in-chromium.md).
//
// The page fills 16 unidirectional streams, each with the `sendOrder` of the mode,
// against a server whose connection window is smaller than the streams carry
// and that reads them slowly; then it writes a request of 64 KiB on a
// bidirectional stream with the mode's `sendOrder` for it, and the server says when it had
// all of it.
//
// Needs `cargo build --locked` in this directory and the certificate that an
// interop run creates under interop/.cert.
// Run: bun spikes/send-order-in-chromium/run.ts [--rounds 5] [--port 47340]
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: o } = parseArgs({ options: {
	rounds: { type: "string", default: "5" },
	port: { type: "string", default: "47340" },
} });
const here = import.meta.dir;
const root = join(here, "..", "..");
const cert = join(root, "interop/.cert/cert.pem"), key = join(root, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");

const lines: string[] = [];
const startServer = async () => {
	const server = spawn(join(here, "target/debug/xmoq-send-order-spike"), [o.port, cert, key], { stdio: ["ignore", "pipe", "inherit"] });
	server.stdout.setEncoding("utf8");
	let rest = "";
	server.stdout.on("data", (d: string) => { rest += d; const p = rest.split("\n"); rest = p.pop() ?? ""; lines.push(...p); });
	const end = Date.now() + 5000;
	while (!lines.some((l) => l.startsWith("LISTENING")) && Date.now() < end) await new Promise((r) => setTimeout(r, 50));
	return server;
};

const page = `<script>
globalThis.round = async (url, hash, sendOrder, requestOrder) => {
  const bytes = new Uint8Array(hash.match(/../g).map((h) => parseInt(h, 16)));
  const wt = new WebTransport(url, { serverCertificateHashes: [{ algorithm: "sha-256", value: bytes }] });
  await wt.ready;
  let writing = true;
  const chunk = new Uint8Array(16 * 1024);
  const options = sendOrder === null ? {} : { sendOrder };
  for (let i = 0; i < 16; i++) {
    const writer = (await wt.createUnidirectionalStream(options)).getWriter();
    (async () => { while (writing) { try { await writer.write(chunk); } catch { return; } } })();
  }
  // Time for the streams to take the credit there is.
  await new Promise((r) => setTimeout(r, 1500));
  const bidi = await wt.createBidirectionalStream(requestOrder === null ? {} : { sendOrder: requestOrder });
  const writer = bidi.writable.getWriter();
  const sentAt = Date.now();
  await writer.write(new Uint8Array(64 * 1024));
  const written = Date.now();
  // Time for the server to say.
  await new Promise((r) => setTimeout(r, 6000));
  writing = false;
  wt.close();
  return { sentAt, written };
};
</script>`;

const server = await startServer();
const http = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response(page, { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
console.log(`chromium ${browser.version()}`);
const modes = [["data -1, request none", -1, null], ["data none, request none", null, null], ["data -1, request 1", -1, 1], ["data 0, request 1000", 0, 1000]] as const;
const results: Record<string, number[]> = Object.fromEntries(modes.map(([name]) => [name, []]));
for (let round = 0; round < Number(o.rounds); round++) {
	for (const [name, sendOrder, requestOrder] of modes) {
		const tab = await (await browser.newContext()).newPage();
		await tab.goto(`http://127.0.0.1:${http.port}/`);
		const before = lines.length;
		const { sentAt } = await tab.evaluate(([u, h, s, r]) => (globalThis as any).round(u, h, s, r), [`https://127.0.0.1:${o.port}/`, sha, sendOrder, requestOrder] as const);
		const arrived = lines.slice(before).find((l) => l.startsWith("REQUEST bytes=65536"));
		const at = arrived ? Number(/at_ms=(\d+)/.exec(arrived)?.[1]) : Number.NaN;
		results[name].push(at - sentAt);
		console.log(`${name} round=${round} request_ms=${at - sentAt}`);
		await tab.close();
	}
}
for (const [name, ms] of Object.entries(results)) {
	const sorted = [...ms].sort((a, b) => a - b);
	console.log(`${name}: ${sorted.join(", ")} (median ${sorted[Math.floor(sorted.length / 2)]})`);
}
await browser.close();
http.stop();
server.kill();
