// The transcriber (`crates/transcriber`, `xmoq-transcriber`) through the own
// relay: a page publishes a broadcast of sound alone (`publishVoice`): a
// voice that says "Hello from the relay." (`fixtures/speech/`) and 1.5 s of
// silence, over and over. The transcriber listens under the prefix
// `xmoq/voice` with the smallest model of whisper.cpp, and own-rust clients
// read the transcript `speech` of `xmoq/transcript/xmoq/voice`.
//
// Usage: bun transcriber.ts [--auth yes|no] [--port N] [--verbose]
//
// What has to be seen, written before the first run:
//
//   - The transcriber is told of the page's namespace by its
//     SUBSCRIBE_NAMESPACE for `xmoq/voice` (`BROADCAST`), finds the audio
//     track in its catalog, and subscribes to it (`AUDIO_SUBSCRIBED`). It
//     has published the transcript (`TRANSCRIPT_PUBLISHED`).
//   - A live reader of `speech` gets 2 pieces: Object 0 of each Group, the
//     second Group one after the first, each `{"from":["xmoq","voice"],
//     "text":...,"startUs":...,"endUs":...,"at":...}` whose text has the
//     word "relay" (any case), where `endUs` is after `startUs` by 1 to 3
//     seconds (the voice is 1.2 s; a piece keeps up to 200 ms before it and
//     after it), and the second begins after the first ends.
//   - The words are there soon: `at` is no more than 3 seconds after
//     `endUs` (the transcriber waits 600 ms of silence after the voice, and
//     tells a voice half a second at a time with the model of voice activity; the
//     smallest model reads 1.4 s in about 0.25 s on a Mac, unmeasured on
//     the processors of CI). The run says how long it was.
//   - A page that reads the transcript with `followTranscript` of
//     `@xmoq/transcript`, from before the live reader, is handed the same
//     two pieces live, with the same text and `from` (written before the
//     run that added it, 2026-10-25).
//   - A reader that comes later FETCHes the whole transcript and gets the
//     pieces the transcriber has kept, the two above among them, and
//     FETCH_OK ends at {largest, 1}.
//   - The transcriber dropped no piece (`PIECE_DROPPED`).
//
// Written before the first run with the reader for agents (2026-10-25):
//
//   - `xmoq-agent --namespace xmoq/voice/page --transcript xmoq/voice`,
//     which comes after the pieces above, writes the latest of them, fetched
//     by its Joining FETCH, and then a piece said after it came, as lines
//     `{"type":"speech","number":N,"from":["xmoq","voice","page"],"text":...,
//     "start_us":...,"end_us":...,"read_at":...}` whose text has "relay";
//     each number once, going up by one.

