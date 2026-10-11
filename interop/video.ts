// A page plays a LOC video track: the Objects of a subscription go through
// the receiver of `@xmoq/media` into the H.264 decoder of the browser.
//
// Usage: bun video.ts --video from-mlmpub [--port N] [--verbose]
//        bun video.ts --video page-to-page --through own|moqtail
//                     --mapping stream-per-group|stream-per-object
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
//
//   page-to-page  A page encodes frames it paints, 320x180 at 25 frames a
//                 second with a key frame every second, and publishes them
//                 with `createVideoTrackEncoder` of `@xmoq/media`, with a
//                 catalog that names the track; another page reads the
//                 catalog and plays the track with `playVideoTrack`, through
//                 a relay. Frame `n` shows `n` as 8 bars, black or white,
//                 and has the time `5,000,000 + n * 40,000` microseconds as
//                 the encoder sees it. The viewer comes when the publisher
//                 has painted 30 frames or more, so in the middle of a
//                 Group. Written before the first run:
//
//     - The viewer finds the track in the catalog with the codec
//       `avc3.42E01F`, 320x180, 25 frames a second, and no `timescale`,
//       and configures its decoder with that codec. **Through the own
//       relay only** (found at the first run through the moqtail relay):
//       that relay answers the viewer's Joining FETCH for the catalog for
//       Group 0, where the catalog is in the Group of its time, so the
//       viewer gets no catalog. There the viewer is told the codec by the
//       driver.
//     - **At least 50 frames are decoded**, each 320x180.
//     - **Every frame shows a number, and each the one after the number of
//       the frame before it** (modulo 256): no frame is missing, twice, out
//       of order, or another picture than was painted. No bar is unclear.
//     - **Each frame is 40,000 microseconds after the one before it.**
//     - **The time of a frame is wall-clock time** (the Timestamp of LOC):
//       the first frame is within a minute of the driver's clock, and not
//       near 5,000,000.
//     - **The decoder did not fail, and the viewer dropped nothing**, not
//       as `no-key-frame` either: the relay subscribes to the publisher
//       when the viewer comes, the publisher asks its encoder for a key
//       frame for that subscriber, and the first Object the relay gets is
//       Object 0 of a Group.
//     - The publisher: **one key frame was asked for** (for the subscriber
//       that came; no Group was cut), the encoder did not fail, no write
//       failed, and nothing was left unsent but as `no-subscriber`, before
//       the relay came. At least 3 key chunks: the first, the one asked
//       for, and one a second later.
//     - **A frame whose time is not after the one before it is not
//       encoded**: the page hands in one such, with the picture of frame 45
//       and the time of frame 44, while the viewer plays. The publisher
//       counts one `time-not-after`, and the viewer's numbers and times
//       have no frame twice.
//     - **Corrected after the first run**: "every frame that was painted
//       was handed to the encoder" did not hold. 9 of 85 were not, as
//       `encoder-behind`: more than 2 frames waited in the encoder. The
//       viewer's numbers had no gap, so it was before the viewer came;
//       when exactly is not looked at. The run takes frames that were not
//       encoded for that reason, and says how many.
//     - **Corrected after a run on Linux in CI: "the viewer dropped
//       nothing" was the machine's speed, and no part of what is to be
//       shown.** There the decoder of the viewer was behind once, with the
//       noise through the moqtail relay: `playVideoTrack` gave up the rest
//       of the Group, as it is to (`decoder-behind` 1, `late` 13), and the
//       numbers went from 47 to 62. The publisher had left nothing unsent
//       and no write had failed. The same shape came here with a decoder
//       that may have 1 chunk waiting. So what is looked at is this:
//       - **Every Object the publisher wrote reached the viewer.** The
//         publisher ends the track, the viewer's subscription ends with
//         it, and then the Objects the publisher wrote are as many as the
//         viewer gave its decoder or dropped with a reason.
//       - The viewer dropped Objects for a decoder that was behind
//         (`decoder-behind`, `late`) and for nothing else, and its decoder
//         gave a frame for every chunk it was given.
//       - **Every frame shows a later number than the frame before it, and
//         is that many times 40,000 microseconds after it**; no more
//         numbers are passed over than Objects were dropped.
//       - At least 50 frames were decoded when the viewer first reported.
//
//   page-to-page --picture noise
//                 The same, with a picture an encoder cannot make small:
//                 1280x720, the lower half random pixels that change with
//                 every frame, at 30,000,000 bits a second, and a viewer
//                 that takes Objects of 1 MiB at most
//                 (`limits.maxObjectPayloadLength`). Written before the
//                 first run:
//
//     - Everything of the run with bars holds: 50 frames in a row by their
//       numbers, nothing dropped by the viewer, no failure of a decoder or
//       an encoder, one key frame asked for, nothing unsent but as
//       `no-subscriber`. So **no chunk is over the 1,000,000 bytes the
//       sender sends at most** (`too-large`), **the relay passes every
//       Object** (it holds 1 MiB of a stream it has not decoded), and
//       **nothing waits for the relay so long that a Group is cut**
//       (`behind`: 32 Objects or 4 MiB).
//     - **A Group is more than 2 MiB**, counted as 25 times the mean
//       chunk, and the largest chunk is 200,000 bytes or more: with a
//       Group on one stream, that stream carries more than the relay holds
//       undecoded of one stream, and more than the 1 MiB of one Object.
//     - How many frames the encoder was behind for is said, and not
//       judged: whether 2 frames are too few to let wait is what this run
//       is to show.
//
//   key-frames-too-large
//                 A page encodes the bars as in page-to-page, through the
//                 own relay, and sends payloads of 100 bytes at most: every
//                 key chunk is longer (256 bytes and more, in the runs with
//                 bars). Nobody views. Written before the first run:
//
//     - **The page is told** (`onKeyChunksNotSent` of
//       `createVideoTrackEncoder`): once, when the third key chunk in a row
//       was not sent, with the reason `too-large` and the length of that
//       chunk, over 100.
//     - **Once and no more**: when two more key chunks have come, five in
//       all, it has still been told once.
//     - The encoder goes on and has not failed; no Object was written; as
//       many chunks were left unsent as `too-large` as key chunks came.
//
//   to-mlmsub     A page encodes the bars as in page-to-page, a Group on a
//                 stream, through the own relay, and `mlmsub` of moqlivemock
//                 v0.16.1 takes the track (`-videoname video -loglevel
//                 debug`). Read in its source before the
//                 first run (`internal/sub/sub.go`, `internal/sub/loc.go` at
//                 the commit `scripts/ci/interop.sh` builds): it reads the
//                 catalog, takes the first video track whose name has the
//                 text of `-videoname`, subscribes, and prints "group
//                 start" for every Object 0 and "object" for every other
//                 Object, with the Group, the Subgroup, the length of the
//                 payload and the Timestamp of LOC. Written before the
//                 first run:
//
//     - `mlmsub` selects the track `video`.
//     - **Three Groups begin, one after the other**: their Group IDs go up
//       by 1, each in Subgroup 0, and their Timestamps are 1,000,000
//       microseconds apart and within a minute of the clock.
//     - **Between the first two, Objects 1 to 24 of the first Group**, in
//       that order, each 40,000 microseconds after the one before it.
//     - **Not in the run: `mlmsub` cannot write this video out.** With
//       `-videoout` it takes a payload to have a length of 4 bytes before
//       every NAL unit (`LOCVideoWriter`); the payloads here begin with a
//       start code, `00 00 00 01`, which it reads as a length of 1, and
//       then four bytes of the parameter set as the next length. The first
//       run was made with `-videoout` and expected that failure on every
//       Object, and the three Groups all the same. **That was wrong, for a
//       line of the source that was not read**: after "failed to write LOC
//       sample" (`NALU length 1107299721 exceeds remaining data 237`, as
//       worked out) `mlmsub` reads the track no further. So the run is
//       without `-videoout`, and the failure is in the record.
//
import { parseArgs } from "node:util";
import {
	captureGroup,
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnMlmsub,
	startMoqlivemockPublisher,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageVideoPublishReport,
	PageVideoReport,
	PageVideoSetup,
} from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		video: { type: "string", default: "from-mlmpub" },
		port: { type: "string", default: "24390" },
		// The matrix gives every driver a relay; the cases here choose theirs.
		relay: { type: "string", default: "" },
		// page-to-page: the relay, and how the Objects go onto streams.
		through: { type: "string", default: "own" },
		mapping: { type: "string", default: "stream-per-group" },
		// page-to-page: "bars", or "noise" for frames an encoder cannot make small.
		picture: { type: "string", default: "bars" },
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

