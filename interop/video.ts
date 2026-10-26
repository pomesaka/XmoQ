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
//       the one before it**, but where the rest of a Group was dropped for
//       a decoder that was behind (below): the frame after it is then the
//       first of a later Group. A frame that is missing otherwise, twice or
//       out of order shows here.
//     - **The browser has an H.264 decoder** (`VideoDecoder.isConfigSupported`
//       for the catalog's codec). Where it has none, the run says so, and
//       does not go on to say that no frame came.
//     - **The decoder did not fail**, and no more frames came out of it than
//       were given to it. How many are still inside it is not looked at: a
//       decoder holds some, and how many is the decoder's.
//     - **Nothing was dropped but what came before the first key frame**
//       (`no-key-frame`), of which there can be none or up to a Group, and
//       the rest of a Group for a decoder that was behind (`decoder-behind`
//       for what was there, `late` for what came after), as many as the
//       frames passed over at most. No `gap`, no `unusable`: `mlmpub` sends
//       in order, on loopback, and its Objects are LOC's.
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
//   late-viewer   The bars through the own relay, a Group on a stream. One
//                 page watches, so the relay is subscribed; a second page
//                 comes later, in the middle of a Group, and plays with the
//                 Joining FETCH of `playVideoTrack`. Written before the
//                 first run:
//
//     - **The Joining FETCH is served** by the relay, from the latest Group
//       it keeps, with one Object or more.
//     - **The second viewer's first frame was made before it came**: its
//       time is before the moment the driver started that viewer. Without
//       the FETCH the first frame is the next key frame, made after.
//     - It dropped nothing as `no-key-frame`: only what came twice, on the
//       fetch stream and on the subscription (`late`), or for a decoder
//       that was behind.
//     - 30 frames or more, each with a later number than the one before it
//       and that many times 40,000 microseconds after it.
//
//   late-viewer-new-group
//                 The same, but the track has dynamic Groups (DYNAMIC_GROUPS,
//                 a Group 500 ms long at least) and a key frame every 10
//                 seconds by itself, and the second page sends no Joining
//                 FETCH: its SUBSCRIBE has NEW_GROUP_REQUEST 0 (transport
//                 §5.1.3.1). The relay's subscription to the publisher is
//                 Established, so the relay is to send the publisher a
//                 REQUEST_UPDATE on the stream of its own SUBSCRIBE
//                 (§10.2.13). Written before the first run:
//
//     - **The relay sends it**: a line `SEND_OWN_REQUEST ... REQUEST_UPDATE`.
//     - **The publisher takes it and begins a Group for it**: one request
//       taken, one key chunk asked for (`newGroupsAskedFor` 1,
//       `newGroupsBegun` 1).
//     - **The second viewer's first frame was made after it came, and came
//       within 1,000 ms of its SUBSCRIBE**. Without the request the next
//       key frame would come up to 8 seconds later (the viewer comes
//       about 1.6 seconds into a Group of 10).
//     - It dropped nothing but what came before that key frame
//       (`no-key-frame`) or for a decoder that was behind; 30 frames or
//       more, in order, as above.
//
//   audio         A page makes a 440 Hz tone, half of full scale, encodes it
//                 as Opus (48,000 Hz, mono) and publishes it with
//                 `createAudioTrackEncoder` of `@xmoq/media`, a Group and a
//                 stream for every chunk; another page plays it with
//                 `playAudioTrack` through the own relay. Nothing is
//                 sounded: what the decoder gives is looked at. Written
//                 before the first run:
//
//     - **100 pieces of audio or more are decoded**, 2 seconds, each
//       48,000 Hz and mono.
//     - **Each piece begins where the one before it ends**: its time is
//       the time of the one before plus that one's frames, in
//       microseconds. **Corrected after the first run**: the time of piece
//       13 was 1 microsecond before that, though the Objects carry whole
//       multiples of 20,000 after the first; the decoder counts the times
//       of what it gives by itself. The run takes a piece within 2
//       microseconds of 20,000 times its place after the first, and says
//       the furthest it saw.
//     - The time of the first piece is wall-clock time, within a minute of
//       the driver's clock.
//     - **It is not silence**: 9 of 10 pieces have a sample over 0.1 of
//       full scale.
//     - The viewer dropped nothing and its decoder did not fail; the
//       publisher's encoder did not fail and left nothing unsent but for
//       want of a subscriber.
//     - Whether the encoder gave a `description` is said, and not judged:
//       none is sent, and whether one is to be is undecided.
//
//   audio-beyond-time
//                 A page serves an audio track Object by Object
//                 (`serveTrack`, `sendObjectIn`), a Group each, with the
//                 Opus packet `f8 ff fe` (one 20 ms frame of CELT, of no
//                 sound); another plays it with `playAudioTrack` through the
//                 own relay. The first Object has the LOC Timestamp 2^60
//                 microseconds, which a decoder of WebCodecs does not take
//                 (`MAX_DECODER_TIME` of `@xmoq/media`); the 50 after it have
//                 the driver's clock, 20 ms apart, each sent once the viewer
//                 decoded the one before: each is a stream of its own, which
//                 can come before the one before it, and this run is not of
//                 the order. Written before the first
//                 run: **the viewer drops the first as `beyond-time`, decodes
//                 the 50 after it**, received 51, and its decoder does not
//                 fail. Before `MAX_DECODER_TIME` was looked at, the first
//                 Object threw as its chunk was made, and nothing after it
//                 was decoded.
//
//   goaway-from-moqtail, goaway-from-moqtail-viewer-first, goaway-from-moqtail-rust
//                 Through moqtail's relay (relay@0.15.0), started with
//                 `--redirect-uri` naming the own relay on the run's port
//                 plus `MOQLIVEMOCK_SIDE_PORT_OFFSET`. Once the clients
//                 play, moqtail's relay is sent SIGTERM: it sends every
//                 session a GOAWAY on the control stream with that URI, a
//                 Timeout of 10,000 ms and Request ID 0, refuses new
//                 requests with GOING_AWAY, and exits after 10 s
//                 (`apps/relay/src/server.rs`, `broadcast_goaway`). Written
//                 before the first run:
//
//     - `goaway-from-moqtail`: a page publishes the bars with
//       `keepPublishing` and another plays them with `keepPlaying`. **Both
//       move to the own relay** (states `moved https://127.0.0.1:<port>`,
//       then `publishing 2` and `playing 2`) **and the viewer shows 25
//       frames on each session**, in order, as in `reconnect`. Each closes
//       its session with moqtail's relay with NO_ERROR before (the own
//       relay is not told that; moqtail's log is not read).
//     - `goaway-from-moqtail-viewer-first`: the same, with the publishing
//       page waiting 3 s before it connects after the GOAWAY. The viewer
//       comes to the own relay first, is refused there (state `failed`
//       after `moved`), **tries the own relay again and not moqtail's**, and
//       plays once the publisher is there.
//     - `goaway-from-moqtail-rust`: own-rust subscribes to the bars of the
//       publishing page and **prints `GOAWAY uri=https://127.0.0.1:<port>
//       timeout_ms=10000 request_id=0` and leaves with 0**, connecting to
//       nothing else. The page publishes with `publishVideo`, on its one
//       session, which it does not leave at a GOAWAY. WHY NOT
//       `keepVideoPublished`: moqtail's relay sends the GOAWAY to one
//       session after another, and a page that got it first left at once,
//       so that the relay ended own-rust's subscription with PUBLISH_DONE
//       before it wrote own-rust's GOAWAY, and own-rust left without it (1
//       of the runs of CI on 2026-10-24).
//
//   goaway-from-own, goaway-from-own-rust
//                 The same two, through the own relay, started with
//                 `--redirect-uri` as moqtail's is. At SIGTERM it sends
//                 every session a GOAWAY on the control stream with that
//                 URI, a Timeout of 10,000 ms (`--goaway-timeout-ms`) and
//                 the smallest Request ID of the client it has not seen,
//                 refuses every request after it with GOING_AWAY and every
//                 new connection, and closes what is left with
//                 GOAWAY_TIMEOUT after the Timeout, or ends as soon as no
//                 session is left. Written before the first run: the pages
//                 move and show 25 frames on each session as through
//                 moqtail's; own-rust prints `GOAWAY uri=... timeout_ms=10000
//                 request_id=2` (its SUBSCRIBE was 0) and leaves with 0; and
//                 the relay prints `GOING_AWAY sessions=2` (the two
//                 clients) and ends with 0 before its Timeout is over, once
//                 they have left. The first run had `sessions=3`: every
//                 page `openConnectedPage` opens holds a session of its own
//                 besides the one that publishes or plays, and that session
//                 does not move. So the count is 3 with one page and 4 with
//                 two, and the relay ends with 0 after its Timeout, once it
//                 has closed that session with GOAWAY_TIMEOUT. With
//                 `goaway-from-own-rust` it is 2: its page publishes on that
//                 session (`goaway-from-moqtail-rust` has why).
//                 The terminal viewer is in `watch-rust.ts --watch goaway`.
//     - The terminal viewer is not run so: it reads the catalog with a
//       Joining FETCH, and moqtail's relay asks a page's publisher for
//       Group 0 there, where the page's Groups are numbered by the clock;
//       the FETCH is empty and no catalog comes (2026-10-24, the record of
//       that day on GOAWAY and moqtail).
//
import { parseArgs } from "node:util";
import { locProperties } from "@xmoq/loc";
import {
	captureGroup,
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	NAMESPACE,
	openConnectedPage,
	pollFor,
	type RunContext,
	runCleanups,
	STEP_TIMEOUT_MS,
	spawnMlmsub,
	spawnRustClient,
	startMoqlivemockPublisher,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageKeptReport,
	PageProperty,
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
		// page-to-page: the viewer sends SUBGROUP_DELIVERY_TIMEOUT and
		// OBJECT_DELIVERY_TIMEOUT of this many milliseconds with its SUBSCRIBE;
		// neither without it.
		"delivery-timeout-ms": { type: "string", default: "" },
		// page-to-page: once its SUBSCRIBE is accepted, the viewer asks for
		// this SUBGROUP_DELIVERY_TIMEOUT with a REQUEST_UPDATE (§10.9), which
		// the relay has to accept; no update without it.
		"update-delivery-timeout-ms": { type: "string", default: "" },
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

/** Throws unless the report is what the top of this file says; the frames passed over with the rest of a Group otherwise. */
function check(report: PageVideoReport, seen: string): number {
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
	// WHY the rest of a Group may be passed over: a decoder that is slower
	// than the track loses it (`abandonGroup` of the video receiver), which
	// is the machine's speed and not the transport's doing, as the run of a
	// page's video has it. Anything else that is missing is not.
	const behind =
		(stats.dropped["decoder-behind"] ?? 0) + (stats.dropped.late ?? 0);
	let passedOver = 0;
	frames.forEach((frame, index) => {
		if (frame.width !== 1280 || frame.height !== 720) {
			throw new Error(
				`frame ${index} is ${frame.width}x${frame.height}, not 1280x720: ${seen}`,
			);
		}
		const before = frames[index - 1];
		if (before === undefined) return;
		const after = frame.timestamp - before.timestamp;
		if (after === 40_000) return;
		if (
			after <= 0 ||
			after % 40_000 !== 0 ||
			frame.timestamp % 1_000_000 !== 0
		) {
			throw new Error(
				`frame ${index} is ${after} microseconds after the one before it, not 40000, nor the first of a later Group: ${seen}`,
			);
		}
		passedOver += after / 40_000 - 1;
	});
	if (passedOver > behind) {
		throw new Error(
			`${passedOver} frames were passed over, and ${behind} dropped for a decoder that was behind: ${seen}`,
		);
	}
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
		(reason) =>
			reason !== "no-key-frame" &&
			reason !== "decoder-behind" &&
			reason !== "late",
	);
	if (reasons.length > 0) {
		throw new Error(`Objects were dropped as ${reasons.join(", ")}: ${seen}`);
	}
	return passedOver;
}

