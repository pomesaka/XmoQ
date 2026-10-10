// Bundles page.ts, serves it on localhost and drives it in a real headless Chromium.
//
//   bun run.ts --role publish --relay http://127.0.0.1:4443/spike \
//       --broadcast demo --payload hello [--protocols moqt-22] [--hold 30]
//
// CHROMIUM_PATH must point at a Chromium/Chrome binary. Playwright's own browser
// download is deliberately not used, so nothing is installed outside this directory.
import { parseArgs } from "node:util";
import { chromium } from "playwright-core";

const { values } = parseArgs({
	options: {
		role: { type: "string" },
		relay: { type: "string" },
		broadcast: { type: "string" },
		track: { type: "string", default: "object" },
		payload: { type: "string" },
		// Comma-separated WebTransport subprotocols. Omit to use the library default offer.
		protocols: { type: "string" },
		// Seconds to wait for the role to finish.
		timeout: { type: "string", default: "20" },
		// Publisher only: seconds to keep the page open after publishing, so a subscriber can attach.
		hold: { type: "string", default: "0" },
		// Publisher only: rewrite the payload as a new group every N milliseconds (live delivery).
		interval: { type: "string" },
	},
});

function required(name: "role" | "relay" | "broadcast" | "payload"): string {
	const value = values[name];
	if (!value) throw new Error(`--${name} is required`);
	return value;
}

const executablePath = process.env.CHROMIUM_PATH;
if (!executablePath) throw new Error("CHROMIUM_PATH is required");

const build = await Bun.build({ entrypoints: [`${import.meta.dir}/page.ts`], target: "browser" });
if (!build.success) throw new AggregateError(build.logs, "failed to bundle page.ts");
const script = await build.outputs[0].text();

const html = `<!doctype html><meta charset="utf-8"><title>xmoq moq-dev spike</title><script type="module" src="/page.js"></script>`;
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch(request) {
		const { pathname } = new URL(request.url);
		if (pathname === "/page.js") return new Response(script, { headers: { "content-type": "text/javascript" } });
		return new Response(html, { headers: { "content-type": "text/html" } });
	},
});

const query = new URLSearchParams({
	role: required("role"),
	relay: required("relay"),
	broadcast: required("broadcast"),
	track: values.track,
	payload: required("payload"),
});
if (values.protocols) query.set("protocols", values.protocols);
if (values.interval) query.set("interval", values.interval);

const browser = await chromium.launch({ executablePath, headless: true });
let ok = false;
try {
	console.log(`BROWSER ${browser.version()}`);
	const page = await browser.newPage();
	page.on("console", (message) => console.log(`[console.${message.type()}] ${message.text()}`));
	page.on("pageerror", (error) => console.log(`[pageerror] ${error.message}`));

	// WHY 127.0.0.1: http://127.0.0.1 is a secure context, which WebTransport requires,
	// and it matches the address the page server is bound to.
	await page.goto(`http://127.0.0.1:${server.port}/?${query}`);
	await page.waitForFunction(() => window.__spike?.done === true, undefined, {
		timeout: Number(values.timeout) * 1000,
	});

	const result = await page.evaluate(() => window.__spike);
	console.log(`RESULT ${JSON.stringify(result)}`);
	ok = result.ok;

	const hold = Number(values.hold);
	if (ok && hold > 0) {
		console.log(`HOLDING ${hold}s`);
		await page.waitForTimeout(hold * 1000);
	}
} finally {
	await browser.close();
	server.stop(true);
}

process.exit(ok ? 0 : 1);
