// The viewer for the terminal, `xmoq-watch`, plays a broadcast a page
// publishes, through the own relay: it finds the video and the audio track
// in the catalog of MSF, takes both on one session, and puts them out by
// the Timestamps of LOC, with no terminal and no sound output
// (`--out /dev/null`, `--sound off`).
//
// Usage: bun watch-rust.ts [--watch page|from-mlmpub|goaway|chat] [--port N] [--verbose]
//
// What has to be seen, written before the first run.
//
// `--watch page` (when left out). The publisher is one
// page with `publishBroadcast`: the catalog, a video track of 640x360 at 30
// frames a second with a key frame a second, and an Opus track of 50 pieces
// a second, all on the page's one connection, each Object with the page's
// clock as its Timestamp. For the first 200 ms of every 2 seconds the
// picture has a white square and the sound a beep. The viewer comes when
// the page has painted 60 frames, and watches for 8 seconds.
//
//   - **The viewer names what it found in the catalog** (`CATALOG`): two
//     tracks, `video` with `avc3.42E01F` at 640x360, and `audio` with
//     `opus`, 48,000 Hz and the channel configuration `1`.
//   - It ends of itself with the exit code 0 and says why (`AV_PLAYED`):
//     "the viewer was ended", which is what `--seconds` does.
//   - **The picture**: frames of 640x360, 150 or more drawn (240 are made
//     in 8 seconds; the session and the first Group take their time), and
//     no more than 30 left out: the viewer comes in the middle of a Group
//     and begins with the next key frame. None was too large, none was
//     dropped because too many waited, none had a Timestamp that is not of
//     the clock.
//   - **The sound**: 300 packets or more (400 are made), none the decoder
//     refused, 5 seconds or more played, no more than 5 underruns, and what
//     the output was handed is not silence: its greatest sample is 0.5 or
//     more, which only the beep reaches.
//   - **Picture and sound are by one clock**: the viewer saw 2 squares or
//     more and 2 beeps or more begin, and at the median a square was
//     written within 150 ms of the moment its beep was due.
//   - Every Object had a Timestamp, and no stream was reset.
//   - The publisher, asked while the viewer watches: neither encoder
//     failed, no write of its video failed, and its sound was written.
//
// `--watch from-mlmpub`. The publisher is `mlmpub` of moqlivemock, another
// implementation, which is a server: the viewer connects to it, and no
// relay is in between. WHY: a page and a viewer of ours that agree can
// have read the drafts wrong in the same way. `mlmpub` has a catalog of 13
// tracks, LOC video of 1280x720 at 25 frames a second with a Group a
// second, and Opus of two channels.
//
//   - The viewer names what it found: of 13 tracks, `video_400kbps_avc`
//     with `avc3.4D401F` at 1280x720, the first H.264 track, and
//     `audio_monotonic_128kbps_opus` with `opus`, 48,000 Hz and the channel
//     configuration `2`, the first Opus track.
//   - It ends of itself with 0 after 8 seconds.
//   - The picture: frames of 1280x720, 120 or more drawn (200 are made),
//     no more than 25 left out, none too large or dropped.
//   - The sound: 250 packets or more (400 are made), none refused, 5
//     seconds or more played, no more than 5 underruns, and not silence:
//     the greatest sample is more than 0.01. Its two channels are mixed
//     into one by the decoder.
//   - Both tracks are by one clock: no Object had a Timestamp that is not
//     of the clock the other track set. (No mark is in this picture or
//     sound, so how far apart they leave the viewer is not measured.)
//   - Every Object of the two tracks had a Timestamp. (The first runs
//     counted 1 without: the catalog's Object, which `mlmpub` also sends
//     on the subscription to the catalog. Objects of that track are no
//     longer counted. In 3 of 7 runs the viewer then failed on the stream
//     of its FETCH for the catalog, which opened after the catalog had
//     come on the subscription; that stream is now let go of.)
//
// `--watch goaway`. Two own relays: the run's, started with
// `--redirect-uri` naming the second, on the run's port plus
// `MOQLIVEMOCK_SIDE_PORT_OFFSET`. A page publishes the broadcast through
// each, as in `--watch page`. The viewer watches through the run's relay,
// which is sent SIGTERM once the viewer has named the catalog and 3
// seconds more have passed. Written before the first run:
//
//   - The relay says `GOING_AWAY sessions=2` (the page and the viewer).
//   - The viewer says `GOAWAY timeout_ms=10000`, then `MOVED` with the
//     second relay's URL, names the same catalog again there, and goes on
//     to the end of its `--seconds`, leaving with 0 (`AV_PLAYED`, "the
//     viewer was ended") having drawn 60 frames or more and played sound
//     on the second relay.
//   - The relay ends with 0 within its Timeout and the 2 seconds its closes
//     take: the page that published through it does not move, and its
//     session is closed with GOAWAY_TIMEOUT. In the first run the page
//     closed that session itself at the GOAWAY, and the relay ended before
//     its Timeout (`STOPPING sessions=0`); either is held to the same time.
//     The viewer's first session ends with its own `AV_PLAYED`, "the relay
//     sent GOAWAY".
//
// `--watch chat`. The chat on MoQ tracks (Q36), through the own relay with
// the gatherer (`xmoq-chat`) beside it, while a page publishes the broadcast
// as in `--watch page`. Two viewers of room `r`, each with `--chat-room r`
// and a line to say (`--say`). Written before the first run:
//
//   - Viewer `wa` comes first, when the gatherer has published no `lines` of
//     the room yet: its SUBSCRIBE to `lines` is held by the relay
//     (RENDEZVOUS_TIMEOUT) until the gatherer, told of `wa`, publishes the
//     room. Its line `a1` is line 0, and `wa` shows `CHAT 0 wa: a1`.
//   - Viewer `wb` comes then, and gets line 0 by its Joining FETCH
//     (`CHAT 0 wa: a1`), and its own `b1` as line 1 (`CHAT 1 wb:
//     b1`), which `wa` shows as well.
//   - Each line is shown once, and both leave with 0 at the end of their
//     `--seconds`.
//
// `--watch chat --relay moqtail`: the same through moqtail's relay@0.15.0.
// The viewers name the tracks (`--video`, `--audio`) and read no catalog:
// through that relay the catalog does not come (the `goaway` runs of
// `video.ts`). Written before the first run: that relay does not hold a
// SUBSCRIBE for a track nobody publishes, so `wa`'s first SUBSCRIBE to
// `lines` is refused (`CHAT_TRACKS_LINES_REFUSED`), and asked for again it
// is taken once the gatherer has published the room; the lines are as
// through the own relay. In the first runs: refused in 1 of 4, taken in the
// others, as the gatherer published the room before or after the SUBSCRIBE
// came, which is not held against the run (the summary counts it). And the
// first run failed: `wb`'s Joining FETCH, then Relative with Joining Start
// 15, was refused by that relay for a room of 2 lines, and `wb` showed no
// line (docs/moqt-implementation-notes.md, "Other observations"); the
// viewer has asked with an Absolute Joining FETCH since.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	type LoggedProcess,
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnChatGatherer,
	spawnTerminalViewer,
	startMoqlivemockPublisher,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras } from "./xmoq-page.ts";