/** The namespace an own page publishes its video under. */
const PAGE_NAMESPACE = "xmoq/video";
const BARS = {
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
};
/** 1280x720 with noise in the lower half, at a bit rate that makes a Group of several MiB. Level 5.1 of H.264, which allows it. */
const NOISE = {
	...BARS,
	codec: "avc1.640033",
	catalogCodec: "avc3.640033",
	width: 1280,
	height: 720,
	bitrate: 30_000_000,
	noise: true,
};
const PAGE_VIDEO = options.picture === "noise" ? NOISE : BARS;
/** What a viewer of the noise takes of one Object at most: what the README of `@xmoq/media` asks of a viewer. */
const VIEWER_MAX_OBJECT = 1024 * 1024;
/** Frames the publisher has painted before the viewer comes: more than a Group. */
const PAINTED_BEFORE = 30;

/** Has the viewer read the catalog of the publishing page, and gives the codec it has for the track. */
async function codecOfTheCatalog(
	viewer: Awaited<ReturnType<typeof openConnectedPage>>,
): Promise<string> {
	const catalog = await withTimeout(
		viewer.evaluate(
			async ([namespace]) => {
				await window.xmoqOwn.followCatalog(namespace);
				return window.xmoqOwn.catalogReport(1);
			},
			[PAGE_NAMESPACE] satisfies [string],
		),
		"the viewer to read the catalog",
	);
	const track = catalog.states
		.at(0)
		?.catalog?.tracks.find((each) => each.name === PAGE_VIDEO.trackName);
	if (
		track?.codec !== PAGE_VIDEO.catalogCodec ||
		track.width !== PAGE_VIDEO.width ||
		track.height !== PAGE_VIDEO.height ||
		track.framerate !== PAGE_VIDEO.framerate ||
		"timescale" in track
	) {
		throw new Error(
			`the catalog has no track ${PAGE_VIDEO.trackName} with ${PAGE_VIDEO.catalogCodec}, ${PAGE_VIDEO.width}x${PAGE_VIDEO.height}, ${PAGE_VIDEO.framerate} frames a second and no timescale: ${JSON.stringify(catalog)}`,
		);
	}
	return track.codec;
}