async function fromMlmpub(): Promise<string> {
	const context = await startMoqlivemockPublisher({
		port: options.port,
		verbose: options.verbose,
	});
	const page = await openConnectedPage(context, "viewer", "own-browser");
	const catalog = await withTimeout(
		page.evaluate(
			async ([namespace, timeoutMs]) => {
				await window.xmoqOwn.followCatalog(namespace);
				return window.xmoqOwn.catalogReport(1, timeoutMs);
			},
			[MLMPUB_NAMESPACE, STEP_TIMEOUT_MS] satisfies [string, number],
		),
		"the page to read the catalog",
		// The page says what it had once its own time is over.
		2 * STEP_TIMEOUT_MS,
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
	const passedOver = check(report, seen);
	const dropped = report.stats.dropped["no-key-frame"] ?? 0;
	return `${report.frames.length} frames of ${TRACK} decoded, 1280x720 and 40 ms apart, from the first frame of a Group on, but for ${passedOver} passed over with the rest of a Group for a decoder that was behind; ${dropped} Objects before it were not played`;
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
			async ([namespace, timeoutMs]) => {
				await window.xmoqOwn.followCatalog(namespace);
				return window.xmoqOwn.catalogReport(1, timeoutMs);
			},
			[PAGE_NAMESPACE, STEP_TIMEOUT_MS] satisfies [string, number],
		),
		"the viewer to read the catalog",
		// The page says what it had once its own time is over.
		2 * STEP_TIMEOUT_MS,
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
			async ([
				namespace,
				name,
				codec,
				frames,
				timeoutMs,
				deliveryTimeoutMs,
				updatedMs,
			]) => {
				await window.xmoqOwn.playVideo(
					namespace,
					name,
					codec,
					true,
					false,
					deliveryTimeoutMs ?? undefined,
				);
				const updated =
					updatedMs === null
						? null
						: await window.xmoqOwn.updateVideoDeliveryTimeout(updatedMs);
				return {
					updated,
					...(await window.xmoqOwn.videoReport(frames, timeoutMs)),
				};
			},
			[
				PAGE_NAMESPACE,
				PAGE_VIDEO.trackName,
				codec,
				FRAMES,
				PLAY_TIMEOUT_MS,
				// The viewer's delivery timeouts, where the run asks for them.
				options["delivery-timeout-ms"] === ""
					? null
					: Number(options["delivery-timeout-ms"]),
				options["update-delivery-timeout-ms"] === ""
					? null
					: Number(options["update-delivery-timeout-ms"]),
			] satisfies [
				string,
				string,
				string,
				number,
				number,
				number | null,
				number | null,
			],
		),
		`the viewer to report on its playing, which it does after ${PLAY_TIMEOUT_MS} ms at the latest`,
		PLAY_TIMEOUT_MS + 5_000,
	);
	// §10.9: "The receiver of a REQUEST_UPDATE MUST respond with exactly one
	// REQUEST_OK or REQUEST_ERROR". A relay that refuses also ends the
	// subscription (§10.9.1), which the frames below would show as well.
	if (report.updated !== null && report.updated !== "accepted") {
		throw new Error(`the update of the delivery timeout was ${report.updated}`);
	}
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

