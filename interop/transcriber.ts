// The transcriber (`crates/transcriber`, `xmoq-transcriber`) through the own
// relay, and through moqtail's for a catalog refused at first: a page
// publishes a broadcast of sound alone (`publishVoice`): a voice that says
// "Hello from the relay." (`fixtures/speech/`) and 1.5 s of silence, over and
// over. The transcriber listens under the prefix `xmoq/voice` with the
// smallest model of whisper.cpp, and own-rust clients read the transcript
// `speech` of `xmoq/transcript/xmoq/voice`.
//
// Usage: bun transcriber.ts [--case voice|catalog-refused] [--relay own|moqtail] [--auth yes|no] [--engine whisper|command] [--port N] [--verbose]
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

// `--engine command`, written before its first run (2026-10-25): the
// transcriber reads each piece with a program (`--engine-command`) in place
// of whisper.cpp. The program is a script that checks that its argument is
// a WAV file with samples in it, and prints "words from a command". A live
// reader of the transcript gets a piece from `xmoq/voice/page` with that
// text: a piece cut from the page's voice reached the program as a file,
// and its output reached the track.

// `--case catalog-refused`, through the own relay or moqtail's (`--relay`),
// written before its first run (2026-10-25): the page announces its
// namespace and refuses the catalog with DOES_NOT_EXIST, as a publisher that
// announced before it had its catalog (`publishVoice` with
// `catalogHeldBack`); the pieces are read with `--engine command`, as
// above. The transcriber is told of the page (`BROADCAST`), is refused the
// catalog (`CATALOG_NOT_SUBSCRIBED`), and says it asks again in 1,000 ms
// (`ASKING_AGAIN ... track=catalog in_ms=1000`: the refusal names no time,
// a Retry Interval of 0, and `xmoq-again` waits as after an end). Only
// once that line is read is the catalog answered (`releaseCatalog`), and the
// page takes the transcriber's SUBSCRIBE for it again. Through the own relay
// the transcriber then has the catalog, subscribes to the sound
// (`AUDIO_SUBSCRIBED`), and a live reader of the transcript gets a piece
// from `xmoq/voice/page` with the script's words. Through moqtail's the run
// ends where the page has taken the SUBSCRIBE again: that relay answers the
// Joining FETCH from the largest Location it has seen itself, so no catalog
// published before the subscriber came reaches it (the notes, the table of
// how that relay answers a FETCH, row a). The page refused at least one
// request for its catalog.

import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	captureGroup,
	log,
	openConnectedPage,
	openPage,
	pollFor,
	type RunContext,
	roomToken,
	runCleanups,
	spawnAgent,
	spawnRustClient,
	spawnTranscriber,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import { samplesOf, VOICE_FILE } from "./voice-file.ts";
