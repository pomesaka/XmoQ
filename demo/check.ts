// Checks the demo that `bun demo/serve.ts` serves, in headless Chromium:
// the picture is shown at the rate it is made, **sound leaves the watching
// page for the loudspeaker** (what goes into the AudioContext's output is
// listened to; nobody hears it), and it leaves without breaks.
//
// Usage: bun demo/check.ts [http://localhost:8080] [more of the publishing page's address]
//
// The second argument is for trying something out: `&timer=slow` gives the
// publishing page the timers of a tab in the background, and the check
// then fails as the demo did there.
//
// Chromium is the one Playwright installed for the interop runs
// (interop/node_modules). What a tab in the background does is not checked
// here: Playwright keeps every page visible.
import { join } from "node:path";

/** What the callbacks of `evaluate` and `waitForFunction` read, in the page. */
declare const document: {
	readonly body: { readonly dataset: Record<string, string | undefined> };
};

const origin = process.argv[2] ?? "http://localhost:8080";
const { chromium } = await import(
	join(
		import.meta.dir,
		"..",
		"interop",
		"node_modules",
		"playwright-core",
		"index.mjs",
	)
);
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const browser = await chromium.launch({ headless: true });
const failures: string[] = [];
try {
	const room = `check-${Date.now() % 1_000_000}`;
	const publisher = await (await browser.newContext()).newPage();
	await publisher.goto(
		`${origin}/publish?room=${room}${process.argv[3] ?? ""}`,
	);
	await wait(2000);
	const viewer = await (await browser.newContext()).newPage();
	await viewer.goto(`${origin}/watch?room=${room}`);
	await wait(1500);
	await viewer.click("#sound");
	// The loudest moment of 4 seconds, and the rates at their end.
	let peak = 0;
	let hz = 0;
	let report: {
		shownASecond: number;
		decodedASecond: number;
		tooLate: number;
		sound: {
			state: string;
			soundedASecond: number;
			breaks: number;
			leaving: { peak: number; hz: number } | null;
		};
	} | null = null;
	const breaksBefore: number =
		JSON.parse(
			(await viewer.evaluate(() => document.body.dataset.report)) ?? "null",
		)?.sound.breaks ?? 0;
	for (let tick = 0; tick < 16; tick++) {
		await wait(250);
		report = JSON.parse(
			(await viewer.evaluate(() => document.body.dataset.report)) ?? "null",
		);
		const leaving = report?.sound.leaving;
		if (leaving !== null && leaving !== undefined && leaving.peak > peak) {
			peak = leaving.peak;
			hz = leaving.hz;
		}
	}
	console.log(((await publisher.textContent("#status")) ?? "").split("\n")[0]);
	console.log((await viewer.textContent("#status")) ?? "");
	if (report === null) {
		failures.push("the watching page has no report");
	} else {
		if (report.shownASecond < 25)
			failures.push(
				`${report.shownASecond} frames a second are shown, where 30 are made`,
			);
		if (report.sound.state !== "running")
			failures.push(
				`the AudioContext is ${report.sound.state} after the click`,
			);
		if (report.sound.soundedASecond < 40)
			failures.push(
				`${report.sound.soundedASecond} pieces of audio a second go to the loudspeaker, where 50 are made`,
			);
		// The tone is 0.25 of full scale between 220 and 660 Hz, the beep 0.6 at 1000 Hz.
		if (peak < 0.1)
			failures.push(
				`what leaves for the loudspeaker is ${peak.toFixed(3)} of full scale at most: silence`,
			);
		if (hz < 150 || hz > 1200)
			failures.push(
				`the loudest pitch is ${hz} Hz, where the tone is 220 to 660 and the beep 1000`,
			);
		// A step from one sample to the next that the tone does not make: a
		// piece of audio that does not join the one before it. 100 a second
		// where every piece begins at its own time.
		const breaks = report.sound.breaks - breaksBefore;
		console.log(`breaks in the sound: ${breaks} in 4 seconds`);
		if (breaks > 10)
			failures.push(`${breaks} breaks in 4 seconds of sound: it crackles`);
	}
	console.log(
		`sound leaving: ${peak.toFixed(2)} of full scale at most, at ${hz} Hz`,
	);
} finally {
	await browser.close();
}
for (const failure of failures) console.log(`FAILED: ${failure}`);
console.log(
	failures.length === 0
		? "THE DEMO PLAYS: picture and sound"
		: "THE DEMO DOES NOT PLAY AS IT SHOULD",
);
process.exitCode = failures.length === 0 ? 0 : 1;