async function lateViewer(): Promise<string> {
	return await viewerInTheMiddle("joining-fetch");
}

async function lateViewerNewGroup(): Promise<string> {
	return await viewerInTheMiddle("new-group");
}

/** `late-viewer` and `late-viewer-new-group`: how the second viewer comes to the start of a Group. */
async function viewerInTheMiddle(
	by: "joining-fetch" | "new-group",
): Promise<string> {
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
				{
					...BARS,
					mapping: "stream-per-group",
					...(by === "new-group"
						? { keyFrameIntervalMs: 10_000, shortestGroupMs: 500 }
						: {}),
				},
			] satisfies [string, PageVideoSetup],
		),
		"the page to publish its video",
	);
	const play = (
		page: Awaited<ReturnType<typeof openConnectedPage>>,
		joiningFetch: boolean,
		frames: number,
		newGroup = false,
	): Promise<PageVideoReport> =>
		withTimeout(
			page.evaluate(
				async ([namespace, name, codec, join, count, timeoutMs, ask]) => {
					await window.xmoqOwn.playVideo(
						namespace,
						name,
						codec,
						true,
						join,
						undefined,
						ask,
					);
					return window.xmoqOwn.videoReport(count, timeoutMs);
				},
				[
					PAGE_NAMESPACE,
					BARS.trackName,
					BARS.catalogCodec,
					joiningFetch,
					frames,
					PLAY_TIMEOUT_MS,
					newGroup,
				] satisfies [string, string, string, boolean, number, number, boolean],
			),
			"a viewer to report on its playing",
			PLAY_TIMEOUT_MS + 5_000,
		);
	// The first viewer has frames: the relay is subscribed, and has Objects.
	const first = await openConnectedPage(context, "viewer-1", "own-browser");
	await play(first, false, 40);
	const late = await openConnectedPage(context, "viewer-2", "own-browser");
	const before = Date.now() * 1000;
	const report = await play(
		late,
		by === "joining-fetch",
		30,
		by === "new-group",
	);
	const seen = JSON.stringify({ ...report, frames: report.frames.slice(0, 3) });
	log("driver", `REPORT ${seen}`);
	if (by === "new-group") {
		return await checkNewGroup(context, publisher, report, before, seen);
	}
	const { frames, stats } = report;
	const served = /^served (\d+)$/.exec(stats.joiningFetch);
	if (served === null || Number(served[1]) < 1) {
		throw new Error(
			`the Joining FETCH was to be served with one Object or more: ${seen}`,
		);
	}
	const [firstFrame] = frames;
	if (frames.length < 30 || firstFrame === undefined) {
		throw new Error(`${frames.length} frames were decoded, not 30: ${seen}`);
	}
	if (firstFrame.timestamp >= before) {
		throw new Error(
			`the first frame was to be one made before the viewer came (${before}); it has the time ${firstFrame.timestamp}: ${seen}`,
		);
	}
	checkDropped(report, ["late", "decoder-behind"], seen);
	checkInOrder(frames, seen);
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopVideo()),
		"the publisher to end its track",
	);
	return `a viewer that came in the middle of a Group fetched ${served[1]} Objects of it from the relay and had its first frame from ${Math.round((before - firstFrame.timestamp) / 1000)} ms before it came; ${frames.length} frames in order, dropped ${JSON.stringify(stats.dropped)}`;
}

