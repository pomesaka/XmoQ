// The own Rust client takes a video track a page publishes, through the own
// relay, and writes its frames as an H.264 byte stream (`--annex-b-out`).
//
// Usage: bun video-rust.ts [--port N] [--verbose]
//
// What has to be seen, written before the first run. The publisher is the
// page of `video.ts`, `page-to-page` with bars: 320x180 at 25 frames a
// second, a key frame every second, a Group on one stream, frame `n` at
// `5,000,000 + n * 40,000` microseconds for its encoder and at wall-clock
// time in the Timestamp of LOC. The Rust client comes when the page has
// painted 30 frames or more, and is told to take 60 Objects. It is not
// told the name of the video track: it reads the catalog of the namespace
// with a SUBSCRIBE and a Joining FETCH (`--catalog-video`).
//
//   - **The client names the track it found in the catalog**
//     (`CATALOG_TRACK`): `video`, with the codec `avc3.42E01F`, 320x180.
//     Added with `--catalog-video`, before its first run.
//   - The client ends with the exit code 0, and wrote 60 frames and skipped
//     none (`VIDEO_WRITTEN`): the relay subscribes to the page when the
//     client comes, the page begins a Group for it, and the Objects come in
//     order on loopback.
//   - **The first frame is Object 0 of a Group and has an IDR picture** (NAL
//     unit type 5), and so has every frame that is Object 0. No other frame
//     has one.
//   - **Within a Group each frame is the next Object ID, and its Timestamp
//     is 40,000 microseconds after the one before it.** Across Groups the
//     Group ID grows, and the Timestamp is later.
//   - **The Timestamp is wall-clock time**: the first is within a minute of
//     the driver's clock.
//   - The publisher: its encoder did not fail and no write failed.
//   - `ffprobe`, where the machine has it: the file is H.264, 320x180, and
//     has 60 frames. Where it has none the run says so.

import { spawnSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageVideoPublishReport,
	PageVideoSetup,
} from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "24391" },
		// The matrix gives every driver a relay; this one is through the own.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const NAMESPACE = "xmoq/video";
const VIDEO: PageVideoSetup = {
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
const PAINTED_BEFORE = 30;
const OBJECTS = 60;
/** The NAL unit type of a slice of an IDR picture, in H.264. */
const IDR = 5;

interface Frame {
	group: bigint;
	object: bigint;
	timestamp: bigint;
	nal: number[];
}

function check(frames: Frame[], before: number): void {
	const first = frames[0];
	if (first === undefined) throw new Error("no frame was written");
	const offBy = Number(first.timestamp) - before;
	if (Math.abs(offBy) > 60_000_000) {
		throw new Error(
			`the first frame has the Timestamp ${first.timestamp}, which is not within a minute of the clock (${before})`,
		);
	}
	frames.forEach((frame, index) => {
		const what = `frame ${index} (${frame.group}/${frame.object}, ${frame.timestamp}, NAL ${frame.nal.join(",")})`;
		if (frame.nal.includes(IDR) !== (frame.object === 0n)) {
			throw new Error(
				`${what}: an IDR picture is in Object 0 of a Group, and nowhere else`,
			);
		}
		const previous = frames[index - 1];
		if (previous === undefined) {
			if (frame.object !== 0n) throw new Error(`${what} is not Object 0`);
			return;
		}
		const inTheGroup =
			frame.group === previous.group &&
			frame.object === previous.object + 1n &&
			frame.timestamp === previous.timestamp + 40_000n;
		const nextGroup =
			frame.group > previous.group &&
			frame.object === 0n &&
			frame.timestamp > previous.timestamp;
		if (!inTheGroup && !nextGroup) {
			throw new Error(
				`${what} follows ${previous.group}/${previous.object} at ${previous.timestamp}`,
			);
		}
	});
}

/** What `ffprobe` says of the file, or why it was not asked. */
function probe(file: string): string {
	const probed = spawnSync(
		"ffprobe",
		[
			"-v",
			"error",
			"-count_frames",
			"-show_entries",
			"stream=codec_name,width,height,nb_read_frames",
			"-of",
			"csv=p=0",
			file,
		],
		{ encoding: "utf8" },
	);
	if (probed.error !== undefined) return "ffprobe is not on this machine";
	const said = probed.stdout.trim();
	if (said !== `h264,${VIDEO.width},${VIDEO.height},${OBJECTS}`) {
		throw new Error(
			`ffprobe was to read h264,${VIDEO.width},${VIDEO.height},${OBJECTS} and read "${said}" ${probed.stderr.trim()}`,
		);
	}
	return `ffprobe read ${said}`;
}

async function run(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser", "own-rust"],
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
			[NAMESPACE, VIDEO] satisfies [string, PageVideoSetup],
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

	const file = join(mkdtempSync(join(tmpdir(), "xmoq-video-")), "video.h264");
	const before = Date.now() * 1000;
	const client = spawnRustClient(
		context,
		"viewer:own-rust",
		"own-rust",
		"subscribe",
		// The catalog track of MSF, where the client finds the video track.
		"catalog",
		[
			"--catalog-video",
			// The last of two wins: the namespace of the page's video.
			"--namespace",
			NAMESPACE,
			"--objects",
			String(OBJECTS),
			"--annex-b-out",
			file,
			"--timeout-secs",
			"15",
		],
	);
	const [, found] = await client.waitForLine(/^CATALOG_TRACK (.*)$/, 20_000);
	const expected = `name=${VIDEO.trackName} codec=${VIDEO.catalogCodec} width=${VIDEO.width} height=${VIDEO.height}`;
	if (found !== expected) {
		throw new Error(
			`the client was to find "${expected}" in the catalog and found "${found}"`,
		);
	}
	const written = await client.waitForLine(
		/^VIDEO_WRITTEN file=\S+ frames=(\d+) groups=(\d+) skipped=(\d+) bytes=(\d+)$/,
		20_000,
	);
	const code = await withTimeout(client.exited, "the Rust client to leave");
	const frames = client
		.matches(
			/^VIDEO_FRAME group=(\d+) object=(\d+) timestamp=(\d+) nal=([\d,]+) bytes=\d+$/,
		)
		.map(
			([, group = "", object = "", timestamp = "", nal = ""]): Frame => ({
				group: BigInt(group),
				object: BigInt(object),
				timestamp: BigInt(timestamp),
				nal: nal.split(",").map(Number),
			}),
		);
	const [, wrote, groups, skipped, bytes] = written;
	if (
		code !== 0 ||
		wrote !== String(OBJECTS) ||
		skipped !== "0" ||
		frames.length !== OBJECTS
	) {
		throw new Error(
			`the Rust client was to leave with 0 after ${OBJECTS} frames and none skipped; it left with ${code}, wrote ${wrote}, skipped ${skipped}, and printed ${frames.length} frames`,
		);
	}
	check(frames, before);
	const sent = await published();
	if (sent.encoderError !== null || sent.writeFailures !== 0) {
		throw new Error(
			`the publisher's encoder was not to fail, and no write: ${JSON.stringify(sent)}`,
		);
	}
	return `${wrote} frames of a page's video in ${groups} Groups, ${bytes} bytes, reached the own Rust client through the own relay in order, each 40 ms after the one before it; ${probe(file)}`;
}

async function main(): Promise<void> {
	let summary: string;
	try {
		summary = await run();
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ video: "page-to-rust" })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED page-to-rust video: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