// `--auth yes`, written before its first run (2026-10-25): the relay, the
// transcriber and the room's tokens have one key (`--auth-key-hex`). The
// page connects with `sub` for its namespace and announces it with `pub`
// for it (`publishVoice` with a token); the transcriber makes its own
// tokens. A reader with `sub` for `xmoq/transcript/xmoq/voice` gets two
// pieces whose text has "relay", from `xmoq/voice/page`: the voice reached
// the transcript's track through a relay that checks every request.

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	captureGroup,
	log,
	openConnectedPage,
	openPage,
	pollFor,
	roomToken,
	runCleanups,
	spawnAgent,
	spawnRustClient,
	spawnTranscriber,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "24499" },
		// The matrix gives every driver a relay; this one is through the own.
		relay: { type: "string", default: "own" },
		auth: { type: "string", default: "no" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const PREFIX = "xmoq/voice";
const TRANSCRIPT = `xmoq/transcript/${PREFIX}`;
const VOICE_FILE = join(
	import.meta.dir,
	"..",
	"fixtures",
	"speech",
	"hello-from-the-relay.wav",
);

/** The samples of a WAV file, its `data` chunk, in base64. */
function samplesOf(file: string): string {
	const bytes = readFileSync(file);
	let at = 12;
	while (at + 8 <= bytes.length) {
		const id = bytes.toString("latin1", at, at + 4);
		const size = bytes.readUInt32LE(at + 4);
		if (id === "data") {
			return bytes.subarray(at + 8, at + 8 + size).toString("base64");
		}
		at += 8 + size + (size % 2);
	}
	throw new Error(`no data chunk in ${file}`);
}

/** A piece of the transcript as a reader got it. */
type Piece = {
	group: number;
	from: string[];
	text: string;
	startUs: number;
	endUs: number;
	at: number;
};

function pieceOf(match: RegExpMatchArray): Piece {
	const group = Number(captureGroup(match, 1));
	const text = Buffer.from(captureGroup(match, 2), "hex").toString("utf8");
	const read: unknown = JSON.parse(text);
	if (typeof read !== "object" || read === null) {
		throw new Error(`a piece that is no object: ${text}`);
	}
	const field = (name: string): unknown =>
		Object.entries(read).find(([key]) => key === name)?.[1];
	const from = field("from");
	const words = field("text");
	const numbers = ["startUs", "endUs", "at"].map((name) => field(name));
	const [startUs, endUs, at] = numbers;
	if (
		!Array.isArray(from) ||
		!from.every((each) => typeof each === "string") ||
		typeof words !== "string" ||
		typeof startUs !== "number" ||
		typeof endUs !== "number" ||
		typeof at !== "number"
	) {
		throw new Error(`a piece not of the form: ${text}`);
	}
	return { group, from, text: words, startUs, endUs, at };
}

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

/** The key of the run with `--auth yes`, which the relay and the transcriber are given: 32 bytes. */
const KEY_HEX = "22".repeat(32);

/** `--auth yes`: the voice through a relay that asks for tokens. */
async function withTokens(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser", "own-rust"],
		verbose: options.verbose,
		relayArgs: ["--auth-key-hex", KEY_HEX],
	});
	const transcriber = spawnTranscriber(context, "transcriber", [
		"--namespace",
		PREFIX,
		"--language",
		"en",
		"--auth-key-hex",
		KEY_HEX,
	]);
	await transcriber.waitForLine(
		new RegExp(`^TRANSCRIPT_PUBLISHED prefix=${PREFIX} forward=true`),
	);
	await transcriber.waitForLine(new RegExp(`^WATCHING prefix=${PREFIX}$`));
	const namespace = `${PREFIX}/page`;
	const page = await openPage(context, "publisher", "own-browser");
	await withTimeout(
		page.evaluate(
			([url, hash, tokenOf]) =>
				window.xmoqOwn.connectWithToken(url, hash, tokenOf),
			[
				context.relayUrl,
				context.certSha256Hex,
				roomToken(KEY_HEX, "sub", namespace),
			] satisfies [string, string, string],
		),
		"the page to connect",
	);
	await withTimeout(
		page.evaluate(
			([at, voice, tokenOf]) => window.xmoqOwn.publishVoice(at, voice, tokenOf),
			[
				namespace,
				samplesOf(VOICE_FILE),
				roomToken(KEY_HEX, "pub", namespace),
			] satisfies [string, string, string],
		),
		"the page to publish its voice with its token",
	);
	await transcriber.waitForLine(
		new RegExp(`^AUDIO_SUBSCRIBED namespace=${namespace}$`),
	);
	const live = spawnRustClient(
		context,
		"live",
		"own-rust",
		"subscribe",
		"speech",
		[
			"--namespace",
			TRANSCRIPT,
			"--objects",
			"2",
			"--timeout-secs",
			"30",
			"--auth-token-hex",
			roomToken(KEY_HEX, "sub", TRANSCRIPT),
		],
	);
	const code = await withTimeout(live.exited, "the live reader to leave");
	if (code !== 0) throw new Error(`the live reader left with ${code}`);
	const pieces = live
		.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
		.map(pieceOf);
	if (pieces.length !== 2) {
		throw new Error(`the live reader got ${pieces.length} pieces, not 2`);
	}
	for (const piece of pieces) {
		expect("a piece is from", piece.from.join("/"), namespace);
		if (!/relay/i.test(piece.text)) {
			throw new Error(
				`a piece says ${JSON.stringify(piece.text)}, without "relay"`,
			);
		}
	}
	return `through the own relay with --auth-key-hex, the page announced ${namespace} with its pub token and the transcriber, with the same key, published two pieces that a reader with sub for ${TRANSCRIPT} got: ${pieces.map((piece) => JSON.stringify(piece.text)).join(" and ")}`;
}