/** The viewer dropped nothing for another reason than these, and its decoder did not fail. */
function checkDropped(
	report: PageVideoReport,
	reasons: string[],
	seen: string,
): void {
	const others = Object.keys(report.stats.dropped).filter(
		(reason) => !reasons.includes(reason),
	);
	if (others.length > 0 || report.stats.decoderFailures !== 0) {
		throw new Error(
			`the viewer was to drop nothing but as ${reasons.join(", ")}, and its decoder was not to fail: ${seen}`,
		);
	}
}

/** Every frame shows a later number than the one before it, and is 40,000 microseconds after it for each. */
function checkInOrder(frames: PageVideoReport["frames"], seen: string): void {
	frames.forEach((frame, index) => {
		const prior = frames[index - 1];
		if (prior === undefined) return;
		const later =
			typeof prior.number === "number" && typeof frame.number === "number"
				? (frame.number - prior.number + 256) % 256
				: 0;
		if (later < 1 || frame.timestamp - prior.timestamp !== later * 40_000) {
			throw new Error(
				`frame ${index} (${JSON.stringify(frame)}) is to show a later number than the frame before it, ${prior.number}, and be 40000 microseconds after it for each: ${seen}`,
			);
		}
	});
}

/** `late-viewer-new-group`, from the second viewer's report on. */
async function checkNewGroup(
	context: RunContext,
	publisher: Awaited<ReturnType<typeof openConnectedPage>>,
	report: PageVideoReport,
	before: number,
	seen: string,
): Promise<string> {
	const [sent] = await context.relay.waitForLine(
		/^SEND_OWN_REQUEST session=\d+ own=\d+ REQUEST_UPDATE/,
		5_000,
	);
	const sentBy = await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.videoPublishReport()),
		"the publisher to report",
	);
	log("driver", `PUBLISHER ${JSON.stringify(sentBy)}`);
	if (sentBy.newGroupsAskedFor !== 1 || sentBy.newGroupsBegun !== 1) {
		throw new Error(
			`the publisher was to take one NEW_GROUP_REQUEST and ask its encoder for one key frame for it: ${JSON.stringify(sentBy)}`,
		);
	}
	const { frames } = report;
	const [firstFrame] = frames;
	if (frames.length < 30 || firstFrame === undefined) {
		throw new Error(`${frames.length} frames were decoded, not 30: ${seen}`);
	}
	if (firstFrame.timestamp < before) {
		throw new Error(
			`the first frame was to be one made after the viewer came (${before}); it has the time ${firstFrame.timestamp}: ${seen}`,
		);
	}
	const after = report.firstFrameAfterMs;
	if (after === null || after > 1_000) {
		throw new Error(
			`the first frame was to come within 1000 ms of the SUBSCRIBE, not after ${after}: ${seen}`,
		);
	}
	checkDropped(report, ["no-key-frame", "decoder-behind"], seen);
	checkInOrder(frames, seen);
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopVideo()),
		"the publisher to end its track",
	);
	return `a viewer that came in the middle of a Group of 10 seconds asked for a new Group in its SUBSCRIBE; the relay passed it on (${sent}), the publisher began one, and the viewer's first frame came ${after} ms after it asked; ${frames.length} frames in order, dropped ${JSON.stringify(report.stats.dropped)}`;
}