import type { OwnPageExtras } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "24499" },
		// The own relay, or moqtail's for `--case catalog-refused`.
		relay: { type: "string", default: "own" },
		auth: { type: "string", default: "no" },
		engine: { type: "string", default: "whisper" },
		case: { type: "string", default: "voice" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const PREFIX = "xmoq/voice";
const TRANSCRIPT = `xmoq/transcript/${PREFIX}`;
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

/** What the script of `--engine command` prints for every piece. */
const COMMAND_WORDS = "words from a command";

/** Writes into `dir` the script of `--engine command`, which says the same words for every piece. */
function commandScript(dir: string): string {
	const script = join(dir, "read-piece.sh");
	writeFileSync(
		script,
		[
			"#!/bin/sh",
			"# $1 is the piece: a WAV file, its header and at least one sample.",
			'[ "$(head -c 4 "$1")" = RIFF ] || exit 1',
			'[ "$(wc -c < "$1")" -gt 46 ] || exit 2',
			`echo '${COMMAND_WORDS}'`,
			"",
		].join("\n"),
	);
	chmodSync(script, 0o755);
	return script;
}

/** Starts the transcriber with `--engine command` and `script`, and waits until it watches the prefix. */
async function spawnCommandTranscriber(
	context: RunContext,
	script: string,
): Promise<ReturnType<typeof spawnTranscriber>> {
	const transcriber = spawnTranscriber(
		context,
		"transcriber",
		[
			"--namespace",
			PREFIX,
			"--language",
			"en",
			"--engine",
			"command",
			"--engine-command",
			script,
		],
		"command",
	);
	await transcriber.waitForLine(/^MODEL_LOADED engine=command /);
	await transcriber.waitForLine(new RegExp(`^WATCHING prefix=${PREFIX}$`));
	return transcriber;
}

/** A live reader's one piece of the transcript, which has the script's words and is from `namespace`. */
async function commandPiece(
	context: RunContext,
	namespace: string,
): Promise<Piece> {
	const live = spawnRustClient(
		context,
		"live",
		"own-rust",
		"subscribe",
		"speech",
		["--namespace", TRANSCRIPT, "--objects", "1", "--timeout-secs", "30"],
	);
	const code = await withTimeout(live.exited, "the live reader to leave");
	if (code !== 0) throw new Error(`the live reader left with ${code}`);
	const [piece] = live
		.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
		.map(pieceOf);
	if (piece === undefined) throw new Error("the live reader got no piece");
	expect("a piece is from", piece.from.join("/"), namespace);
	expect("a piece says", piece.text, COMMAND_WORDS);
	return piece;
}

/** `--engine command`: each piece read by a script, which says the same words. */
async function withCommand(dir: string): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser", "own-rust"],
		verbose: options.verbose,
	});
	const transcriber = await spawnCommandTranscriber(
		context,
		commandScript(dir),
	);
	const namespace = `${PREFIX}/page`;
	const page = await openConnectedPage(context, "publisher", "own-browser");
	await withTimeout(
		page.evaluate(([at, voice]) => window.xmoqOwn.publishVoice(at, voice), [
			namespace,
			samplesOf(VOICE_FILE),
		] satisfies [string, string]),
		"the page to publish its voice",
	);
	await transcriber.waitForLine(
		new RegExp(`^AUDIO_SUBSCRIBED namespace=${namespace}$`),
	);
	const piece = await commandPiece(context, namespace);
	expect(
		"pieces the program failed on",
		String(transcriber.matches(/^PIECE_NOT_READ /).length),
		"0",
	);
	return `through the own relay, the transcriber read a piece of the page's voice under ${PREFIX} with --engine command: a script got it as a WAV file, and a live reader of ${TRANSCRIPT} got ${JSON.stringify(piece.text)} from ${namespace} in Group ${piece.group}`;
}

/** `--case catalog-refused`: the catalog is refused at first, and asked for again. */
async function catalogRefused(dir: string): Promise<string> {
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: ["own-browser", "own-rust"],
		verbose: options.verbose,
	});
	const transcriber = await spawnCommandTranscriber(
		context,
		commandScript(dir),
	);
	const namespace = `${PREFIX}/page`;
	const page = await openConnectedPage(context, "publisher", "own-browser");
	await withTimeout(
		page.evaluate(
			([at, voice]) => window.xmoqOwn.publishVoice(at, voice, undefined, true),
			[namespace, samplesOf(VOICE_FILE)] satisfies [string, string],
		),
		"the page to publish its voice and hold its catalog back",
	);
	await transcriber.waitForLine(
		new RegExp(`^BROADCAST namespace=${namespace}$`),
	);
	const [refused] = await transcriber.waitForLine(
		new RegExp(`^CATALOG_NOT_SUBSCRIBED namespace=${namespace} .*$`),
	);
	const [, pause = ""] = await transcriber.waitForLine(
		new RegExp(
			`^ASKING_AGAIN namespace=${namespace} track=catalog in_ms=(\\d+)$`,
		),
	);
	expect("the pause before the catalog is asked for again", pause, "1000");
	const refusals = await withTimeout(
		page.evaluate(() => window.xmoqOwn.releaseCatalog()),
		"the page to answer for its catalog from now on",
	);
	if (refusals < 1) {
		throw new Error("the page refused no request for its catalog");
	}
	await pollFor(
		async () =>
			(await page.evaluate(() => window.xmoqOwn.catalogAskedAfterRelease())) > 0
				? true
				: undefined,
		"the page to take the transcriber's SUBSCRIBE for its catalog again",
	);
	// WHY the run ends here through moqtail's relay: the catalog was published
	// before the transcriber's SUBSCRIBE that is taken now, and that relay
	// answers its Joining FETCH from what it has seen itself, nothing (the
	// notes, the table of how that relay answers a FETCH, row a).
	if (options.relay === "moqtail") {
		return `through moqtail's relay, the transcriber was refused the catalog of ${namespace} (${refused}), said it would ask again in ${pause} ms, and asked again: the page took its SUBSCRIBE once it answered for its catalog (${refusals} request(s) of it refused before)`;
	}
	await transcriber.waitForLine(
		new RegExp(`^AUDIO_SUBSCRIBED namespace=${namespace}$`),
	);
	const piece = await commandPiece(context, namespace);
	return `through the ${options.relay} relay, the transcriber was refused the catalog of ${namespace} (${refused}), said it would ask again in ${pause} ms, had the catalog once the page answered for it (${refusals} request(s) of it refused before), subscribed to the sound, and a live reader of ${TRANSCRIPT} got ${JSON.stringify(piece.text)} in Group ${piece.group}`;
}

