// How much of the sound is in time under loss, with a longer buffer and
// with several chunks of audio in a Group, 10 seconds each, twice.
//
// Usage: bun audio-under-loss.ts <demo origin> <link port> <control origin> <repo>/interop
import { join } from "node:path";

const [origin, linkPort, control, interopDir] = process.argv.slice(2);
if (origin === undefined || linkPort === undefined || control === undefined || interopDir === undefined) {
	throw new Error("usage: bun audio-under-loss.ts <demo origin> <link port> <control origin> <interop dir>");
}
const { chromium } = await import(join(interopDir, "node_modules", "playwright-core", "index.mjs"));
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const loss = (share: number) => fetch(`${control}/set?loss=${share}&delay=0&jitter=0`);
const WAYS = [
	{ name: "as it is: a stream a chunk, buffer 150 ms", publish: "", watch: "" },
	{ name: "buffer 400 ms", publish: "", watch: "&buffer=400" },
	{ name: "5 chunks a Group (100 ms a stream)", publish: "&audioGroup=5", watch: "" },
	{ name: "25 chunks a Group (500 ms a stream)", publish: "&audioGroup=25", watch: "" },
];
const browser = await chromium.launch({ headless: true });
try {
	for (const share of [0.05, 0.1]) {
		for (const way of WAYS) {
			const runs: string[] = [];
			for (let run = 0; run < 2; run++) {
				const room = `audio-${Date.now()}`;
				await loss(0);
				const publisher = await (await browser.newContext()).newPage();
				await publisher.goto(`${origin}/publish?room=${room}${way.publish}`);
				await wait(1500);
				await loss(share);
				const viewer = await (await browser.newContext()).newPage();
				await viewer.goto(`${origin}/watch?room=${room}&via=${linkPort}${way.watch}`);
				await wait(10_000);
				const report = JSON.parse((await viewer.evaluate(() => document.body.dataset.report)) ?? "null");
				runs.push(
					report === null
						? "no report"
						: `${report.tooLate} of ${report.audioPieces} too late (came ${report.arrived.audio}; dropped ${JSON.stringify(report.dropped.audio)}); video ${report.shown} shown`,
				);
				await viewer.close();
				await publisher.close();
			}
			console.log(JSON.stringify({ loss: `${share * 100}%`, way: way.name, runs }));
		}
	}
} finally {
	await loss(0);
	await browser.close();
}