async function audio(): Promise<string> {
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
			([namespace]) => window.xmoqOwn.publishAudio(namespace, "audio"),
			[PAGE_NAMESPACE] satisfies [string],
		),
		"the page to publish its audio",
	);
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	const before = Date.now() * 1000;
	const report = await withTimeout(
		viewer.evaluate(
			async ([namespace, timeoutMs]) => {
				// WHY the viewer holds a piece for the ones before it as long as
				// it waits for the report: a chunk is a Group and a stream of its
				// own, and nothing keeps the order of two streams. One that came
				// before the stream of the piece ahead of it went on at once, and
				// that piece was dropped as late. The demo's viewer holds a piece
				// too (`holdForOrderMs` of `createPlayout`). WHY a limit and not
				// none: a wait of the receiving side has one; a piece that never
				// comes leaves the pieces short, which fails the run.
				await window.xmoqOwn.playAudio(namespace, "audio", timeoutMs);
				return window.xmoqOwn.audioReport(100, timeoutMs);
			},
			[PAGE_NAMESPACE, PLAY_TIMEOUT_MS] satisfies [string, number],
		),
		"the viewer to report on its audio",
		PLAY_TIMEOUT_MS + 5_000,
	);
	const sent = await publisher.evaluate(() =>
		window.xmoqOwn.audioPublishReport(),
	);
	const seen = JSON.stringify({
		...report,
		pieces: report.pieces.slice(0, 4),
		publisher: sent,
	});
	log("driver", `REPORT ${seen}`);
	const { pieces } = report;
	if (pieces.length < 100) {
		throw new Error(`${pieces.length} pieces were decoded, not 100: ${seen}`);
	}
	/** The furthest a piece is from 20 ms times its place after the first, in microseconds. */
	let offBy = 0;
	pieces.forEach((piece, index) => {
		const what = `piece ${index} (${JSON.stringify(piece)})`;
		if (piece.sampleRate !== 48_000 || piece.channels !== 1) {
			throw new Error(`${what} is not 48000 Hz and mono: ${seen}`);
		}
		const start = pieces[0];
		if (start === undefined || piece.frames !== 960) {
			throw new Error(`${what} has not 960 frames, 20 ms: ${seen}`);
		}
		const expected = start.timestamp + index * 20_000;
		offBy = Math.max(offBy, Math.abs(piece.timestamp - expected));
		if (Math.abs(piece.timestamp - expected) > 2) {
			throw new Error(
				`${what} does not begin where the one before it ends, at ${expected}: ${seen}`,
			);
		}
	});
	const [first] = pieces;
	if (first === undefined || Math.abs(first.timestamp - before) > 60_000_000) {
		throw new Error(
			`the first piece has the time ${first?.timestamp}, which is not within a minute of the clock (${before}): ${seen}`,
		);
	}
	const loud = pieces.filter((piece) => piece.peak > 0.1).length;
	if (loud * 10 < pieces.length * 9) {
		throw new Error(
			`${loud} of ${pieces.length} pieces have a sample over 0.1: ${seen}`,
		);
	}
	const unsent = Object.keys(sent.skipped).filter(
		(reason) => reason !== "no-subscriber",
	);
	if (
		Object.keys(report.dropped).length > 0 ||
		report.decoderError !== null ||
		sent.encoderError !== null ||
		unsent.length > 0
	) {
		throw new Error(
			`the viewer was to drop nothing, no decoder or encoder to fail, and nothing to be left unsent but for want of a subscriber: ${seen}`,
		);
	}
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.stopAudio()),
		"the publisher to end its track",
	);
	const peak = Math.max(...pieces.map((piece) => piece.peak));
	return `${pieces.length} pieces of Opus audio of a page decoded by another through the own relay, ${first.frames} frames each at 48000 Hz, one after the other (within ${offBy} microseconds), the largest sample ${peak.toFixed(2)} of 1; ${Math.round(sent.encodedBytes / sent.encoded)} bytes a chunk; the encoder gave ${sent.descriptionLength === null ? "no description" : `a description of ${sent.descriptionLength} bytes, which is not sent`}`;
}

/** Objects of the audio track of `audioBeyondTime` after the first: a second of Opus. */
const AUDIO_AFTER_BEYOND = 50;
/** The time of its first: past 2^53 - 1 microseconds, where a LOC Timestamp still is. */
const BEYOND_DECODER_TIME = 2n ** 60n;
/**
 * One Opus frame of 20 ms, of no sound (RFC 6716 §3.1: TOC 0xF8 is CELT,
 * fullband, 20 ms, mono, one frame).
 */
const OPUS_SILENCE_HEX = "f8fffe";

async function audioBeyondTime(): Promise<string> {
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
			([namespace]) =>
				window.xmoqOwn.serveTrack(namespace, "audio", "refuse", ""),
			[PAGE_NAMESPACE] satisfies [string],
		),
		"the page to announce its namespace",
	);
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	await withTimeout(
		viewer.evaluate(
			([namespace]) => window.xmoqOwn.playAudio(namespace, "audio"),
			[PAGE_NAMESPACE] satisfies [string],
		),
		"the viewer to subscribe",
	);
	await withTimeout(
		publisher.evaluate(() => window.xmoqOwn.servingTrack()),
		"the publisher to be asked for the track",
	);
	/** A Group a piece, one Object each, as `createAudioTrackEncoder` sends. */
	const send = async (group: number, microseconds: bigint): Promise<void> => {
		const properties = locProperties({ timestamp: microseconds }).map(
			({ type, value }) => ({
				type: type.toString(),
				varint: value.toString(),
			}),
		);
		await withTimeout(
			publisher.evaluate(
				([g, hex, props]) =>
					window.xmoqOwn.sendObjectIn(g, hex, true, props, null),
				[String(group), OPUS_SILENCE_HEX, properties] satisfies [
					string,
					string,
					PageProperty[],
				],
			),
			`the publisher to send Group ${group}`,
		);
	};
	await send(0, BEYOND_DECODER_TIME);
	const now = BigInt(Date.now()) * 1000n;
	for (let piece = 1; piece <= AUDIO_AFTER_BEYOND; piece++) {
		await send(piece, now + BigInt(piece) * 20_000n);
		// The next goes once this one is decoded (the top of this file).
		await withTimeout(
			viewer.evaluate(
				([pieces, timeoutMs]) => window.xmoqOwn.audioReport(pieces, timeoutMs),
				[piece, PLAY_TIMEOUT_MS] satisfies [number, number],
			),
			`the viewer to decode piece ${piece}`,
			PLAY_TIMEOUT_MS + 5_000,
		);
	}
	const report = await withTimeout(
		viewer.evaluate(
			([pieces, timeoutMs]) => window.xmoqOwn.audioReport(pieces, timeoutMs),
			[AUDIO_AFTER_BEYOND, PLAY_TIMEOUT_MS] satisfies [number, number],
		),
		"the viewer to report on its audio",
		PLAY_TIMEOUT_MS + 5_000,
	);
	const seen = JSON.stringify({ ...report, pieces: report.pieces.length });
	log("driver", `REPORT ${seen}`);
	if (
		report.pieces.length !== AUDIO_AFTER_BEYOND ||
		report.received !== AUDIO_AFTER_BEYOND + 1 ||
		JSON.stringify(report.dropped) !== JSON.stringify({ "beyond-time": 1 }) ||
		report.decoderError !== null
	) {
		throw new Error(
			`the viewer was to drop the first Object as beyond-time, and decode the ${AUDIO_AFTER_BEYOND} after it, with no decoder error: ${seen}`,
		);
	}
	return `an Object whose time is 2^60 microseconds was dropped as beyond-time, and the ${AUDIO_AFTER_BEYOND} after it were decoded`;
}

