// A page plays a LOC video track: the Objects of a subscription go through
// the receiver of `@xmoq/media` into the H.264 decoder of the browser.
//
// Usage: bun video.ts --video from-mlmpub [--port N] [--verbose]
//
// What has to be seen, written before the first run.
//
//   from-mlmpub   `mlmpub`, the publisher of moqlivemock, is a server: the
//                 page connects to it, with no relay. It publishes video at
//                 25 frames a second, a Group a second, a Group on one
//                 stream, the payloads with a length before every NAL unit
//                 (fixtures/loc-04/README.md).
//
//     - The page reads the catalog and finds the track `video_400kbps_avc`
//       in it, with the codec `avc3.4D401F`, 1280x720, 25 frames a second.
//       The codec the decoder is configured with is the catalog's.
//     - **At least 50 frames are decoded**, which is two Groups.
//     - **The first frame is the first of a Group**: its time is a whole
//       number of seconds (the Timestamp of Object 0 of Group G is G
//       seconds in the capture), since the page comes in the middle of a
//       Group and has to wait for a key frame.
//     - **Every frame is 1280x720, and each is 40,000 microseconds after
//       the one before it.** A frame that is missing, twice or out of
//       order shows here.
//     - **The browser has an H.264 decoder** (`VideoDecoder.isConfigSupported`
//       for the catalog's codec). Where it has none, the run says so, and
//       does not go on to say that no frame came.
//     - **The decoder did not fail**, and no more frames came out of it than
//       were given to it. How many are still inside it is not looked at: a
//       decoder holds some, and how many is the decoder's.
//     - **Nothing was dropped but what came before the first key frame**
//       (`no-key-frame`), of which there can be none or up to a Group.
//       No `gap`, no `unusable`, no `late`: `mlmpub` sends in order, on
//       loopback, and its Objects are LOC's.

import { parseArgs } from "node:util";
import {
	log,
	openConnectedPage,
	runCleanups,
	startMoqlivemockPublisher,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, PageVideoReport } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		video: { type: "string", default: "from-mlmpub" },
		port: { type: "string", default: "24390" },
		// The matrix gives every driver a relay; the case here has none.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The namespace of `mlmpub`'s LOC tracks and their MSF catalog. */
const MLMPUB_NAMESPACE = "mlm/msf/clear";
const TRACK = "video_400kbps_avc";
/** Two Groups of `mlmpub`. */
const FRAMES = 50;
/** Longer than the 50 frames take, 2 seconds, and the wait for a key frame, 1 second at most. The page reports what it has when this is over. */
const PLAY_TIMEOUT_MS = 10_000;

/** Throws unless the report is what the top of this file says. */
function check(report: PageVideoReport, seen: string): void {
	const { frames, stats } = report;
	if (frames.length < FRAMES) {
		throw new Error(
			`${frames.length} frames were decoded, not ${FRAMES}, when the page reported (after ${PLAY_TIMEOUT_MS} ms at the latest); the playing ${report.ended === null ? "goes on" : `ended as ${report.ended}`}, the decoder failed ${stats.decoderFailures} times (${stats.lastDecoderError}): ${seen}`,
		);
	}
	const [first] = frames;
	if (first === undefined || first.timestamp % 1_000_000 !== 0) {
		throw new Error(
			`the first frame is at ${first?.timestamp}, which is not the start of a Group: ${seen}`,
		);
	}
	frames.forEach((frame, index) => {
		if (frame.width !== 1280 || frame.height !== 720) {
			throw new Error(
				`frame ${index} is ${frame.width}x${frame.height}, not 1280x720: ${seen}`,
			);
		}
		const before = frames[index - 1];
		if (before !== undefined && frame.timestamp - before.timestamp !== 40_000) {
			throw new Error(
				`frame ${index} is ${frame.timestamp - before.timestamp} microseconds after the one before it, not 40000: ${seen}`,
			);
		}
	});
	if (stats.decoderFailures !== 0) {
		throw new Error(
			`the decoder failed ${stats.decoderFailures} times (${stats.lastDecoderError}): ${seen}`,
		);
	}
	if (stats.submitted < stats.decoded) {
		throw new Error(
			`${stats.submitted} frames were given to the decoder and ${stats.decoded} came out: ${seen}`,
		);
	}
	const reasons = Object.keys(stats.dropped).filter(
		(reason) => reason !== "no-key-frame",
	);
	if (reasons.length > 0) {
		throw new Error(`Objects were dropped as ${reasons.join(", ")}: ${seen}`);
	}
}

async function fromMlmpub(): Promise<string> {
	const context = await startMoqlivemockPublisher({
		port: options.port,
		verbose: options.verbose,
	});
	const page = await openConnectedPage(context, "viewer", "own-browser");
	const catalog = await withTimeout(
		page.evaluate(
			async ([namespace]) => {
				await window.xmoqOwn.followCatalog(namespace);
				return window.xmoqOwn.catalogReport(1);
			},
			[MLMPUB_NAMESPACE] satisfies [string],
		),
		"the page to read the catalog",
	);
	const track = catalog.states
		.at(0)
		?.catalog?.tracks.find((each) => each.name === TRACK);
	if (
		track?.codec !== "avc3.4D401F" ||
		track.width !== 1280 ||
		track.height !== 720 ||
		track.framerate !== 25
	) {
		throw new Error(
			`the catalog has no track ${TRACK} with avc3.4D401F, 1280x720 and 25 frames a second: ${JSON.stringify(track)}`,
		);
	}
	const support = await withTimeout(
		page.evaluate(([codec]) => window.xmoqOwn.videoDecoderSupport(codec), [
			track.codec,
		] satisfies [string]),
		"the page to say whether it has a decoder",
	);
	log("driver", `DECODER ${track.codec}: ${support}`);
	if (support !== "supported") {
		throw new Error(
			`this browser has no decoder for ${track.codec} (the page said: ${support}); the run needs H.264`,
		);
	}
	const report = await withTimeout(
		page.evaluate(
			async ([namespace, name, codec, frames, timeoutMs]) => {
				await window.xmoqOwn.playVideo(namespace, name, codec);
				return window.xmoqOwn.videoReport(frames, timeoutMs);
			},
			[
				MLMPUB_NAMESPACE,
				TRACK,
				track.codec,
				FRAMES,
				PLAY_TIMEOUT_MS,
			] satisfies [string, string, string, number, number],
		),
		`the page to report on its playing, which it does after ${PLAY_TIMEOUT_MS} ms at the latest`,
		PLAY_TIMEOUT_MS + 5_000,
	);
	const seen = JSON.stringify({
		...report,
		frames: report.frames.slice(0, 3),
	});
	log("driver", `REPORT ${seen}`);
	check(report, seen);
	const dropped = report.stats.dropped["no-key-frame"] ?? 0;
	return `${report.frames.length} frames of ${TRACK} decoded, 1280x720 and 40 ms apart, from the first frame of a Group on; ${dropped} Objects before it were not played`;
}

const CASES: Record<string, () => Promise<string>> = {
	"from-mlmpub": fromMlmpub,
};

async function main(): Promise<void> {
	const run = CASES[options.video];
	if (run === undefined) {
		throw new Error(
			`--video "${options.video}" is not one of ${Object.keys(CASES).join(", ")}`,
		);
	}
	let summary: string;
	try {
		summary = await run();
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ video: options.video })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.video} video: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
