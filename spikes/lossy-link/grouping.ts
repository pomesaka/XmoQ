// What packing several chunks of audio into a Group costs and gains: how
// long a viewer that comes late waits for its first sound, and how much
// of the sound is too late under loss.
//
// Usage: bun grouping.ts <demo origin> <link port> <control origin> <repo>/interop
import { join } from "node:path";

const [origin, linkPort, control, interopDir] = process.argv.slice(2);
if (origin === undefined || linkPort === undefined || control === undefined || interopDir === undefined) {
	throw new Error("usage: bun grouping.ts <demo origin> <link port> <control origin> <interop dir>");
}
const { chromium } = await import(join(interopDir, "node_modules", "playwright-core", "index.mjs"));
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const link = (setting: string) => fetch(`${control}/set?loss=0&delay=0&jitter=0&up=0&${setting}`);
const browser = await chromium.launch({ headless: true });
const open = async (path: string) => {
	const page = await (await browser.newContext()).newPage();
	await page.goto(`${origin}${path}`);
	return page;
};
const report = async (page: Awaited<ReturnType<typeof open>>) =>
	JSON.parse((await page.evaluate(() => document.body.dataset.report)) ?? "null");
try {
	await link("down=0");
	for (const [chunks, join_] of [[1, ""], [5, ""], [10, ""], [25, ""], [25, "&audioJoin=1"]] as const) {
		// 1. A viewer that comes while another watches: its first sound.
		const room = `late-${chunks}-${Date.now()}`;
		const publisher = await open(`/publish?room=${room}&audioGroup=${chunks}`);
		await wait(1500);
		const keeper = await open(`/watch?room=${room}`);
		await wait(1500);
		const decoded: number[] = [];
		const sounded: number[] = [];
		for (let viewer = 0; viewer < 5; viewer++) {
			await wait(130 + viewer * 90);
			const late = await open(`/watch?room=${room}${join_}`);
			await wait(1800);
			const seen = await report(late);
			decoded.push(seen?.firstAudioAfterMs ?? -1);
			sounded.push(seen?.firstSoundAfterMs ?? -1);
			await late.close();
		}
		console.log(JSON.stringify({ chunksAGroup: chunks, joiningFetch: join_ !== "", firstDecodedAfterMs: decoded, firstInTimeAfterMs: sounded }));
		await keeper.close();
		await publisher.close();
	}
	// 2. Under loss from the relay, 10 seconds.
	for (const chunks of [1, 5, 10, 25]) {
		for (const share of [0.05, 0.1]) {
			const room = `loss-${chunks}-${Date.now()}`;
			await link("down=0");
			const publisher = await open(`/publish?room=${room}&audioGroup=${chunks}`);
			await wait(1500);
			await link(`down=${share}`);
			const viewer = await open(`/watch?room=${room}&via=${linkPort}`);
			await wait(10_000);
			const seen = await report(viewer);
			console.log(JSON.stringify({ chunksAGroup: chunks, loss: `${share * 100}%`, tooLate: `${seen?.tooLate} of ${seen?.audioPieces}`, came: seen?.arrived.audio, dropped: seen?.dropped.audio }));
			await viewer.close();
			await publisher.close();
		}
	}
	// 3. The video with a stream for every Object, under the same loss, once.
	for (const mapping of ["stream-per-object", "stream-per-group"]) {
		const room = `video-${Date.now()}`;
		await link("down=0");
		const publisher = await open(`/publish?room=${room}&mapping=${mapping}&audio=none`);
		await wait(1500);
		await link("down=0.1");
		const viewer = await open(`/watch?room=${room}&via=${linkPort}`);
		await wait(10_000);
		const seen = await report(viewer);
		console.log(JSON.stringify({ video: mapping, loss: "10%", came: seen?.arrived.video, shown: `${seen?.shown} of ${seen?.videoFrames}`, dropped: seen?.dropped.video, stalls: seen?.stalls.length }));
		await viewer.close();
		await publisher.close();
	}
} finally {
	await link("down=0");
	await browser.close();
}