import { BROADCAST } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		watch: { type: "string", default: "page" },
		port: { type: "string", default: "24393" },
		// The matrix gives every driver a relay; this one is through the own.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const NAMESPACE = "xmoq/watch";
const PAINTED_BEFORE = 60;
const SECONDS = 8;

/** The fields of an `AV_PLAYED` line: `name=value`, a value in quotes where it has blanks. */
function fieldsOf(line: string): Map<string, string> {
	const fields = new Map<string, string>();
	for (const [, name = "", quoted, plain = ""] of line.matchAll(
		/(\S+?)=(?:"([^"]*)"|(\S*))/g,
	)) {
		fields.set(name, quoted ?? plain);
	}
	return fields;
}

/** The flags of a viewer that has no terminal and no sound output. */
const HEADLESS = [
	"--seconds",
	String(SECONDS),
	"--sound",
	"off",
	"--out",
	"/dev/null",
	"--medium",
	"direct",
	"--fit",
	"none",
];

/** What the viewer has to have done, at the least and at the most. */
interface Wanted {
	catalog: string;
	size: string;
	drawn: number;
	leftOut: number;
	packets: number;
	peak: number;
	/** Squares and beeps seen, and how far apart at the median, in ms; none where the broadcast has no marks. */
	marks?: { seen: number; apartMs: number };
	noTimestamp: number;
}