/** Frames the viewer shows on a session before the run goes on: a second of them. */
const SHOWN_A_SESSION = 25;

/**
 * Throws unless the frames a viewer showed on `session` are at least
 * `SHOWN_A_SESSION`, each with a number that can be read and that is after
 * the one before it (by 1 to 50 modulo 256: a repaint shows the newest
 * frame that is due, so numbers are passed over, and none comes back).
 */
function checkShown(report: PageKeptReport, session: number): number {
	const shown = report.shown.filter((each) => each.session === session);
	if (shown.length < SHOWN_A_SESSION) {
		throw new Error(
			`${shown.length} frames were shown on session ${session}, not ${SHOWN_A_SESSION}: ${JSON.stringify(report)}`,
		);
	}
	shown.forEach((frame, index) => {
		const before = shown[index - 1];
		if (frame.number === null) {
			throw new Error(
				`frame ${index} of session ${session} shows no number: ${JSON.stringify(report)}`,
			);
		}
		if (before === undefined || before.number === null) return;
		const ahead = (frame.number - before.number + 256) % 256;
		if (ahead < 1 || ahead > 50) {
			throw new Error(
				`frame ${index} of session ${session} shows ${frame.number} after ${before.number}: ${JSON.stringify(report)}`,
			);
		}
	});
	return shown.length;
}

/**
 * `throughEdge`: the viewer connects to a second own relay whose parent is
 * the relay of the run (`--parent`), on the run's port plus
 * `MOQLIVEMOCK_SIDE_PORT_OFFSET`, and only the relay above is stopped. The
 * relay below loses its parent, ends the track for the viewer, refuses
 * its SUBSCRIBEs while the one above is gone, and connects to it again when
 * it is back.
 */
