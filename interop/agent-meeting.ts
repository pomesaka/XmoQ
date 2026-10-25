// An agent in a meeting: two pages each publish a broadcast of their own
// (`publishBroadcast`, 640x360) under `xmoq/meet/r/<id>`, as the
// participants of a meeting do (`docs/scale-and-meetings.md`, Part 2), and
// `xmoq-agent --meeting r` is told of them and asked for their pictures.
// No transcriber runs: nobody's voice is read (`--speech off`).
//
// Usage: bun agent-meeting.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-25), and
// before the reader had `--meeting`:
//
//   - The reader writes `{"type":"participant","state":"joined","id":"p1"}`
//     and the same for `p2`, in either order, and no other participant.
//   - `/frame` alone is answered with the error line `frame: nobody has
//     spoken; /frame <id>, and there are p1, p2`.
//   - `/frame p1` is answered with a `frame` line of `"id":"p1"`, and a PNG
//     of 640x360 that a page decodes to the grey and the green square it
//     painted, as `agent-frames.ts` checks them.
//   - `/frame p2` then, with a `frame` line of `"id":"p2"` and such a PNG.
//   - `/frame p3`: the error `frame: p3 is not in the meeting, and there are
//     p1, p2`.
//   - The page of p2 closes its session: the reader writes
//     `{"type":"participant","state":"left","id":"p2"}`, and `/frame p2`
//     after it is answered with `frame: p2 is not in the meeting, and there
//     are p1`.
//   - events.jsonl has the same lines as stdout.
//

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	askForFrame,
	checkPicture,
	frameLines,
	runDirectory,
} from "./agent-pictures.ts";
import {
	type LoggedProcess,
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnAgent,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
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

/** The lines of JSON the reader wrote on stdout, without their `at`. */
function events(agent: LoggedProcess): string[] {
	return agent
		.matches(/^\{.*\}$/)
		.map((match) => match[0].replace(/"at":\d+,?/, ""));
}

/** Waits for the reader to write a line that has `fragment` in it. */
function waitForEvent(agent: LoggedProcess, fragment: string): Promise<string> {
	return pollFor(
		() => events(agent).find((line) => line.includes(fragment)),
		`xmoq-agent to write ${fragment}`,
	);
}

/** Says `command`, and waits for the error line it is answered with. */
async function erred(
	agent: LoggedProcess,
	command: string,
	message: string,
): Promise<void> {
	agent.child.stdin.write(`${command}\n`);
	await waitForEvent(
		agent,
		JSON.stringify({ message: `frame: ${message}`, type: "error" }).slice(
			1,
			-1,
		),
	);
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-agent-meeting-"));
	const out = join(directory, "out");
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const pages = new Map<string, Page>();
		for (const id of ["p1", "p2"]) {
			const page = await openConnectedPage(
				context,
				`page:${id}`,
				"own-browser",
			);
			await withTimeout(
				page.evaluate(
					([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
					[`xmoq/meet/r/${id}`] satisfies [string],
				),
				`${id} to publish its broadcast`,
			);
			pages.set(id, page);
		}
		const agent = spawnAgent(context, "agent:claude", [
			"--meeting",
			"r",
			"--out",
			out,
			"--speech",
			"off",
			"--seconds",
			"60",
		]);
		for (const id of ["p1", "p2"]) {
			await waitForEvent(
				agent,
				`{"id":"${id}","state":"joined","type":"participant"}`,
			);
		}
		// The pictures of the run are in a directory of its own, made when the
		// reader began.
		const framesDir = runDirectory(join(out, "frames"));
		await erred(
			agent,
			"/frame",
			"nobody has spoken; /frame <id>, and there are p1, p2",
		);

		const decoder = pages.get("p1");
		if (decoder === undefined) throw new Error("no page of p1");
		const seen: string[] = [];
		for (const [index, id] of ["p1", "p2"].entries()) {
			const line = await askForFrame(agent, index, `/frame ${id}`);
			const raw = events(agent).filter((each) =>
				each.includes('"type":"frame"'),
			)[index];
			if (raw === undefined || !raw.includes(`"id":"${id}"`)) {
				throw new Error(`the frame line of ${id}: ${raw ?? "none"}`);
			}
			const look = await checkPicture(decoder, line, framesDir, `${id}'s`);
			seen.push(`${id} grey ${JSON.stringify(look.grey)} green ${look.green}`);
		}
		await erred(
			agent,
			"/frame p3",
			"p3 is not in the meeting, and there are p1, p2",
		);

		await pages.get("p2")?.evaluate(() => window.xmoq.disconnect());
		await waitForEvent(
			agent,
			`{"id":"p2","state":"left","type":"participant"}`,
		);
		await erred(
			agent,
			"/frame p2",
			"p2 is not in the meeting, and there are p1",
		);

		const participants = events(agent).filter((line) =>
			line.includes('"type":"participant"'),
		);
		if (participants.length !== 3) {
			throw new Error(`the participant lines: ${JSON.stringify(participants)}`);
		}
		const written = events(agent);
		const inFile = readFileSync(join(out, "events.jsonl"), "utf8")
			.trim()
			.split("\n")
			.map((line) => line.replace(/"at":\d+,?/, ""));
		if (JSON.stringify(inFile) !== JSON.stringify(written)) {
			throw new Error("events.jsonl is not what stdout has");
		}
		summary = `through the own relay, xmoq-agent --meeting r was told of p1 and p2, answered /frame alone with "nobody has spoken", /frame p1 and /frame p2 with a PNG each (${seen.join("; ")}) in ${frameLines(agent).length} frame lines, /frame p3 with "not in the meeting", and wrote p2 as left when its page closed its session; events.jsonl had the same ${written.length} lines`;
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
		`FAILED agent-meeting: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
