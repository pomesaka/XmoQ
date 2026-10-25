// A relay that is full sends the next session to another (`--when-full`):
// a GOAWAY right after its SETUP names that relay, with `sent-away=1` in the
// query, and a relay that is full itself closes a session that came so
// rather than send it on. The relay of the run is the one above
// (`--max-sessions 2`, `--when-full` the second); the second is below it
// (`--upstream`), on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET. A page
// publishes the bars at the relay above, a first viewer plays them at the
// relay below, which makes the relay below's session upstream the second of
// the relay above, and a second viewer comes to the relay above.
//
// Usage: bun when-full.ts --case moves|both-full [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run:
//
//   - moves: the relay above prints SESSION_SENT_AWAY for the second viewer
//     with the URL of the relay below and `?sent-away=1`; the second viewer
//     says `moved` with that URI and then plays at the relay below: it draws
//     frames on its first session that plays (`keepPlaying` counts those).
//   - both-full: the relay below has `--max-sessions 1` and `--when-full`
//     the relay above. The relay above sends the second viewer away once;
//     the relay below closes it (SESSION_REFUSED) and does not send it back:
//     after 5 seconds the relay above has printed SESSION_SENT_AWAY once,
//     and the second viewer has drawn no frame.

import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openPage,
	runCleanups,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageKeptReport,
	PageVideoSetup,
	XmoqPage,
} from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		case: { type: "string", default: "moves" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24396" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const NAMESPACE = "xmoq/video";
/** The bars of `interop/video.ts`. */
const BARS: PageVideoSetup = {
	trackName: "video",
	codec: "avc1.42E01F",
	catalogCodec: "avc3.42E01F",
	width: 320,
	height: 180,
	framerate: 25,
	bitrate: 500_000,
	keyFrameIntervalMs: 1000,
	firstTimestamp: 5_000_000,
	noise: false,
	mapping: "stream-per-group",
};
/** How long the second viewer is watched where it is not to play, and the relay above for a second session sent away. */
const WATCHED_MS = 5_000;

const sleep = (ms: number): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, ms));

async function report(page: Page): Promise<PageKeptReport> {
	return page.evaluate(() => window.xmoqOwn.keptReport());
}

async function play(page: Page, url: string, hash: string): Promise<void> {
	await page.evaluate(
		([at, certificate, namespace, name, codec]) =>
			window.xmoqOwn.keepVideoPlayed(at, certificate, namespace, name, codec),
		[url, hash, NAMESPACE, BARS.trackName, BARS.catalogCodec] satisfies [
			string,
			string,
			string,
			string,
			string,
		],
	);
}

/** Waits until the page has drawn `frames` frames on its `session`-th session. */
async function drawn(
	page: Page,
	session: number,
	frames: number,
): Promise<void> {
	for (;;) {
		const now = await report(page);
		if (now.shown.filter((frame) => frame.session === session).length >= frames)
			return;
		await sleep(100);
	}
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const bothFull = options.case === "both-full";
	if (!bothFull && options.case !== "moves") {
		throw new Error(`--case is moves or both-full, not ${options.case}`);
	}
	let summary: string;
	try {
		const belowPort = String(
			Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET,
		);
		const belowUrl = `https://127.0.0.1:${belowPort}`;
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
			relayArgs: ["--max-sessions", "2", "--when-full", belowUrl],
		});
		const below = await startSecondOwnRelay(
			belowPort,
			context,
			bothFull ? ["--max-sessions", "1", "--when-full", context.relayUrl] : [],
		);
		const publisher = await openPage(context, "publisher", "own-browser");
		await publisher.evaluate(
			([url, hash, namespace, setup]) =>
				window.xmoqOwn.keepVideoPublished(url, hash, namespace, setup),
			[context.relayUrl, context.certSha256Hex, NAMESPACE, BARS] satisfies [
				string,
				string,
				string,
				PageVideoSetup,
			],
		);
		const first = await openPage(context, "viewer:first", "own-browser");
		await play(first, below.url, context.certSha256Hex);
		await withTimeout(
			drawn(first, 1, 25),
			"the first viewer to play at the relay below",
		);

		const second = await openPage(context, "viewer:second", "own-browser");
		await play(second, context.relayUrl, context.certSha256Hex);
		const sentTo = `${belowUrl}?sent-away=1`;
		await withTimeout(
			context.relay.waitForLines(
				/^SESSION_SENT_AWAY session=\d+ uri=(\S+)$/,
				1,
			),
			"the relay above to send the second viewer away",
		);
		if (
			!context.relay.hasPrinted(
				new RegExp(
					`^SESSION_SENT_AWAY session=\\d+ uri=${sentTo.replaceAll(".", "\\.").replaceAll("?", "\\?")}$`,
				),
			)
		) {
			throw new Error(
				`the relay above did not send the second viewer to ${sentTo}`,
			);
		}
		if (!bothFull) {
			await withTimeout(
				drawn(second, 1, 25),
				"the second viewer to play at the relay below",
			);
			const states = (await report(second)).playing;
			if (
				states.indexOf(`moved ${sentTo}`) === -1 ||
				states.indexOf("playing 1") < states.indexOf(`moved ${sentTo}`)
			) {
				throw new Error(
					`the second viewer did not say it moved: ${states.join(", ")}`,
				);
			}
			summary = `the relay above, full with the publisher and the relay below, sent the second viewer to ${sentTo}, and it played there (${states.join(", ")})`;
		} else {
			await withTimeout(
				below.relay.waitForLines(
					/^SESSION_REFUSED session=\d+ too many sessions$/,
					1,
				),
				"the relay below to close the session that was sent to it",
			);
			const again = await context.relay
				.waitForLines(/^SESSION_SENT_AWAY /, 2, WATCHED_MS)
				.then(
					() => true,
					() => false,
				);
			if (again) {
				throw new Error("the relay above sent a second session away");
			}
			if (below.relay.hasPrinted(/^SESSION_SENT_AWAY /)) {
				throw new Error(
					"the relay below sent the session that was sent to it on",
				);
			}
			const now = await report(second);
			if (now.shown.length > 0) {
				throw new Error(`the second viewer drew ${now.shown.length} frames`);
			}
			summary = `both relays were full: the relay above sent the second viewer away once, the relay below closed it and sent nothing on, and in ${WATCHED_MS} ms nothing more was sent away (${now.playing.join(", ")})`;
		}
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ whenFull: options.case })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED when full: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