async function reconnect(throughEdge: boolean): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const edge = throughEdge
		? await startSecondOwnRelay(
				String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
				context,
			)
		: undefined;
	const viewerUrl = edge?.url ?? context.relayUrl;
	// The pages make their own connections; the one `openConnectedPage`
	// makes is not used.
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	/** WHY each with its name: an `evaluate` on a page that crashed says only "Target crashed". */
	const reportOf = async (
		who: "publisher" | "viewer",
		page: typeof publisher,
	): Promise<PageKeptReport> => {
		try {
			return await withTimeout(
				page.evaluate(() => window.xmoqOwn.keptReport()),
				`the ${who}'s report`,
			);
		} catch (error) {
			throw new Error(
				`the ${who}'s report: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	};
	const kept = async (): Promise<PageKeptReport> => {
		const [published, played] = await Promise.all([
			reportOf("publisher", publisher),
			reportOf("viewer", viewer),
		]);
		return {
			publishing: published.publishing,
			playing: played.playing,
			chat: played.chat,
			shown: played.shown,
		};
	};
	const until = (
		what: string,
		reached: (report: PageKeptReport) => boolean,
	): Promise<PageKeptReport> =>
		pollFor(async () => {
			const report = await kept();
			return reached(report) ? report : undefined;
		}, what);
	const shownOn = (report: PageKeptReport, session: number): number =>
		report.shown.filter((each) => each.session === session).length;

	await withTimeout(
		publisher.evaluate(
			([url, hash, namespace, setup]) =>
				window.xmoqOwn.keepVideoPublished(url, hash, namespace, setup),
			[
				context.relayUrl,
				context.certSha256Hex,
				PAGE_NAMESPACE,
				{ ...BARS, mapping: "stream-per-group" },
			] satisfies [string, string, string, PageVideoSetup],
		),
		"the page to begin publishing",
	);
	await until("the publisher to publish on its first session", (report) =>
		report.publishing.includes("publishing 1"),
	);
	await withTimeout(
		viewer.evaluate(
			([url, hash, namespace, name, codec]) =>
				window.xmoqOwn.keepVideoPlayed(url, hash, namespace, name, codec),
			[
				viewerUrl,
				context.certSha256Hex,
				PAGE_NAMESPACE,
				BARS.trackName,
				BARS.catalogCodec,
			] satisfies [string, string, string, string, string],
		),
		"the page to begin playing",
	);
	await until(
		`the viewer to show ${SHOWN_A_SESSION} frames on its first session`,
		(report) => shownOn(report, 1) >= SHOWN_A_SESSION,
	);

	await withTimeout(context.stopRelay(), "the relay to stop");
	const down = await until(
		"both pages to fail to connect while the relay is gone",
		(report) =>
			report.publishing.includes("failed") && report.playing.includes("failed"),
	);
	await withTimeout(context.startRelayAnew(), "the relay to start anew");
	const back = await until(
		`the viewer to show ${SHOWN_A_SESSION} frames on its second session`,
		(report) => shownOn(report, 2) >= SHOWN_A_SESSION,
	);

	for (const [who, states] of [
		["publisher", down.publishing],
		["viewer", down.playing],
	] as const) {
		// The own relay, stopped with SIGINT, first sends GOAWAY with no
		// New Session URI ("moved " with nothing after it) and is killed by
		// the harness before its Timeout is over; either way the page was
		// told.
		const ended = states.find(
			(state) => state.endsWith("-ended") || state === "moved ",
		);
		if (ended === undefined) {
			throw new Error(
				`the ${who} was not told that its session ended: ${JSON.stringify(down)}`,
			);
		}
	}
	if (!back.publishing.includes("publishing 2")) {
		throw new Error(
			`the publisher did not publish on a second session: ${JSON.stringify(back)}`,
		);
	}
	const first = checkShown(back, 1);
	const second = checkShown(back, 2);
	const failed = (states: string[]): number =>
		states.filter((state) => state === "failed").length;
	const where = throughEdge ? "the relay above the viewer's" : "the relay";
	return `${where} was stopped and started anew: the publisher published on a second session after ${failed(back.publishing)} tries that failed, and the viewer showed ${first} frames before and ${second} after, after ${failed(back.playing)} tries that failed`;
}

/** The relays that send GOAWAY at SIGTERM, and the GOAWAY each sends own-rust, which subscribes with Request ID 0. */
const GOAWAY_AT_SIGTERM = {
	/** moqtail's relay@0.15.0: its `DRAIN_TIMEOUT_MS`, and Request ID 0 whatever came. */
	moqtail: { timeoutMs: 10_000, requestId: 0 },
	/** The own relay: its `--goaway-timeout-ms` of 10,000, and the smallest Request ID it has not seen. */
	own: { timeoutMs: 10_000, requestId: 2 },
} as const;

type RelayThatSendsAway = keyof typeof GOAWAY_AT_SIGTERM;

/** How long the own relay waits for its closes to reach the peers (`CLOSE_REACHES_THE_PEER` of `crates/relay/src/hub.rs`). */
const CLOSE_REACHES_THE_PEER_MS = 2000;

/**
 * Sends the run's relay SIGTERM. The own relay is to say that it sends
 * GOAWAY to `sessions` sessions, naming `uri`, and to end with 0 within
 * its Timeout and the time its closes take to reach the peers
 * (`whenGone`, awaited by the caller); moqtail's log is not read.
 */
async function sendAway(
	context: RunContext,
	relay: RelayThatSendsAway,
	sessions: number,
	uri: string,
): Promise<{ whenGone: Promise<void> }> {
	const ended = relay === "own" ? context.relay.exited : undefined;
	context.signalRelay("SIGTERM");
	if (relay === "moqtail" || ended === undefined) {
		return { whenGone: Promise.resolve() };
	}
	const { timeoutMs } = GOAWAY_AT_SIGTERM[relay];
	await context.relay.waitForLine(
		new RegExp(
			`^GOING_AWAY sessions=${sessions} timeout_ms=${timeoutMs} uri=${uri.replaceAll(".", "\\.")}$`,
		),
	);
	const whenGone = (async () => {
		const code = await withTimeout(
			ended,
			"the own relay to end",
			timeoutMs + CLOSE_REACHES_THE_PEER_MS + 2000,
		);
		if (code !== 0) {
			throw new Error(`the own relay was to end with 0, not ${code}`);
		}
	})();
	// Awaited by the caller later; not an unhandled rejection until then.
	whenGone.catch(() => undefined);
	return { whenGone };
}

/**
 * `relay` on the run's port, naming the own relay on the port plus
 * `MOQLIVEMOCK_SIDE_PORT_OFFSET` in its GOAWAY (`--redirect-uri`, the flag
 * of both), and that relay.
 */
async function relayThatSendsAway(
	relay: RelayThatSendsAway,
	clients: Parameters<typeof startRun>[0]["clients"],
): Promise<{
	context: Awaited<ReturnType<typeof startRun>>;
	second: Awaited<ReturnType<typeof startSecondOwnRelay>>;
}> {
	const second = await startSecondOwnRelay(
		String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
	);
	const context = await startRun({
		relay,
		port: options.port,
		clients,
		verbose: options.verbose,
		relayArgs: [
			"--redirect-uri",
			second.url,
			// The own relay's own: moqtail's has its Timeout fixed.
			...(relay === "own"
				? ["--goaway-timeout-ms", String(GOAWAY_AT_SIGTERM.own.timeoutMs)]
				: []),
		],
	});
	return { context, second };
}

/**
 * `viewerFirst`: the publishing page waits 3 s before it connects after the
 * GOAWAY, so that the viewer comes to the new relay first, is refused there
 * and tries again: it is to try the new relay again, not the one that sent
 * the GOAWAY (that order failed in CI once, with the GOAWAY handed to the
 * next `connect` alone).
 */
async function goAwayAtSigterm(
	relay: RelayThatSendsAway,
	viewerFirst: boolean,
): Promise<string> {
	const { context, second } = await relayThatSendsAway(relay, ["own-browser"]);
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	const kept = async (): Promise<PageKeptReport> => {
		const [published, played] = await Promise.all([
			withTimeout(
				publisher.evaluate(() => window.xmoqOwn.keptReport()),
				"the publisher's report",
			),
			withTimeout(
				viewer.evaluate(() => window.xmoqOwn.keptReport()),
				"the viewer's report",
			),
		]);
		return {
			publishing: published.publishing,
			playing: played.playing,
			chat: played.chat,
			shown: played.shown,
		};
	};
	const until = (
		what: string,
		reached: (report: PageKeptReport) => boolean,
	): Promise<PageKeptReport> =>
		pollFor(async () => {
			const report = await kept();
			return reached(report) ? report : undefined;
		}, what);
	const shownOn = (report: PageKeptReport, session: number): number =>
		report.shown.filter((each) => each.session === session).length;

	await withTimeout(
		publisher.evaluate(
			([url, hash, namespace, setup]) =>
				window.xmoqOwn.keepVideoPublished(url, hash, namespace, setup),
			[
				context.relayUrl,
				context.certSha256Hex,
				PAGE_NAMESPACE,
				{
					...BARS,
					mapping: "stream-per-group",
					...(viewerFirst ? { moveDelayMs: 3000 } : {}),
				},
			] satisfies [string, string, string, PageVideoSetup],
		),
		"the page to begin publishing",
	);
	await until("the publisher to publish on its first session", (report) =>
		report.publishing.includes("publishing 1"),
	);
	await withTimeout(
		viewer.evaluate(
			([url, hash, namespace, name, codec]) =>
				window.xmoqOwn.keepVideoPlayed(url, hash, namespace, name, codec),
			[
				context.relayUrl,
				context.certSha256Hex,
				PAGE_NAMESPACE,
				BARS.trackName,
				BARS.catalogCodec,
			] satisfies [string, string, string, string, string],
		),
		"the page to begin playing",
	);
	await until(
		`the viewer to show ${SHOWN_A_SESSION} frames on its first session`,
		(report) => shownOn(report, 1) >= SHOWN_A_SESSION,
	);

	const { whenGone } = await sendAway(context, relay, 4, second.url);
	const moved = `moved ${second.url}`;
	const back = await until(
		`both pages to move to ${second.url} and the viewer to show ${SHOWN_A_SESSION} frames there`,
		(report) =>
			report.publishing.includes("publishing 2") &&
			shownOn(report, 2) >= SHOWN_A_SESSION,
	);
	for (const [who, states] of [
		["publisher", back.publishing],
		["viewer", back.playing],
	] as const) {
		if (!states.includes(moved)) {
			throw new Error(
				`the ${who} was to be moved by the GOAWAY of ${relay}'s relay to ${second.url}: ${JSON.stringify(states)}`,
			);
		}
	}
	// The order this run is for: the viewer tried the new relay before the
	// publisher was there, failed, and tried it again.
	const afterTheMove = back.playing.slice(back.playing.indexOf(moved));
	if (viewerFirst && !afterTheMove.includes("failed")) {
		throw new Error(
			`the viewer was to come to ${second.url} before the publisher and fail there first: ${JSON.stringify(back.playing)}`,
		);
	}
	const first = checkShown(back, 1);
	const after = checkShown(back, 2);
	await whenGone;
	return `${relay}'s relay sent GOAWAY at SIGTERM, naming the own relay on another port; ${viewerFirst ? "the publisher came 3 s after the viewer; " : ""}both pages moved there (publisher: ${back.publishing.join(", ")}; viewer: ${back.playing.join(", ")}), and the viewer showed ${first} frames before and ${after} after`;
}