/**
 * Runs a case, and prints its RESULT and SUMMARY once what it started is
 * stopped; `after` removes what the case made, also where it failed.
 */
async function report(
	run: () => Promise<string>,
	after: () => void = () => {},
): Promise<void> {
	let summary: string;
	try {
		summary = await run();
	} finally {
		await runCleanups();
		after();
	}
	log("driver", "RESULT {}");
	log("driver", `SUMMARY ${summary}`);
}

/** A directory of the run's own, removed when the case is over. */
function scratch(name: string): { dir: string; remove: () => void } {
	const dir = mkdtempSync(join(tmpdir(), name));
	return { dir, remove: () => rmSync(dir, { recursive: true, force: true }) };
}

async function main(): Promise<void> {
	if (options.case === "catalog-refused") {
		const { dir, remove } = scratch("xmoq-engine-command-");
		return report(() => catalogRefused(dir), remove);
	}
	if (options.case !== "voice") {
		throw new Error(`--case is voice or catalog-refused, not ${options.case}`);
	}
	// The relay is the own one for the voice: the cases below start it themselves.
	if (options.relay !== "own") {
		throw new Error(
			`--case voice runs through the own relay, not ${options.relay}`,
		);
	}
	if (options.engine === "command") {
		const { dir, remove } = scratch("xmoq-engine-command-");
		return report(() => withCommand(dir), remove);
	}
	if (options.engine !== "whisper") {
		throw new Error(`--engine is whisper or command, not ${options.engine}`);
	}
	if (options.auth === "yes") {
		return report(withTokens);
	}
	if (options.auth !== "no") {
		throw new Error(`--auth is yes or no, not ${options.auth}`);
	}
	const agentOut = scratch("xmoq-agent-speech-");
	await report(async () => {
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
		// In the order of the Groups: each comes on a subgroup stream of its
		// own, and streams arrive in no order.
		const pieces = live
			.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
			.map(pieceOf)
			.sort((one, other) => one.group - other.group);
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
			agentOut.dir,
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
		return `through the own relay, the transcriber listened to the page's voice under ${PREFIX} with the smallest model and published two pieces a live reader and a page with @xmoq/transcript got, Groups ${first.group} and ${second.group}, saying ${JSON.stringify(first.text)} and ${JSON.stringify(second.text)}, of ${(first.endUs - first.startUs) / 1000} and ${(second.endUs - second.startUs) / 1000} ms, there ${first.at - Math.round(first.endUs / 1000)} and ${second.at - Math.round(second.endUs / 1000)} ms after their ends; a late reader fetched ${fetched.length} pieces up to Group ${largest}; xmoq-agent, which came then, wrote the pieces ${numbers.join(",")} as speech lines, from ${fetchedByAgent.number} by its FETCH and ${liveToAgent.number} live; the page made ${report.made} pieces of sound, ${report.voices} voices`;
	}, agentOut.remove);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED transcriber: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
