// Which of the APIs XmoQ needs a Playwright build of a browser has.
// Run from the repository root: bun spikes/browser-support/probe.ts <chromium|firefox|webkit>
import { chromium, firefox, webkit } from "../../interop/node_modules/playwright-core";

const engines = { chromium, firefox, webkit };
const name = process.argv[2] ?? "";
if (name !== "chromium" && name !== "firefox" && name !== "webkit") {
	throw new Error("usage: probe.ts <chromium|firefox|webkit>");
}

// WHY a limit on everything: a browser that does not start must not leave
// this script, and its process, waiting.
const browser = await engines[name].launch({
	...(name === "chromium" ? { channel: "chromium" } : {}),
	timeout: 30_000,
});
try {
	const page = await browser.newPage();
	page.setDefaultTimeout(15_000);
	// A secure context without a server: the APIs are exposed only there.
	await page.route("https://probe.test/", (route) =>
		route.fulfill({ contentType: "text/html", body: "<title>probe</title>" }),
	);
	await page.goto("https://probe.test/");
	const found = await page.evaluate(() => ({
		userAgent: navigator.userAgent,
		secure: isSecureContext,
		WebTransport: typeof WebTransport,
		WebTransportError: typeof WebTransportError,
		VideoEncoder: typeof VideoEncoder,
		VideoDecoder: typeof VideoDecoder,
		AudioEncoder: typeof AudioEncoder,
		AudioDecoder: typeof AudioDecoder,
	}));
	console.log(name, browser.version(), JSON.stringify(found));
} finally {
	await browser.close();
}
