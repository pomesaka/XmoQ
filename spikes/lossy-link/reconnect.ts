// Cuts the link of the viewer, or of the publisher, for longer than the
// relay's idle timeout, and says how long picture and sound take to come
// back after the link does.
//
// Usage: bun reconnect.ts <demo origin> <link port> <control origin> <repo>/interop
import { join } from "node:path";

const [origin, linkPort, control, interopDir] = process.argv.slice(2);
if (origin === undefined || linkPort === undefined || control === undefined || interopDir === undefined) {
	throw new Error("usage: bun reconnect.ts <demo origin> <link port> <control origin> <interop dir>");
}
const { chromium } = await import(join(interopDir, "node_modules", "playwright-core", "index.mjs"));
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const loss = (share: number) => fetch(`${control}/set?loss=${share}&delay=0&jitter=0`);
const CUT_SECONDS = 8;

const browser = await chromium.launch({ headless: true });
try {
	for (const whose of ["viewer", "publisher"]) {
		const room = `cut-${whose}-${Date.now()}`;
		await loss(0);
		const publisher = await (await browser.newContext()).newPage();
		await publisher.goto(`${origin}/publish?room=${room}${whose === "publisher" ? `&via=${linkPort}` : ""}`);
		await wait(1500);
		const viewer = await (await browser.newContext()).newPage();
		await viewer.goto(`${origin}/watch?room=${room}${whose === "viewer" ? `&via=${linkPort}` : ""}`);
		await viewer.click("#sound");
		await wait(4000);
		const read = async () => JSON.parse((await viewer.evaluate(() => document.body.dataset.report)) ?? "null");
		const before = await read();
		await loss(1);
		const lines: string[] = [];
		for (let second = 1; second <= CUT_SECONDS; second++) {
			await wait(1000);
			lines.push(`${second}s: ${(await read())?.link}`);
		}
		await loss(0);
		const restored = Date.now();
		let picture: number | undefined;
		let sound: number | undefined;
		let last = await read();
		for (let tick = 0; tick < 150 && (picture === undefined || sound === undefined); tick++) {
			await wait(100);
			last = await read();
			if (picture === undefined && last.lastShown > restored) picture = last.lastShown - restored;
			if (sound === undefined && last.lastHeard !== null && last.lastHeard > restored) sound = last.lastHeard - restored;
		}
		console.log(
			JSON.stringify({
				cut: `the ${whose}'s link, ${CUT_SECONDS} s`,
				before: `${before.shown} frames shown, ${before.audioPieces} pieces`,
				whileCut: [lines[0], lines[5], lines[7]],
				pictureBackAfterMs: picture ?? "not in 15 s",
				soundBackAfterMs: sound ?? "not in 15 s",
				after: `${last.link}; session ${last.sessions}`,
				publisher: await publisher.evaluate(() => document.body.dataset.link ?? "its first session"),
			}),
		);
		await viewer.close();
		await publisher.close();
	}
} finally {
	await loss(0);
	await browser.close();
}