async function pageToPage(): Promise<string> {
	const mapping = (
		[
			"stream-per-group",
			"stream-per-object",
		] satisfies PageVideoSetup["mapping"][]
	).find((each) => each === options.mapping);
	if (mapping === undefined) {
		throw new Error(`--mapping "${options.mapping}" is none`);
	}
	const context = await startRun({
		relay: options.through,
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(
			([namespace, setup]) => window.xmoqOwn.publishVideo(namespace, setup),
			[PAGE_NAMESPACE, { ...PAGE_VIDEO, mapping }] satisfies [
				string,
				PageVideoSetup,
			],
		),
		"the page to publish its video",
	);
	const published = (): Promise<PageVideoPublishReport> =>
		publisher.evaluate(() => window.xmoqOwn.videoPublishReport());
	await pollFor(
		async () =>
			(await published()).painted >= PAINTED_BEFORE ? true : undefined,
		`the publisher to paint ${PAINTED_BEFORE} frames`,
	);

	const viewer = await openConnectedPage(
		context,
		"viewer",
		"own-browser",
		PAGE_VIDEO.noise ? VIEWER_MAX_OBJECT : undefined,
	);
	const codec =
		options.through === "own"
			? await codecOfTheCatalog(viewer)
			: PAGE_VIDEO.catalogCodec;
	const before = Date.now() * 1000;
	const report = await withTimeout(
		viewer.evaluate(
			async ([namespace, name, codec, frames, timeoutMs]) => {
				await window.xmoqOwn.playVideo(namespace, name, codec, true);
				return window.xmoqOwn.videoReport(frames, timeoutMs);
			},
			[
				PAGE_NAMESPACE,
				PAGE_VIDEO.trackName,
				codec,
				FRAMES,
				PLAY_TIMEOUT_MS,
			] satisfies [string, string, string, number, number],
		),
		`the viewer to report on its playing, which it does after ${PLAY_TIMEOUT_MS} ms at the latest`,
		PLAY_TIMEOUT_MS + 5_000,
	);
	// The track is ended, and the viewer says what it has when its
	// subscription has ended: nothing is on its way any more, so what the
	// publisher wrote and what the viewer got can be counted against each
	// other.
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopVideo()),
		"the publisher to end its track",
	);
	const final = await withTimeout(
		viewer.evaluate(
			([timeoutMs]) =>
				window.xmoqOwn.videoReport(Number.MAX_SAFE_INTEGER, timeoutMs),
			[PLAY_TIMEOUT_MS] satisfies [number],
		),
		"the viewer to report when its subscription has ended",
		PLAY_TIMEOUT_MS + 5_000,
	);
	const sent = await published();
	const seen = JSON.stringify({
		...final,
		frames: final.frames.slice(0, 3),
		whenItHadEnough: { frames: report.frames.length, stats: report.stats },
		publisher: sent,
	});
	log("driver", `REPORT ${seen}`);

	const { frames, stats } = final;
	if (report.frames.length < FRAMES) {
		const tooLarge = sent.notSent["too-large"] ?? 0;
		// The cause where it is this one: the counts alone do not say it.
		const cause =
			tooLarge > 0
				? `; **the publisher sent ${sent.written} Objects: ${tooLarge} chunks were over the longest payload it sends**, the largest ${sent.largestChunk} bytes (another encoder than the one the picture was sized for?)`
				: "";
		throw new Error(
			`${report.frames.length} frames were decoded, not ${FRAMES}, when the viewer reported (after ${PLAY_TIMEOUT_MS} ms at the latest); the playing ${report.ended === null ? "went on" : `ended as ${report.ended}`}, the decoder failed ${report.stats.decoderFailures} times (${report.stats.lastDecoderError})${cause}: ${seen}`,
		);
	}
	if (final.ended?.startsWith("subscription-ended ") !== true) {
		throw new Error(
			`the playing was to end with its subscription when the publisher ended the track; it ${final.ended === null ? "went on" : `ended as ${final.ended}`}: ${seen}`,
		);
	}
	// Every Object the viewer got was given to the decoder or dropped with a
	// reason.
	const dropped = Object.values(stats.dropped).reduce(
		(sum, count) => sum + count,
		0,
	);
	if (stats.submitted + dropped !== sent.written) {
		throw new Error(
			`the publisher wrote ${sent.written} Objects and the viewer got ${stats.submitted + dropped} (${stats.submitted} to its decoder, ${dropped} dropped): ${seen}`,
		);
	}
	if (stats.decoderFailures !== 0 || stats.decoded !== stats.submitted) {
		throw new Error(
			`the decoder failed ${stats.decoderFailures} times (${stats.lastDecoderError}), and gave ${stats.decoded} frames for the ${stats.submitted} it was given: ${seen}`,
		);
	}
	// WHY these two are let through: a decoder that is slower than the
	// track loses the rest of a Group (`decoder-behind` for what was there,
	// `late` for what came after), which is the machine's speed and not the
	// transport's doing. Anything else is.
	const others = Object.keys(stats.dropped).filter(
		(reason) => reason !== "decoder-behind" && reason !== "late",
	);
	if (others.length > 0) {
		throw new Error(
			`the viewer dropped Objects as ${others.join(", ")}: ${JSON.stringify(stats.dropped)}: ${seen}`,
		);
	}
	let missing = 0;
	frames.forEach((frame, index) => {
		const what = `frame ${index} (${JSON.stringify(frame)})`;
		if (
			frame.width !== PAGE_VIDEO.width ||
			frame.height !== PAGE_VIDEO.height
		) {
			throw new Error(
				`${what} is not ${PAGE_VIDEO.width}x${PAGE_VIDEO.height}: ${seen}`,
			);
		}
		if (typeof frame.number !== "number") {
			throw new Error(`${what} shows no number that can be read: ${seen}`);
		}
		const prior = frames[index - 1];
		if (prior === undefined) return;
		// How many frames later the picture says it is, and the time.
		const later =
			typeof prior.number === "number"
				? (frame.number - prior.number + 256) % 256
				: 0;
		if (later < 1 || frame.timestamp - prior.timestamp !== later * 40_000) {
			throw new Error(
				`${what} is to show a later number than the frame before it, ${prior.number}, and be 40000 microseconds after it for each; it is ${frame.timestamp - prior.timestamp} after it: ${seen}`,
			);
		}
		missing += later - 1;
	});
	// A number that is passed over is an Object the viewer dropped.
	if (missing > dropped) {
		throw new Error(
			`${missing} numbers are passed over between the frames, and the viewer dropped ${dropped} Objects: ${seen}`,
		);
	}
	const [first] = frames;
	if (first === undefined || Math.abs(first.timestamp - before) > 60_000_000) {
		throw new Error(
			`the first frame has the time ${first?.timestamp}, which is not within a minute of the clock (${before}): ${seen}`,
		);
	}
	if (sent.notEncoded["time-not-after"] !== 1) {
		throw new Error(
			`the one frame the page handed in with the time of the frame before it was to be left unencoded as time-not-after: ${JSON.stringify(sent)}`,
		);
	}
	const unsent = Object.keys(sent.notSent).filter(
		(reason) => reason !== "no-subscriber",
	);
	if (
		sent.encoderError !== null ||
		sent.keyFramesAskedFor !== 1 ||
		sent.submitted + (sent.notEncoded["encoder-behind"] ?? 0) !==
			sent.painted ||
		unsent.length > 0 ||
		sent.writeFailures !== 0 ||
		sent.keyChunks < 3
	) {
		throw new Error(
			`the publisher was to be asked for one key frame, encode every frame it painted but the ones its encoder was behind for, make 3 key chunks or more, and leave nothing unsent but for want of a subscriber: ${JSON.stringify(sent)}`,
		);
	}
	if (PAGE_VIDEO.noise) {
		// 25 frames are a second, and a Group.
		const aGroup = Math.round((sent.encodedBytes / sent.encoded) * 25);
		if (aGroup <= 2 * 1024 * 1024 || sent.largestChunk < 200_000) {
			throw new Error(
				`the noise was to make a Group of more than 2 MiB and a chunk of 200,000 bytes or more; a Group was ${aGroup} bytes and the largest chunk ${sent.largestChunk}: ${JSON.stringify(sent)}`,
			);
		}
	}
	return `${sent.written} Objects of a page's video reached another through the ${options.through} relay (${mapping}), which decoded ${frames.length} frames, ${PAGE_VIDEO.width}x${PAGE_VIDEO.height}, each showing a later number than the one before it and that many times 40 ms after it, from ${first.number} on, and dropped ${dropped} with a decoder that was behind (${missing} numbers passed over); the largest chunk was ${sent.largestChunk} bytes and 25 chunks ${Math.round((sent.encodedBytes / sent.encoded) * 25)} on average, ${sent.keyChunks} key chunks of ${sent.encoded}, and ${sent.notEncoded["encoder-behind"] ?? 0} of ${sent.painted} frames were not encoded, the encoder being behind`;
}

