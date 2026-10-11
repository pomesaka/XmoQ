// Spike, outside lint and review: a server opens a request to a page that
// runs `@xmoq/moqt`, is refused, and does not end its direction of the
// stream. The question: is that direction stopped after the 3 seconds the
// library waits, with which code, and does the session stay open?
//
// Needs what run.ts needs. Run: bun spikes/wt-write-abort/moqt-refused.ts [--rounds 5]
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: options } = parseArgs({
	options: { rounds: { type: "string", default: "5" }, port: { type: "string", default: "47341" } },
});
const here = import.meta.dir;
const root = join(here, "..", "..");
const cert = join(root, "interop/.cert/cert.pem");
const key = join(root, "interop/.cert/key.pem");
const certSha256 = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const fixture = (name: string): string => readFileSync(join(root, "fixtures/moqt-18", `${name}.hex`), "utf8").trim();
// The captured SUBSCRIBE, with a Request ID of the server.
const subscribe = fixture("subscribe-moqtail-rs").split(/\s+/);
subscribe[3] = "01";

const lines: string[] = [];
const server = spawn(join(here, "target/debug/xmoq-wt-write-abort-spike"), [options.port, cert, key], {
	stdio: ["ignore", "pipe", "inherit"],
	env: { ...process.env, XMOQ_SETUP_HEX: fixture("setup-moqtail-relay"), XMOQ_SUBSCRIBE_HEX: subscribe.join(" "), XMOQ_IDLE_SECS: "30" },
});
process.on("exit", () => server.kill("SIGKILL"));
await new Promise<void>((resolve) => {
	server.stdout?.setEncoding("utf8");
	let rest = "";
	server.stdout?.on("data", (text: string) => {
		const parts = (rest + text).split("\n");
		rest = parts.pop() ?? "";
		for (const line of parts) {
			if (line.includes("LISTENING")) resolve();
			else lines.push(line);
		}
	});
});

const build = await Bun.build({ entrypoints: [join(here, "moqt-entry.ts")], target: "browser", format: "iife" });
const script = await build.outputs[0]?.text();
if (script === undefined) throw new Error("the page script was not built");

// Runs in the page: connects, rejects a SUBSCRIBE it is asked for, and says
// after 4.5 seconds whether the session is still open.
async function sample([url, hashHex]: [string, string]): Promise<string> {
	const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((pair) => Number.parseInt(pair, 16)));
	// biome-ignore lint/suspicious/noExplicitAny: the global the page script sets
	const connection = await (globalThis as any).xmoqConnect(url, {
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
	});
	// biome-ignore lint/suspicious/noExplicitAny: the request of the library, used for one call
	connection.serve({ subscribe: (request: any) => void request.reject(0x10n, "no such track") });
	let closed = "open";
	connection.closed.then((close: unknown) => {
		closed = `closed ${JSON.stringify(close, (_key, value) => (typeof value === "bigint" ? String(value) : value))}`;
	});
	await new Promise((resolve) => setTimeout(resolve, 4500));
	const state = closed;
	connection.close();
	return state;
}

const pageServer = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }),
});
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
await page.goto(`http://localhost:${pageServer.port}/`);
await page.addScriptTag({ content: script });
console.log(`browser=${browser.version()}`);
for (const path of ["/moqt-refused", "/moqt-rejected", "/moqt-refused-fin"]) {
	for (let round = 0; round < Number(options.rounds); round++) {
		lines.length = 0;
		const state = await page.evaluate(sample, [`https://127.0.0.1:${options.port}${path}`, certSha256] satisfies [string, string]);
		await new Promise((resolve) => setTimeout(resolve, 200));
		console.log(`${path} | the page after 4.5 s: ${state} | the server: ${lines.join(" | ")}`);
	}
}
await browser.close();
pageServer.stop(true);
process.exit(0);