/** Waits for the viewer to end, and holds what it said against `wanted`. Gives its fields. */
async function watched(
	viewer: LoggedProcess,
	wanted: Wanted,
): Promise<Map<string, string>> {
	const [, found] = await viewer.waitForLine(/^CATALOG (.*)$/, 20_000);
	if (found !== wanted.catalog) {
		throw new Error(
			`the viewer was to find "${wanted.catalog}" in the catalog and found "${found}"`,
		);
	}
	const [line] = await viewer.waitForLine(/^AV_PLAYED .*$/, 30_000);
	const code = await withTimeout(viewer.exited, "the viewer to leave");
	const fields = fieldsOf(line);
	const text = (name: string): string => fields.get(name) ?? "missing";
	const number = (name: string): number =>
		Number(fields.get(name) ?? Number.NaN);
	const wrong: string[] = [];
	const hold = (holds: boolean, what: string): void => {
		if (!holds) wrong.push(what);
	};
	const atLeast = (name: string, least: number): void =>
		hold(number(name) >= least, `${name}=${text(name)}`);
	const atMost = (name: string, most: number): void =>
		hold(number(name) <= most, `${name}=${text(name)}`);
	hold(code === 0, `it left with ${code}`);
	hold(text("why") === "the viewer was ended", `why=${text("why")}`);
	hold(text("video_size") === wanted.size, `video_size=${text("video_size")}`);
	atLeast("video_drawn", wanted.drawn);
	atMost("video_left_out", wanted.leftOut);
	atMost("pictures_too_large", 0);
	atMost("frames_dropped_full", 0);
	atMost("not_of_the_clock", 0);
	atLeast("audio_packets", wanted.packets);
	atMost("audio_refused", 0);
	atLeast("audio_played_ms", 5000);
	atMost("underruns", 5);
	hold(number("peak") > wanted.peak, `peak=${text("peak")}`);
	if (wanted.marks !== undefined) {
		atLeast("marks_video", wanted.marks.seen);
		atLeast("marks_audio", wanted.marks.seen);
		const apart = Number(
			text("picture_after_sound_ms_min/p50/p95/max").split("/")[1] ??
				Number.NaN,
		);
		hold(
			Math.abs(apart) <= wanted.marks.apartMs,
			`picture after sound ${apart} ms at the median`,
		);
	}
	atMost("no_timestamp", wanted.noTimestamp);
	atMost("streams_reset", 0);
	if (wrong.length > 0) {
		throw new Error(`the viewer: ${wrong.join("; ")}. ${line}`);
	}
	return fields;
}

async function fromPage(): Promise<string> {
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
			([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
			[NAMESPACE] satisfies [string],
		),
		"the page to publish its broadcast",
	);
	const published = () =>
		publisher.evaluate(() => window.xmoqOwn.broadcastReport());
	await pollFor(
		async () =>
			(await published()).painted >= PAINTED_BEFORE ? true : undefined,
		`the publisher to paint ${PAINTED_BEFORE} frames`,
	);
	const viewer = spawnTerminalViewer(context, "viewer:xmoq-watch", [
		"--namespace",
		NAMESPACE,
		...HEADLESS,
		"--measure-demo-marks",
		"on",
	]);
	// WHY the publisher is asked while the viewer watches: when the viewer
	// leaves, a write of the frame that is under way fails, which is no
	// fault of the publisher's.
	const asked = (async () => {
		await new Promise((resolve) => setTimeout(resolve, (SECONDS * 1000) / 2));
		return published();
	})();
	const fields = await watched(viewer, {
		catalog: `tracks=2 video=${BROADCAST.video} codec=${BROADCAST.catalogCodec} size=${BROADCAST.width}x${BROADCAST.height} audio=${BROADCAST.audio} codec=opus samplerate=48000 channel_config=1`,
		size: `${BROADCAST.width}x${BROADCAST.height}`,
		drawn: 150,
		leftOut: 30,
		packets: 300,
		// Only the beep reaches it: the tone is 0.25 of full scale.
		peak: 0.5,
		marks: { seen: 2, apartMs: 150 },
		noTimestamp: 0,
	});
	const sent = await asked;
	if (
		sent.videoEncoderError !== null ||
		sent.audioEncoderError !== null ||
		sent.videoWriteFailures !== 0 ||
		sent.audioWritten === 0
	) {
		throw new Error(
			`the publisher's encoders were not to fail, nor a write of its video, and its sound was to be written: ${JSON.stringify(sent)}`,
		);
	}
	const apart = (
		fields.get("picture_after_sound_ms_min/p50/p95/max") ?? ""
	).split("/")[1];
	return `xmoq-watch found both tracks of a page's broadcast in its catalog and played them for ${SECONDS} s through the own relay: ${fields.get("video_drawn")} frames drawn, ${fields.get("audio_packets")} packets of sound, ${fields.get("underruns")} underruns, the picture ${apart} ms after the sound at the median of ${fields.get("marks_video")} marks`;
}

