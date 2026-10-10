// Spike, outside lint and review: what `WebTransport.close({ closeCode, reason })` of
// Chromium sends, as the own relay reports it
// (docs/experiments/2026-10-10-webtransport-close-info-in-chromium.md).
// Needs `cargo build --locked -p xmoq-relay` and the certificate that an interop run
// creates under interop/.cert.
// Run: bun spikes/wt-close-info/run.ts
//   --write-fixtures: also writes fixtures/webtransport-close/reasons.tsv from what the
//     relay got for each reason.
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const W = join(import.meta.dir, "..", "..");
const port = "47310";
const cert = join(W, "interop/.cert/cert.pem"), key = join(W, "interop/.cert/key.pem");
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const lines: string[] = [];
const relay = spawn(join(W, "target/debug/xmoq-relay"), ["--port", port, "--cert-file", cert, "--key-file", key]);
for (const s of [relay.stdout, relay.stderr]) { s.setEncoding("utf8"); let b = ""; s.on("data", (d: string) => { b += d; const p = b.split("\n"); b = p.pop() ?? ""; lines.push(...p); }); }
const line = async (re: RegExp) => { for (let i = 0; i < 100; i++) { const l = lines.find((x) => re.test(x)); if (l) return l; await new Promise((r) => setTimeout(r, 50)); } return "(no line)"; };
await line(/^LISTENING/);
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
await page.goto(`http://localhost:${server.port}/`);

/** Undoes the escapes of Rust's `{:?}` for a string, which is how the relay prints the reason. */
function unescapeRustDebug(text: string): string {
	return text.replace(/\\(u\{([0-9a-f]+)\}|.)/g, (_all, escape: string, hex: string | undefined) => {
		if (hex !== undefined) return String.fromCodePoint(Number.parseInt(hex, 16));
		const simple: Record<string, string> = { n: "\n", r: "\r", t: "\t", "0": "\0" };
		return simple[escape] ?? escape;
	});
}
const toHex = (bytes: Uint8Array): string => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
/** UTF-16 code units, big endian, for a reason that is not valid text. */
const utf16Hex = (text: string): string => Array.from({ length: text.length }, (_v, i) => text.charCodeAt(i).toString(16).padStart(4, "0")).join("");
const fixtureRows: string[] = [];
/** What the relay got as the reason of the session closed last. */
let lastReason: string | undefined;

let session = 0;
/** Opens a session, closes it with `info` built in the page by `make`, and returns what the page and the relay saw. */
async function closeWith(label: string, make: string): Promise<void> {
	const n = session++;
	const inPage = await page.evaluate(async ([url, hashHex, source]) => {
		const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((h) => Number.parseInt(h, 16)));
		const wt = new (globalThis as any).WebTransport(url, { protocols: ["moqt-18"], serverCertificateHashes: [{ algorithm: "sha-256", value: hash }] });
		await wt.ready;
		const info = new Function(`return (${source});`)();
		try { wt.close(info); } catch (error) { return `close threw ${String(error)}`; }
		return wt.closed.then((c: any) => `closed resolved closeCode=${c.closeCode} reason_utf16=${c.reason.length}`, (e: unknown) => `closed rejected ${String(e)}`);
	}, [`https://127.0.0.1:${port}`, sha, make] as const);
	const ended = await line(new RegExp(`^CONNECTION_ENDED session=${n} `));
	lastReason = undefined;
	const escaped = ended.match(/reason: "(.*)" \}$/)?.[1];
	const reason = escaped === undefined ? undefined : unescapeRustDebug(escaped);
	lastReason = reason;
	const code = ended.match(/code: (\d+)/)?.[1] ?? ended.replace(/^CONNECTION_ENDED session=\d+ /, "");
	const bytes = reason === undefined ? "-" : String(new TextEncoder().encode(reason).length);
	const tail = reason === undefined ? "" : JSON.stringify(reason.slice(-3));
	console.log(`${label} | page: ${inPage} | relay: code=${code} reason_bytes=${bytes} ends=${tail}`);
}

