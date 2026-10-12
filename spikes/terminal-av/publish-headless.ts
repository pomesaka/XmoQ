// Opens the publishing page of a demo in headless Chromium and stays until
// it is stopped: the clock's picture and the tone, no camera.
//
// Usage: bun spikes/terminal-av/publish-headless.ts http://localhost:48901
import { join } from "node:path";

const origin = process.argv[2] ?? "http://localhost:8080";
const { chromium } = await import(
	join(import.meta.dir, "..", "..", "interop", "node_modules", "playwright-core", "index.mjs")
);
const browser = await chromium.launch({ headless: true });
const page = await (await browser.newContext()).newPage();
await page.goto(`${origin}/publish`);
await new Promise((resolve) => setTimeout(resolve, 2000));
console.log(`PUBLISHING ${((await page.textContent("#status")) ?? "").split("\n")[0]}`);
const stop = async () => {
	await browser.close();
	process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
setInterval(() => {}, 60_000);