async function goAwayAtSigtermRust(relay: RelayThatSendsAway): Promise<string> {
	const { context, second } = await relayThatSendsAway(relay, [
		"own-browser",
		"own-rust",
	]);
	const goAway = GOAWAY_AT_SIGTERM[relay];
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(
			([namespace, setup]) => window.xmoqOwn.publishVideo(namespace, setup),
			[NAMESPACE, { ...BARS, mapping: "stream-per-group" }] satisfies [
				string,
				PageVideoSetup,
			],
		),
		"the page to begin publishing",
	);
	const rust = spawnRustClient(
		context,
		"client:own-rust",
		"own-rust",
		"subscribe",
		BARS.trackName,
		["--objects", "100000", "--timeout-secs", "60"],
	);
	await rust.waitForLine(/^OBJECT /);
	// The page's session and own-rust's.
	const { whenGone } = await sendAway(context, relay, 2, second.url);
	const [line] = await rust.waitForLine(
		new RegExp(
			`^GOAWAY uri=${second.url.replaceAll(".", "\\.")} timeout_ms=${goAway.timeoutMs} request_id=${goAway.requestId}$`,
		),
	);
	const code = await withTimeout(rust.exited, "own-rust to leave");
	if (code !== 0) throw new Error(`own-rust was to leave with 0, not ${code}`);
	// WHY its own lines: the relay it names is not looked at.
	const connected = rust.matches(/^CONNECTED /).length;
	if (connected !== 1) {
		throw new Error(`own-rust was to connect once, not ${connected} times`);
	}
	await whenGone;
	return `own-rust took the GOAWAY ${relay}'s relay sent at SIGTERM (${line}) and left with 0`;
}

const CASES: Record<string, () => Promise<string>> = {
	"goaway-from-moqtail": () => goAwayAtSigterm("moqtail", false),
	"goaway-from-moqtail-viewer-first": () => goAwayAtSigterm("moqtail", true),
	"goaway-from-moqtail-rust": () => goAwayAtSigtermRust("moqtail"),
	"goaway-from-own": () => goAwayAtSigterm("own", false),
	"goaway-from-own-rust": () => goAwayAtSigtermRust("own"),
	audio,
	"audio-beyond-time": audioBeyondTime,
	"late-viewer": lateViewer,
	"late-viewer-new-group": lateViewerNewGroup,
	"to-mlmsub": toMlmsub,
	"from-mlmpub": fromMlmpub,
	"key-frames-too-large": keyFramesTooLarge,
	"page-to-page": pageToPage,
	reconnect: () => reconnect(false),
	"reconnect-through-edge": () => reconnect(true),
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