async function keyFramesTooLarge(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(
			([namespace, setup]) => window.xmoqOwn.publishVideo(namespace, setup),
			[
				PAGE_NAMESPACE,
				{ ...BARS, mapping: "stream-per-group", maxPayloadLength: 100 },
			] satisfies [string, PageVideoSetup],
		),
		"the page to publish its video",
	);
	// Five key chunks: two more than it takes to be told.
	const sent = await pollFor(async () => {
		const report = await publisher.evaluate(() =>
			window.xmoqOwn.videoPublishReport(),
		);
		return report.keyChunks >= 5 ? report : undefined;
	}, "the publisher to encode five key chunks");
	const seen = JSON.stringify(sent);
	log("driver", `REPORT ${seen}`);
	const [told, ...again] = sent.keyChunksNotSent;
	if (
		told === undefined ||
		again.length > 0 ||
		told.because !== "too-large" ||
		told.inARow !== 3 ||
		told.length <= 100
	) {
		throw new Error(
			`the page was to be told once, at the third key chunk over 100 bytes in a row, as too-large: ${seen}`,
		);
	}
	if (
		sent.encoderError !== null ||
		sent.written !== 0 ||
		sent.notSent["too-large"] !== sent.keyChunks
	) {
		throw new Error(
			`the encoder was to go on, no Object to be written, and every key chunk to be left unsent as too-large: ${seen}`,
		);
	}
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopVideo()),
		"the publisher to end its track",
	);
	return `a page whose key chunks are all over what it sends was told so once, at the third in a row (${told.length} bytes, over 100), and went on encoding: ${sent.keyChunks} key chunks, none sent`;
}