for (const code of ["0", "255", "256", "4294967295", "4294967296", "-1", "1.5", "NaN"]) {
	await closeWith(`closeCode ${code}`, `{ closeCode: ${code}, reason: "r" }`);
}
// name, how the page builds the reason, and whether the reason is valid text (Rust can
// hold it) or only a sequence of UTF-16 code units.
const reasons: [string, string, "utf8" | "utf16"][] = [
	["empty", `""`, "utf8"],
	["short", `"bye"`, "utf8"],
	["1023-bytes", `"a".repeat(1023)`, "utf8"],
	["1024-bytes", `"a".repeat(1024)`, "utf8"],
	["1025-bytes", `"a".repeat(1025)`, "utf8"],
	["2-byte-character-across-the-limit", `"a".repeat(1023) + "é"`, "utf8"],
	["3-byte-character-across-the-limit", `"a".repeat(1022) + "あ"`, "utf8"],
	["4-byte-character-across-the-limit", `"a".repeat(1021) + "😀"`, "utf8"],
	["4-byte-character-one-byte-over", `"a".repeat(1023) + "😀"`, "utf8"],
	["4-byte-character-ending-at-the-limit", `"a".repeat(1020) + "😀"`, "utf8"],
	["leading-bom", `"\\ufeffabc"`, "utf8"],
	["bom-only", `"\\ufeff"`, "utf8"],
	["half-of-a-pair-at-the-end", `"a\\ud83d"`, "utf16"],
	["only-halves-of-pairs", `"\\ud83d\\ud83d"`, "utf16"],
];
for (const [name, make, encoding] of reasons) {
	await closeWith(`reason ${name}`, `{ closeCode: 0, reason: ${make} }`);
	const input: string = new Function(`return (${make});`)();
	// A row whose reason the relay did not report must not be written as an empty one.
	if (lastReason === undefined) throw new Error(`the relay reported no reason for "${name}"`);
	const inputHex = encoding === "utf8" ? toHex(new TextEncoder().encode(input)) : utf16Hex(input);
	fixtureRows.push([name, encoding, inputHex, toHex(new TextEncoder().encode(lastReason ?? ""))].join("\t"));
}
if (process.argv.includes("--write-fixtures")) {
	const dir = join(W, "fixtures", "webtransport-close");
	mkdirSync(dir, { recursive: true });
	writeFileSync(join(dir, "reasons.tsv"), `${fixtureRows.join("\n")}\n`);
	console.log(`wrote ${fixtureRows.length} rows to ${join(dir, "reasons.tsv")}`);
}

// The same through @xmoq/moqt, with the page of spikes/wt-error-form: what `closed`
// reports has to be what the relay got.
const build = await Bun.build({ entrypoints: [join(import.meta.dir, "..", "wt-error-form", "page.ts")], target: "browser" });
const js = await build.outputs[0].text();
const library = await browser.newPage();
await library.goto(`http://localhost:${server.port}/`);
await library.addScriptTag({ content: js, type: "module" });
await library.waitForFunction(() => typeof (globalThis as any).probe === "function");
await library.evaluate(([u, h]) => (globalThis as any).probe(u, h), [`https://127.0.0.1:${port}`, sha] as const);
const reported = JSON.parse(await library.evaluate(() => (globalThis as any).closeWith(5, "a".repeat(1023) + "é")));
const ended = await line(new RegExp(`^CONNECTION_ENDED session=${session} `));
const got = unescapeRustDebug(ended.match(/reason: "(.*)" \}$/)?.[1] ?? "");
console.log(`@xmoq/moqt close(5, 1023 x a + é) | closed: code=${reported.code} reason_bytes=${new TextEncoder().encode(reported.reason).length} | relay: code=${ended.match(/code: (\d+)/)?.[1]} reason_bytes=${new TextEncoder().encode(got).length} same=${got === reported.reason}`);
const refused = await library.evaluate(() => { try { (globalThis as any).closeWith(4294967296, "x"); return "not refused"; } catch (error) { return String(error); } });
console.log(`@xmoq/moqt close(4294967296) after the session is closed | ${refused}`);
await browser.close(); server.stop(true); relay.kill("SIGINT");
