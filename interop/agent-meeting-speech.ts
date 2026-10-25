// Who speaks in a meeting, for an agent: two pages each publish a broadcast
// of a picture and a voice (`publishBroadcast` with the voice of
// `fixtures/speech/`, the same for both) under `xmoq/meet/r/<id>`, the
// transcriber listens under `xmoq/meet/r` with the smallest model, and
// `xmoq-agent --meeting r` follows its transcript and asks for `/frame`
// alone. At first p1 speaks and p2 is silent; then p1 is silent and p2
// speaks.
//
// Usage: bun agent-meeting-speech.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-25):
//
//   - The transcriber is told of both pages' namespaces and subscribes to
//     the audio of each (`AUDIO_SUBSCRIBED namespace=xmoq/meet/r/p1`, and
//     p2).
//   - The reader writes p1 and p2 as `participant` joined, and then a
//     `speech` line whose `from` is `["xmoq","meet","r","p1"]` and whose
//     text has "relay" (any case); no `speech` line of p2 while p2 is
//     silent.
//   - `/frame` alone, after that line, is answered with a `frame` line of
//     `"id":"p1"` and a PNG the page decodes, as `agent-frames.ts` checks
//     them.
//   - Once p1 is silent and p2 speaks, a `speech` line of `from`
//     `["xmoq","meet","r","p2"]` with "relay", and `/frame` alone after it
//     is answered with a `frame` line of `"id":"p2"` and such a PNG.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import { askForFrame, checkPicture, runDirectory } from "./agent-pictures.ts";
import {
	type LoggedProcess,
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnAgent,
	spawnTranscriber,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import { samplesOf, VOICE_FILE } from "./voice-file.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24399" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const PREFIX = "xmoq/meet/r";

/** The lines of JSON the reader wrote on stdout, without their `at`. */
function events(agent: LoggedProcess): string[] {
	return agent
		.matches(/^\{.*\}$/)
		.map((match) => match[0].replace(/"at":\d+,?/, ""));
}

/** The `speech` lines of a participant, each with its text. */
function speechOf(agent: LoggedProcess, id: string): string[] {
	const from = JSON.stringify(["xmoq", "meet", "r", id]);
	return events(agent).filter(
		(line) =>
			line.includes('"type":"speech"') && line.includes(`"from":${from}`),
	);
}

/** Waits for a `speech` line of `id` that has "relay" in it, and gives it. */
function spoke(agent: LoggedProcess, id: string): Promise<string> {
	return pollFor(
		() => speechOf(agent, id).find((line) => /relay/i.test(line)),
		`xmoq-agent to write what ${id} said`,
		// The voice is 1.2 s and its pause 1.5 s, and the transcriber reads a
		// piece after 600 ms of silence.
		30_000,
	);
}

/** The `id` of the `index`th `frame` line. */
function frameId(agent: LoggedProcess, index: number): string {
	const line = events(agent).filter((each) => each.includes('"type":"frame"'))[
		index
	];
	return /"id":"([^"]*)"/.exec(line ?? "")?.[1] ?? "none";
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-agent-meeting-speech-"));
	const out = join(directory, "out");
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const transcriber = spawnTranscriber(context, "transcriber", [
			"--namespace",
			PREFIX,
			"--language",
			"en",
		]);
		await transcriber.waitForLine(new RegExp(`^WATCHING prefix=${PREFIX}$`));
		const voice = samplesOf(VOICE_FILE);
		const pages = new Map<string, Page>();
		for (const [id, speaking] of [
			["p1", true],
			["p2", false],
		] as const) {
			const page = await openConnectedPage(
				context,
				`page:${id}`,
				"own-browser",
			);
			await withTimeout(
				page.evaluate(
					([namespace, samples, speaks]) =>
						window.xmoqOwn.publishBroadcast(
							namespace,
							undefined,
							samples,
							speaks,
						),
					[`${PREFIX}/${id}`, voice, speaking] satisfies [
						string,
						string,
						boolean,
					],
				),
				`${id} to publish its broadcast`,
			);
			pages.set(id, page);
		}
		for (const id of ["p1", "p2"]) {
			await transcriber.waitForLine(
				new RegExp(`^AUDIO_SUBSCRIBED namespace=${PREFIX}/${id}$`),
			);
		}
		const agent = spawnAgent(context, "agent:claude", [
			"--meeting",
			"r",
			"--out",
			out,
			"--seconds",
			"90",
		]);
		const decoder = pages.get("p1");
		if (decoder === undefined) throw new Error("no page of p1");

		const first = await spoke(agent, "p1");
		// The pictures of the run are in a directory of its own, made when the
		// reader began.
		const framesDir = runDirectory(join(out, "frames"));
		if (speechOf(agent, "p2").length !== 0) {
			throw new Error(`p2 spoke while silent: ${speechOf(agent, "p2")}`);
		}
		const ofP1 = await askForFrame(agent, 0);
		if (frameId(agent, 0) !== "p1") {
			throw new Error(`the first /frame was of ${frameId(agent, 0)}, not p1`);
		}
		await checkPicture(decoder, ofP1, framesDir, "p1's");

		await pages.get("p1")?.evaluate(() => window.xmoqOwn.setSpeaking(false));
		await pages.get("p2")?.evaluate(() => window.xmoqOwn.setSpeaking(true));
		const second = await spoke(agent, "p2");
		const ofP2 = await askForFrame(agent, 1);
		if (frameId(agent, 1) !== "p2") {
			throw new Error(`the second /frame was of ${frameId(agent, 1)}, not p2`);
		}
		await checkPicture(decoder, ofP2, framesDir, "p2's");
		summary = `through the own relay, the transcriber read p1 and then p2 of the meeting r, and xmoq-agent --meeting r wrote ${first} and ${second}; /frame alone gave a PNG of p1 after the first and of p2 after the second`;
	} finally {
		await runCleanups();
		rmSync(directory, { recursive: true, force: true });
	}
	log("driver", `RESULT ${JSON.stringify({ relay: options.relay })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED agent-meeting-speech: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
