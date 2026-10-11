// Runs the demo's watching page through the lossy link, one condition after
// the other, and prints one line for each.
//
// Usage: bun measure.ts <demo origin> <link port> <control origin> <repo>/interop
//
// The demo (`bun demo/serve.ts`) and the link (`node proxy.mjs`) are started
// by the caller.
import { join } from "node:path";

const [origin, linkPort, control, interopDir] = process.argv.slice(2);
if (origin === undefined || linkPort === undefined || control === undefined || interopDir === undefined) {
	throw new Error("usage: bun measure.ts <demo origin> <link port> <control origin> <interop dir>");
}
const { chromium } = await import(join(interopDir, "node_modules", "playwright-core", "index.mjs"));
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const set = (link: { loss: number; delay: number; jitter: number }) =>
	fetch(`${control}/set?loss=${link.loss}&delay=${link.delay}&jitter=${link.jitter}`);

interface Condition {
	name: string;
	loss: number;
	delay: number;
	jitter: number;
	/** Seconds without any datagram, in the middle of the run. */
	cut?: number;
}
const CONDITIONS: Condition[] = [
	{ name: "nothing", loss: 0, delay: 0, jitter: 0 },
	{ name: "loss 1%", loss: 0.01, delay: 0, jitter: 0 },
	{ name: "loss 5%", loss: 0.05, delay: 0, jitter: 0 },
	{ name: "loss 10%", loss: 0.1, delay: 0, jitter: 0 },
	{ name: "delay 50 ms", loss: 0, delay: 50, jitter: 0 },
	{ name: "delay 200 ms", loss: 0, delay: 200, jitter: 0 },
	{ name: "delay 50 ms, jitter 30 ms, loss 1%", loss: 0.01, delay: 50, jitter: 30 },
	{ name: "cut for 3 s", loss: 0, delay: 0, jitter: 0, cut: 3 },
	{ name: "cut for 8 s", loss: 0, delay: 0, jitter: 0, cut: 8 },
];
const SECONDS = 10;

const browser = await chromium.launch({ headless: true });
try {
	for (const mapping of ["stream-per-group", "stream-per-object"]) {
		for (const condition of CONDITIONS) {
			const room = `link-${Date.now()}`;
			await set({ loss: 0, delay: 0, jitter: 0 });
			const publisher = await (await browser.newContext()).newPage();
			await publisher.goto(`${origin}/publish?room=${room}&mapping=${mapping}`);
			await wait(1500);
			await set(condition);
			const viewer = await (await browser.newContext()).newPage();
			await viewer.goto(`${origin}/watch?room=${room}&via=${linkPort}`);
			let restored: number | undefined;
			if (condition.cut === undefined) {
				await wait(SECONDS * 1000);
			} else {
				await wait(4000);
				await set({ loss: 1, delay: 0, jitter: 0 });
				await wait(condition.cut * 1000);
				await set({ loss: 0, delay: 0, jitter: 0 });
				restored = Date.now();
				await wait(6000);
			}
			const status = ((await viewer.textContent("#status")) ?? "").replace(/\s+/g, " ");
			const report = JSON.parse((await viewer.evaluate(() => document.body.dataset.report)) ?? "null");
			if (report === null) {
				console.log(JSON.stringify({ mapping, condition: condition.name, failed: status.slice(0, 200) }));
			} else {
				const stalls: { from: number; to: number }[] = report.stalls;
				const longest = Math.max(0, ...stalls.map((stall) => stall.to - stall.from));
				console.log(
					JSON.stringify({
						mapping,
						condition: condition.name,
						frames: `${report.shown} shown of ${report.videoFrames} decoded`,
						came: report.arrived.video,
						shown: report.played.video,
						droppedVideo: report.dropped.video,
						audio: `${report.audioPieces} pieces, ${report.tooLate} too late`,
						droppedAudio: report.dropped.audio,
						stalls: `${stalls.length}, the longest ${Math.round(longest)} ms, ${Math.round(stalls.reduce((sum, stall) => sum + stall.to - stall.from, 0))} ms in all`,
						...(restored === undefined
							? {}
							: {
									afterTheCut:
										report.lastShown !== null && report.lastShown > restored
											? `a frame ${Math.round(Math.min(...stalls.filter((stall) => stall.to > (restored ?? 0)).map((stall) => stall.to), report.lastShown) - restored)} ms after the link came back`
											: `no frame after the link came back: ${status.slice(0, 160)}`,
								}),
					}),
				);
			}
			await viewer.close();
			await publisher.close();
		}
	}
} finally {
	await set({ loss: 0, delay: 0, jitter: 0 });
	await browser.close();
}