/** The namespace of `mlmpub`'s LOC tracks and their MSF catalog. */
const MLMPUB_NAMESPACE = "mlm/msf/clear";

async function fromMlmpub(): Promise<string> {
	const context = await startMoqlivemockPublisher({
		port: options.port,
		verbose: options.verbose,
	});
	const viewer = spawnTerminalViewer(context, "viewer:xmoq-watch", [
		"--namespace",
		MLMPUB_NAMESPACE,
		...HEADLESS,
	]);
	const fields = await watched(viewer, {
		catalog:
			"tracks=13 video=video_400kbps_avc codec=avc3.4D401F size=1280x720 audio=audio_monotonic_128kbps_opus codec=opus samplerate=48000 channel_config=2",
		size: "1280x720",
		drawn: 120,
		leftOut: 25,
		packets: 250,
		peak: 0.01,
		noTimestamp: 0,
	});
	return `xmoq-watch found a video and an audio track among the 13 of mlmpub's catalog and played them for ${SECONDS} s: ${fields.get("video_drawn")} frames drawn, ${fields.get("audio_packets")} packets of sound, ${fields.get("underruns")} underruns`;
}

/** The Timeout of the own relay's GOAWAY (`--goaway-timeout-ms`), and the time its closes take to reach the peers after it. */
const GOAWAY_TIMEOUT_MS = 10_000;
const CLOSE_REACHES_THE_PEER_MS = 2000;