async function toMlmsub(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(
			([namespace, setup]) => window.xmoqOwn.publishVideo(namespace, setup),
			[PAGE_NAMESPACE, { ...BARS, mapping: "stream-per-group" }] satisfies [
				string,
				PageVideoSetup,
			],
		),
		"the page to publish its video",
	);
	await pollFor(
		async () =>
			(await publisher.evaluate(() => window.xmoqOwn.videoPublishReport()))
				.painted >= PAINTED_BEFORE
				? true
				: undefined,
		`the publisher to paint ${PAINTED_BEFORE} frames`,
	);
	const before = Date.now() * 1000;
	const { process: mlmsub } = spawnMlmsub(context, PAGE_NAMESPACE, [
		"-videoname",
		BARS.trackName,
		"-loglevel",
		"debug",
	]);
	await mlmsub.waitForLine(
		new RegExp(`selected video track.*trackName=${BARS.trackName}\\b`),
	);
	const START =
		/msg="group start" track=video groupID=(\d+) subGroupID=(\d+) payloadLength=(\d+) locTimestampUs=(\d+)/;
	// The third Group has begun: the first two are whole.
	await pollFor(
		async () => (mlmsub.matches(START).length >= 3 ? true : undefined),
		"mlmsub to see three Groups begin",
	);
	const groups = mlmsub.matches(START).slice(0, 3);
	const starts = groups.map((line) => ({
		group: BigInt(captureGroup(line, 1)),
		subgroup: captureGroup(line, 2),
		time: Number(captureGroup(line, 4)),
	}));
	const seen = groups.map((line) => line[0]).join("; ");
	log("driver", `REPORT ${seen}`);
	const [first, second, third] = starts;
	if (
		first === undefined ||
		second === undefined ||
		third === undefined ||
		second.group !== first.group + 1n ||
		third.group !== first.group + 2n ||
		starts.some((start) => start.subgroup !== "0") ||
		second.time - first.time !== 1_000_000 ||
		third.time - second.time !== 1_000_000 ||
		Math.abs(first.time - before) > 60_000_000
	) {
		throw new Error(
			`three Groups were to begin one after the other, in Subgroup 0, a second apart and at the time of the clock (${before}): ${seen}`,
		);
	}
	const objects = mlmsub
		.matches(
			/msg=object track=video objectID=(\d+) groupID=(\d+) subGroupID=0 payloadLength=\d+ locTimestampUs=(\d+)/,
		)
		.filter((line) => BigInt(captureGroup(line, 2)) === first.group)
		.map((line) => ({
			object: Number(captureGroup(line, 1)),
			time: Number(captureGroup(line, 3)),
		}));
	const wrong = objects.findIndex(
		({ object, time }, index) =>
			object !== index + 1 || time !== first.time + (index + 1) * 40_000,
	);
	if (objects.length !== 24 || wrong !== -1) {
		throw new Error(
			`mlmsub was to have Objects 1 to 24 of Group ${first.group}, each 40000 microseconds after the one before it; it had ${objects.length}, and the first that is not so is ${JSON.stringify(objects[wrong])}`,
		);
	}
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopVideo()),
		"the publisher to end its track",
	);
	return `mlmsub took the video of a page through the own relay: Groups ${first.group} to ${third.group} began a second apart, Objects 1 to 24 of the first 40 ms apart, with wall-clock Timestamps`;
}

const CASES: Record<string, () => Promise<string>> = {
	"to-mlmsub": toMlmsub,
	"from-mlmpub": fromMlmpub,
	"key-frames-too-large": keyFramesTooLarge,
	"page-to-page": pageToPage,
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
