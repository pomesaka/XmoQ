// How many voices one transcriber reads at once without falling behind, on one machine. N pages
// publish a voice each through the own relay (`publishVoice` of the interop page: a voice made
// with `say` and 1.5 s of silence, over and over), all at once; one xmoq-transcriber listens
// under their prefix with large-v3-turbo-q5_0 (Metal on macOS) and Silero, and the run reads
// what it prints for `--seconds`.
//
//   bun spikes/transcriber-load/run.ts --speakers N [--seconds 60] [--language auto]
//     [--model large-v3-turbo-q5_0|small-q5_1] [--readers N] [--port N]
//
// Needs `say` and `afconvert` (macOS), the release build of the transcriber, and the models in
// the cache (`mise run speech-model`, `mise run speech-model -- silero-v5.1.2`).
//
// Prints one line, LOAD {...}:
//   speakers, seconds, language, model, readers
//   cut: pieces the transcriber cut; read: pieces it read (SPEECH); dropped: PIECE_DROPPED
//   behind_ms: p50, p95 and max of SPEECH's behind_ms, the transcriber's clock when it had read
//     a piece less the piece's endUs, which is up to 200 ms after the voice ended
//   behind_ms_first, behind_ms_last: the median of the first and of the last quarter of the
//     pieces in the order they were read: a reader that falls behind has the last above the first
//   waited_ms: p50 and max of the time a piece waited for the reader; took_ms: p50 and max of
//     the time whisper.cpp took
// A throwaway: not part of the interop run.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	openConnectedPage,
	runCleanups,
	spawnTranscriber,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "../../interop/harness.ts";
import type { OwnPageExtras } from "../../interop/xmoq-page.ts";

declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		speakers: { type: "string", default: "1" },
		seconds: { type: "string", default: "60" },
		language: { type: "string", default: "auto" },
		model: { type: "string", default: "large-v3-turbo-q5_0" },
		readers: { type: "string", default: "1" },
		port: { type: "string", default: "24599" },
	},
});
stopEverythingOnExit();

const PREFIX = "xmoq/voice";
const TEXT =
	"Hello everyone. Today we will talk about how the relay delivers video and audio to many viewers.";

/** The samples of a voice made with `say`, 16-bit at 16,000 Hz, in base64, as `publishVoice` takes them. */
function voice(): string {
	const dir = mkdtempSync(join(tmpdir(), "xmoq-transcriber-load-"));
	try {
		execFileSync("say", ["-v", "Samantha", "-o", join(dir, "voice.aiff"), TEXT]);
		execFileSync("afconvert", [
			"-f",
			"WAVE",
			"-d",
			"LEI16@16000",
			"-c",
			"1",
			join(dir, "voice.aiff"),
			join(dir, "voice.wav"),
		]);
		const bytes = readFileSync(join(dir, "voice.wav"));
		let at = 12;
		while (at + 8 <= bytes.length) {
			const size = bytes.readUInt32LE(at + 4);
			if (bytes.toString("latin1", at, at + 4) === "data") {
				return bytes.subarray(at + 8, at + 8 + size).toString("base64");
			}
			at += 8 + size + (size % 2);
		}
		throw new Error("no data chunk");
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
}

const quantile = (values: number[], q: number): number | null => {
	if (values.length === 0) return null;
	const sorted = [...values].sort((a, b) => a - b);
	return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? null;
};

async function main(): Promise<void> {
	const speakers = Number(options.speakers);
	const seconds = Number(options.seconds);
	const samples = voice();
	const model = join(
		process.env.XDG_CACHE_HOME || join(homedir(), ".cache"),
		"xmoq",
		"whisper",
		`ggml-${options.model}.bin`,
	);
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: false,
	});
	// `--model` after the harness's own: the last of a flag counts.
	const transcriber = spawnTranscriber(context, "transcriber", [
		"--namespace",
		PREFIX,
		"--language",
		options.language,
		"--model",
		model,
		"--readers",
		options.readers,
		"--tracks",
		String(Math.max(16, speakers)),
	]);
	await transcriber.waitForLine(new RegExp(`^WATCHING prefix=${PREFIX}$`));
	const pages = [];
	for (let index = 0; index < speakers; index++) {
		pages.push(await openConnectedPage(context, `speaker-${index}`, "own-browser"));
	}
	// All at once: each page begins its voice when it is told, within a few milliseconds.
	await Promise.all(
		pages.map((page, index) =>
			withTimeout(
				page.evaluate(
					([namespace, voiceOf]) => window.xmoqOwn.publishVoice(namespace, voiceOf),
					[`${PREFIX}/s${index}`, samples] satisfies [string, string],
				),
				`speaker ${index} to publish`,
			),
		),
	);
	const began = Date.now();
	await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
	const read = transcriber
		.matches(/^SPEECH .* waited_ms=(\d+) took_ms=(\d+) behind_ms=(-?\d+) /)
		.map((match) => ({
			waited: Number(match[1]),
			took: Number(match[2]),
			behind: Number(match[3]),
		}));
	const behind = read.map((piece) => piece.behind);
	const quarter = Math.max(1, Math.floor(read.length / 4));
	console.log(
		`LOAD ${JSON.stringify({
			speakers,
			seconds: Math.round((Date.now() - began) / 1000),
			language: options.language,
			model: options.model,
			readers: Number(options.readers),
			texts: [...new Set(transcriber.matches(/^SPEECH .* text="(.*)"$/).map((match) => match[1]))].slice(0, 5),
			cut: transcriber.matches(/^PIECE_CUT /).length,
			read: read.length,
			dropped: transcriber.matches(/^PIECE_DROPPED /).length,
			behind_ms: [quantile(behind, 0.5), quantile(behind, 0.95), quantile(behind, 1)],
			behind_ms_first: quantile(behind.slice(0, quarter), 0.5),
			behind_ms_last: quantile(behind.slice(-quarter), 0.5),
			waited_ms: [
				quantile(
					read.map((piece) => piece.waited),
					0.5,
				),
				quantile(
					read.map((piece) => piece.waited),
					1,
				),
			],
			took_ms: [
				quantile(
					read.map((piece) => piece.took),
					0.5,
				),
				quantile(
					read.map((piece) => piece.took),
					1,
				),
			],
		})}`,
	);
}

main()
	.catch((error: unknown) => {
		console.log(`FAILED ${error instanceof Error ? error.message : String(error)}`);
		process.exitCode = 1;
	})
	.finally(() => runCleanups());
