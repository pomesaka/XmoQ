// Spike, outside lint and review: what a write that is under way is rejected
// with in Chromium when the page itself resets the stream, with
// `writer.abort(new WebTransportError({ streamErrorCode }))` as `@xmoq/moqt`
// does. The question: can that be told from a stop by the peer?
//
// Needs what run.ts needs. Run: bun spikes/wt-write-abort/own-reset.ts [--rounds 20]
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values: options } = parseArgs({
	options: { rounds: { type: "string", default: "20" }, port: { type: "string", default: "47331" } },
});
const here = import.meta.dir;
const root = join(here, "..", "..");
const cert = join(root, "interop/.cert/cert.pem");
const key = join(root, "interop/.cert/key.pem");
const certSha256 = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");

const server = spawn(join(here, "target/debug/xmoq-wt-write-abort-spike"), [options.port, cert, key], {
	stdio: ["ignore", "pipe", "inherit"],
});
process.on("exit", () => server.kill("SIGKILL"));
await new Promise<void>((resolve) => {
	server.stdout?.setEncoding("utf8");
	server.stdout?.on("data", (text: string) => {
		if (text.includes("LISTENING")) resolve();
	});
});

// Runs in the page. `mode` is what the writer does when it is reset.
async function sample([url, hashHex, kind, mode]: [string, string, string, string]): Promise<string> {
	const describe = (error: unknown): string => {
		// biome-ignore lint/suspicious/noExplicitAny: a value of the browser, looked at field by field
		const e = error as any;
		return `${Object.getPrototypeOf(e)?.constructor?.name} source=${e?.source} code=${e?.streamErrorCode} "${e?.message}"`;
	};
	const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((pair) => Number.parseInt(pair, 16)));
	// biome-ignore lint/suspicious/noExplicitAny: `WebTransport` is not in the types of this script
	const g = globalThis as any;
	const transport = new g.WebTransport(`${url}/hold`, {
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
	});
	await transport.ready;
	const writable: WritableStream<Uint8Array> =
		kind === "uni" ? await transport.createUnidirectionalStream() : (await transport.createBidirectionalStream()).writable;
	const writer = writable.getWriter();
	await writer.write(new Uint8Array([1]));
	let closed = "pending";
	writer.closed.then(
		() => {
			closed = "resolved";
		},
		(error: unknown) => {
			closed = describe(error);
		},
	);
	let write = "none";
	if (mode === "waiting") {
		write = "pending";
		writer.write(new Uint8Array(8 * 1024 * 1024)).then(
			() => {
				write = "resolved";
			},
			(error: unknown) => {
				write = describe(error);
			},
		);
		await new Promise((resolve) => setTimeout(resolve, 50));
	}
	// The form Chromium 156 takes (docs/experiments/2026-10-10-webtransport-error-in-chromium.md).
	await writer.abort(new g.WebTransportError({ message: "reset by the page", streamErrorCode: 5 })).catch(() => {});
	await new Promise((resolve) => setTimeout(resolve, 100));
	const later: string = await writer.write(new Uint8Array([2])).then(() => "resolved", describe);
	transport.close();
	return `${kind} | ${mode} | writer.closed: ${closed} | the write under way: ${write} | a later write: ${later}`;
}

const pageServer = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }),
});
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
await page.goto(`http://localhost:${pageServer.port}/`);
const counts = new Map<string, number>();
for (const kind of ["uni", "bidi"]) {
	for (const mode of ["idle", "waiting"]) {
		for (let round = 0; round < Number(options.rounds); round++) {
			const row = await page.evaluate(sample, [`https://127.0.0.1:${options.port}`, certSha256, kind, mode] satisfies [
				string,
				string,
				string,
				string,
			]);
			counts.set(row, (counts.get(row) ?? 0) + 1);
		}
	}
}
console.log(`browser=${browser.version()}`);
for (const [row, count] of [...counts.entries()].sort()) console.log(`${count} x ${row}`);
await browser.close();
pageServer.stop(true);
process.exit(0);
