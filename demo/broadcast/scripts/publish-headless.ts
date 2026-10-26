// Opens the publishing page of a demo in headless Chromium and stays until
// it is stopped: the clock's picture and the tone, no camera.
//
// Usage: bun demo/broadcast/scripts/publish-headless.ts http://localhost:48901 [room] [a line for the chat]
//
// With a room the page joins it (`?room=`); the line is typed into the
// page's chat 6 seconds after the page publishes. When stopped it prints
// the chat as the page shows it (PAGE_CHAT).
import { join } from "node:path";

const origin = process.argv[2] ?? "http://localhost:8080";
const room = process.argv[3];
const line = process.argv[4];
const { chromium } = await import(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"interop",
		"node_modules",
		"playwright-core",
		"index.mjs",
	)
);
// WHY the signals are not Playwright's: the chat is read from the page when this is stopped.
const browser = await chromium.launch({
	headless: true,
	handleSIGINT: false,
	handleSIGTERM: false,
});
const page = await (await browser.newContext()).newPage();
// PUBLISH_QUERY: more for the page's address, as `size=720`.
const query = [
	...(room === undefined ? [] : [`room=${encodeURIComponent(room)}`]),
	...(process.env.PUBLISH_QUERY ? [process.env.PUBLISH_QUERY] : []),
].join("&");
await page.goto(`${origin}/publish${query === "" ? "" : `?${query}`}`);
// WHY until the page says it publishes, and not a fixed wait: the scripts start
// the viewer at this line, and a viewer without a room that asks before the
// broadcast is there is refused (a first build of the pages takes longer).
// After 20 s the line says what the page shows instead.
const status = async (): Promise<string> =>
	((await page.textContent("#status")) ?? "").split("\n")[0] ?? "";
for (let waited = 0; waited < 20_000; waited += 250) {
	if ((await status()).startsWith("publishing")) break;
	await new Promise((resolve) => setTimeout(resolve, 250));
}
console.log(`PUBLISHING ${await status()}`);
if (line !== undefined) {
	setTimeout(async () => {
		await page.fill("form input", line);
		await page.press("form input", "Enter");
		console.log(`PAGE_SAID ${line}`);
	}, 6000);
}
const stop = async () => {
	if (room !== undefined) {
		for (const shown of await page.locator("#chat div").allTextContents()) {
			console.log(`PAGE_CHAT ${shown}`);
		}
	}
	await browser.close();
	process.exit(0);
};
// SIGUSR1: the page is loaded anew, which ends its broadcast and begins another.
process.on("SIGUSR1", async () => {
	await page.reload();
	console.log("PAGE_RELOADED");
});
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
setInterval(() => {}, 60_000);