async function goAway(): Promise<string> {
	const second = await startSecondOwnRelay(
		String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
	);
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
		relayArgs: [
			"--redirect-uri",
			second.url,
			"--goaway-timeout-ms",
			String(GOAWAY_TIMEOUT_MS),
		],
	});
	const publish = async (label: string, relayUrl: string): Promise<void> => {
		const page = await openConnectedPage(
			{ ...context, relayUrl },
			label,
			"own-browser",
		);
		await withTimeout(
			page.evaluate(
				([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
				[NAMESPACE] satisfies [string],
			),
			`${label} to publish its broadcast`,
		);
		await pollFor(
			async () =>
				(await page.evaluate(() => window.xmoqOwn.broadcastReport())).painted >=
				PAINTED_BEFORE
					? true
					: undefined,
			`${label} to paint ${PAINTED_BEFORE} frames`,
		);
	};
	await publish("publisher:first", context.relayUrl);
	await publish("publisher:second", second.url);
	const viewer = spawnTerminalViewer(context, "viewer:xmoq-watch", [
		"--namespace",
		NAMESPACE,
		...HEADLESS,
	]);
	const [, catalog] = await viewer.waitForLine(/^CATALOG (.*)$/, 20_000);
	await new Promise((resolve) => setTimeout(resolve, 3000));
	const ended = context.relay.exited;
	context.signalRelay("SIGTERM");
	await context.relay.waitForLine(
		new RegExp(
			`^GOING_AWAY sessions=2 timeout_ms=${GOAWAY_TIMEOUT_MS} uri=${second.url.replaceAll(".", "\\.")}$`,
		),
	);
	await viewer.waitForLine(
		new RegExp(`^GOAWAY timeout_ms=${GOAWAY_TIMEOUT_MS}$`),
	);
	await viewer.waitForLine(
		new RegExp(`^MOVED ${second.url.replaceAll(".", "\\.")}$`),
	);
	const catalogs = (): string[] =>
		viewer.matches(/^CATALOG (.*)$/).map((match) => match[1] ?? "");
	await pollFor(
		async () => (catalogs().length >= 2 ? true : undefined),
		"the viewer to name the catalog on the second relay",
	);
	if (catalogs().some((each) => each !== catalog)) {
		throw new Error(
			`the viewer was to name the same catalog on both relays: ${JSON.stringify(catalogs())}`,
		);
	}
	// Each session ends with its own line: the first with "the relay sent
	// GOAWAY".
	const [line] = await viewer.waitForLine(
		/^AV_PLAYED why="the viewer was ended".*$/,
		30_000,
	);
	const code = await withTimeout(viewer.exited, "the viewer to leave");
	const fields = fieldsOf(line);
	const drawn = Number(fields.get("video_drawn") ?? Number.NaN);
	const played = Number(fields.get("audio_played_ms") ?? Number.NaN);
	if (
		code !== 0 ||
		fields.get("why") !== "the viewer was ended" ||
		!(drawn >= 60) ||
		!(played > 0)
	) {
		throw new Error(
			`the viewer was to leave with 0 at its end, having drawn 60 frames or more and played sound on the second relay: ${code}, ${line}`,
		);
	}
	const relayCode = await withTimeout(
		ended,
		"the own relay to end",
		GOAWAY_TIMEOUT_MS + CLOSE_REACHES_THE_PEER_MS + 2000,
	);
	if (relayCode !== 0) {
		throw new Error(`the own relay was to end with 0, not ${relayCode}`);
	}
	return `the own relay sent GOAWAY at SIGTERM naming another; xmoq-watch moved there, named the same catalog, and drew ${drawn} frames and played ${played} ms of sound there; the relay ended with 0`;
}

/** The flags of a viewer of room `r` that says `say`, with no terminal and no sound output, for `seconds`. */
function chatViewer(
	name: string,
	say: string,
	seconds: number,
	tracks: string[],
): string[] {
	return [
		...tracks,
		"--namespace",
		NAMESPACE,
		"--seconds",
		String(seconds),
		"--sound",
		"off",
		"--out",
		"/dev/null",
		"--medium",
		"direct",
		"--fit",
		"none",
		"--chat-room",
		"r",
		"--name",
		name,
		"--say",
		say,
	];
}

async function chat(): Promise<string> {
	const relay = options.relay === "moqtail" ? "moqtail" : "own";
	const directory = mkdtempSync(join(tmpdir(), "xmoq-watch-chat-"));
	try {
		const context = await startRun({
			relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const gatherer = spawnChatGatherer(
			context,
			"gatherer",
			join(directory, "chat.sqlite3"),
		);
		await gatherer.waitForLine(/^WATCHING prefix=xmoq\/chat$/);
		const publisher = await openConnectedPage(
			context,
			"publisher",
			"own-browser",
		);
		await withTimeout(
			publisher.evaluate(
				([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
				[NAMESPACE] satisfies [string],
			),
			"the page to publish its broadcast",
		);
		// Through moqtail's relay the catalog does not come: the tracks by name.
		const tracks =
			relay === "moqtail"
				? ["--video", BROADCAST.video, "--audio", BROADCAST.audio]
				: [];
		const a = spawnTerminalViewer(
			context,
			"viewer:wa",
			chatViewer("wa", "a1", 16, tracks),
		);
		await a.waitForLine(/^CHAT 0 wa: a1$/, 20_000);
		const b = spawnTerminalViewer(
			context,
			"viewer:wb",
			chatViewer("wb", "b1", 8, tracks),
		);
		await b.waitForLine(/^CHAT 0 wa: a1$/, 20_000);
		await b.waitForLine(/^CHAT 1 wb: b1$/, 20_000);
		await a.waitForLine(/^CHAT 1 wb: b1$/, 20_000);
		const codes = await Promise.all([
			withTimeout(a.exited, "wa to leave", 30_000),
			withTimeout(b.exited, "wb to leave", 30_000),
		]);
		const shown = (viewer: LoggedProcess): string[] =>
			viewer.matches(/^CHAT (\d+ .*)$/).map((match) => match[1] ?? "");
		const wrong: string[] = [];
		if (codes.some((code) => code !== 0)) {
			wrong.push(`they left with ${codes.join(" and ")}`);
		}
		const once = ["0 wa: a1", "1 wb: b1"];
		for (const [name, viewer] of [
			["wa", a],
			["wb", b],
		] as const) {
			if (JSON.stringify(shown(viewer)) !== JSON.stringify(once)) {
				wrong.push(`${name} showed ${JSON.stringify(shown(viewer))}`);
			}
		}
		if (wrong.length > 0) {
			throw new Error(`the chat on tracks: ${wrong.join("; ")}`);
		}
		const refused = a.matches(/^CHAT_TRACKS_LINES_REFUSED /).length;
		return `through ${relay}'s relay, two terminal viewers said a line each on said through the gatherer (wa's SUBSCRIBE to lines refused ${refused} times first): wa, which came first, showed ${once.join(" and ")} live, and wb, which came later, got the first by its Joining FETCH`;
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
}

const RUNS: Record<string, () => Promise<string>> = {
	page: fromPage,
	"from-mlmpub": fromMlmpub,
	goaway: goAway,
	chat,
};

async function main(): Promise<void> {
	const run = RUNS[options.watch];
	if (run === undefined)
		throw new Error(`unknown run --watch ${options.watch}`);
	let summary: string;
	try {
		summary = await run();
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ watch: options.watch })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.watch} watch: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