async function main(): Promise<void> {
	if (options.auth === "yes") {
		let summary: string;
		try {
			summary = await withTokens();
		} finally {
			await runCleanups();
		}
		log("driver", "RESULT {}");
		log("driver", `SUMMARY ${summary}`);
		return;
	}
	if (options.auth !== "no") {
		throw new Error(`--auth is yes or no, not ${options.auth}`);
	}
	let summary: string;
	const agentOut = mkdtempSync(join(tmpdir(), "xmoq-agent-speech-"));
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: ["own-browser", "own-rust"],
			verbose: options.verbose,
		});
		const transcriber = spawnTranscriber(context, "transcriber", [
			"--namespace",
			PREFIX,
			"--language",
			"en",
		]);
		await transcriber.waitForLine(/^MODEL_LOADED /);
		await transcriber.waitForLine(
			new RegExp(`^TRANSCRIPT_PUBLISHED prefix=${PREFIX} forward=true`),
		);
		await transcriber.waitForLine(new RegExp(`^WATCHING prefix=${PREFIX}$`));
		const page = await openConnectedPage(context, "publisher", "own-browser");
		await withTimeout(
			page.evaluate(
				([namespace, voice]) => window.xmoqOwn.publishVoice(namespace, voice),
				[`${PREFIX}/page`, samplesOf(VOICE_FILE)] satisfies [string, string],
			),
			"the page to publish its voice",
		);
		await transcriber.waitForLine(
			new RegExp(`^BROADCAST namespace=${PREFIX}/page$`),
		);
		await transcriber.waitForLine(
			new RegExp(`^AUDIO_SUBSCRIBED namespace=${PREFIX}/page$`),
		);
		const reader = await openConnectedPage(context, "reader", "own-browser");
		await withTimeout(
			reader.evaluate(
				([prefix]) => window.xmoqOwn.followTranscript(prefix, 0),
				[PREFIX] satisfies [string],
			),
			"the page to follow the transcript",
		);
		const live = spawnRustClient(
			context,
			"live",
			"own-rust",
			"subscribe",
			"speech",
			["--namespace", TRANSCRIPT, "--objects", "2", "--timeout-secs", "30"],
		);
		await live.waitForLine(/^SUBSCRIBED /);
		const liveCode = await withTimeout(live.exited, "the live reader to leave");
		if (liveCode !== 0)
			throw new Error(`the live reader left with ${liveCode}`);
		const pieces = live
			.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
			.map(pieceOf);
		const [first, second] = pieces;
		if (first === undefined || second === undefined) {
			throw new Error(`the live reader got ${pieces.length} pieces, not 2`);
		}
		expect(
			"the second piece's Group",
			String(second.group),
			String(first.group + 1),
		);
		for (const piece of [first, second]) {
			expect("a piece is from", piece.from.join("/"), `${PREFIX}/page`);
			if (!/relay/i.test(piece.text)) {
				throw new Error(
					`a piece says ${JSON.stringify(piece.text)}, without "relay"`,
				);
			}
			const lasted = piece.endUs - piece.startUs;
			if (lasted < 1_000_000 || lasted > 3_000_000) {
				throw new Error(`a piece lasts ${lasted} µs, not 1 to 3 s`);
			}
			const behind = piece.at - piece.endUs / 1000;
			if (behind > 3000) {
				throw new Error(
					`a piece was there ${behind} ms after its end, more than 3 s`,
				);
			}
		}
		if (second.startUs < first.endUs) {
			throw new Error("the second piece begins before the first ends");
		}
		const onPage = await pollFor(async () => {
			const pieces = await reader.evaluate(() =>
				window.xmoqOwn.transcriptReport(),
			);
			const both = [first, second].map((piece) =>
				pieces.find((had) => had.number === String(piece.group)),
			);
			return both.every((had) => had !== undefined) ? both : undefined;
		}, "the page to be handed both pieces");
		for (const [index, piece] of [first, second].entries()) {
			const had = onPage[index];
			expect(
				`the page's piece ${piece.group}`,
				JSON.stringify([had?.text, had?.from, had?.how]),
				JSON.stringify([piece.text, piece.from, "live"]),
			);
		}

		const late = spawnRustClient(
			context,
			"late",
			"own-rust",
			"fetch",
			"speech",
			[
				"--namespace",
				TRANSCRIPT,
				"--fetch-start",
				"0,0",
				"--fetch-end",
				`${Number.MAX_SAFE_INTEGER},0`,
			],
		);
		const [ok] = await late.waitForLine(
			/^FETCH_OK request_id=\d+ end_group=(\d+) end_object=(\d+)/,
		);
		await late.waitForLine(/^FETCH_ENDED .*how=fin/);
		const fetched = late
			.matches(/^FETCH_OBJECT request_id=\d+ group=(\d+) /)
			.map((match) => Number(captureGroup(match, 1)));
		for (const piece of [first, second]) {
			if (!fetched.includes(piece.group)) {
				throw new Error(
					`the late reader did not get Group ${piece.group}: ${fetched.join(",")}`,
				);
			}
		}
		const largest = Math.max(...fetched);
		expect(
			"the late reader's FETCH_OK ends at",
			ok.replace(/^.*end_group=(\d+) end_object=(\d+).*$/, "{$1, $2}"),
			`{${largest}, 1}`,
		);
		expect(
			"pieces the transcriber dropped",
			String(transcriber.matches(/^PIECE_DROPPED /).length),
			"0",
		);

		const agent = spawnAgent(context, "agent", [
			"--namespace",
			`${PREFIX}/page`,
			"--transcript",
			PREFIX,
			"--chat-room",
			"r",
			"--name",
			"claude",
			"--frames",
			"off",
			"--out",
			agentOut,
			"--seconds",
			"60",
		]);
		const spoken = (): { number: number; line: string }[] =>
			agent.matches(/^\{.*"type":"speech".*\}$/).map((match) => ({
				number: Number(/"number":(\d+)/.exec(match[0])?.[1]),
				line: match[0],
			}));
		const fetchedByAgent = await pollFor(
			() => spoken()[0],
			"xmoq-agent to write the pieces it fetched",
			30_000,
		);
		const liveToAgent = await pollFor(
			() => spoken().find((piece) => piece.number > largest),
			"xmoq-agent to write a piece said after it came",
			30_000,
		);
		const numbers = spoken().map((piece) => piece.number);
		for (const [at, number] of numbers.entries()) {
			if (at > 0 && number !== (numbers[at - 1] ?? 0) + 1) {
				throw new Error(`xmoq-agent wrote the pieces ${numbers.join(",")}`);
			}
		}
		for (const piece of [fetchedByAgent, liveToAgent]) {
			if (!piece.line.includes(`"from":["xmoq","voice","page"]`)) {
				throw new Error(`xmoq-agent wrote ${piece.line}`);
			}
			if (!/"text":"[^"]*relay/i.test(piece.line)) {
				throw new Error(`xmoq-agent wrote ${piece.line}, without "relay"`);
			}
		}
		const report = await page.evaluate(() => window.xmoqOwn.voiceReport());
		await pollFor(
			() => (report.audioEncoderError === null ? true : undefined),
			"the page's encoder to have no error",
		);
		summary = `through the own relay, the transcriber listened to the page's voice under ${PREFIX} with the smallest model and published two pieces a live reader and a page with @xmoq/transcript got, Groups ${first.group} and ${second.group}, saying ${JSON.stringify(first.text)} and ${JSON.stringify(second.text)}, of ${(first.endUs - first.startUs) / 1000} and ${(second.endUs - second.startUs) / 1000} ms, there ${first.at - Math.round(first.endUs / 1000)} and ${second.at - Math.round(second.endUs / 1000)} ms after their ends; a late reader fetched ${fetched.length} pieces up to Group ${largest}; xmoq-agent, which came then, wrote the pieces ${numbers.join(",")} as speech lines, from ${fetchedByAgent.number} by its FETCH and ${liveToAgent.number} live; the page made ${report.made} pieces of sound, ${report.voices} voices`;
	} finally {
		await runCleanups();
		rmSync(agentOut, { recursive: true, force: true });
	}
	log("driver", "RESULT {}");
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED transcriber: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
