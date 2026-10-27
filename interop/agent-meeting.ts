// An agent in a meeting: two pages each publish a broadcast of their own
// (`publishBroadcast`, 640x360) under `xmoq/meet/r/<id>`, as the
// participants of a meeting do (the meetings of `docs/architecture.md`), and
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
//   - `/frame p1` again (written before its first run, 2026-10-26): p1's
//     video was let go of for p2's, and is subscribed to again on the same
//     session, with a `frame` line of `"id":"p1"` and such a PNG.
//   - `/frame p3`: the error `frame: p3 is not in the meeting, and there are
//     p1, p2`.
//   - The page of p2 closes its session: the reader writes
//     `{"type":"participant","state":"left","id":"p2"}`, and `/frame p2`
//     after it is answered with `frame: p2 is not in the meeting, and there
//     are p1`.
//   - events.jsonl has the same lines as stdout.
//
// Over a GOAWAY, written before its first run (2026-10-25): the run's relay
// names a second own relay (`--redirect-uri`), as in `agent-frames.ts`. p4
// joins on the first relay; p1 (again) and p3 publish on the second. The
// first relay is sent SIGTERM, and the reader moves to the second:
//
//   - It writes `{"id":"p4","state":"joined",...}` before the move, and
//     after it `{"id":"p3","state":"joined",...}`, and p4 as `left` (not
//     told of on the second within 2 s).
//   - p1 is written as joined once in the whole run: the second relay tells
//     of it again, and the reader kept it.
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
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnAgent,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

/** The Timeout of the run's relay's GOAWAY (`--goaway-timeout-ms`), as `agent-frames.ts` has it. */
const GOAWAY_TIMEOUT_MS = 10_000;

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
		const second = await startSecondOwnRelay(
			String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
		);
		const context = await startRun({
			relay: options.relay,
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
		/** A page that publishes the broadcast of `id` at `relayUrl`. */
		const participant = async (id: string, relayUrl: string): Promise<Page> => {
			const page = await openConnectedPage(
				{ ...context, relayUrl },
				`page:${id}@${relayUrl}`,
				"own-browser",
			);
			await withTimeout(
				page.evaluate(
					([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
					[`xmoq/meet/r/${id}`] satisfies [string],
				),
				`${id} to publish its broadcast`,
			);
			return page;
		};
		const pages = new Map<string, Page>();
		for (const id of ["p1", "p2"]) {
			pages.set(id, await participant(id, context.relayUrl));
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
		// p1 again: the video let go of for p2's is asked for anew on the same
		// session.
		for (const [index, id] of ["p1", "p2", "p1"].entries()) {
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

		// Over a GOAWAY: p4 is on the first relay alone, p1 and p3 on the
		// second, and the reader keeps who was there from one session to the
		// next.
		await participant("p4", context.relayUrl);
		await waitForEvent(
			agent,
			`{"id":"p4","state":"joined","type":"participant"}`,
		);
		for (const id of ["p1", "p3"]) await participant(id, second.url);
		context.signalRelay("SIGTERM");
		await waitForEvent(
			agent,
			`{"id":"p3","state":"joined","type":"participant"}`,
		);
		await waitForEvent(
			agent,
			`{"id":"p4","state":"left","type":"participant"}`,
		);

		const participants = events(agent).filter((line) =>
			line.includes('"type":"participant"'),
		);
		const p1Joined = participants.filter((line) =>
			line.includes('{"id":"p1","state":"joined"'),
		);
		if (participants.length !== 6 || p1Joined.length !== 1) {
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
		summary = `through the own relay, xmoq-agent --meeting r was told of p1 and p2, answered /frame alone with "nobody has spoken", /frame p1, /frame p2 and /frame p1 again with a PNG each (${seen.join("; ")}) in ${frameLines(agent).length} frame lines, /frame p3 with "not in the meeting", and wrote p2 as left when its page closed its session; after the relay's GOAWAY it moved to the second relay, wrote p3 as joined and p4 as left, and p1, there again, not as joined a second time; events.jsonl had the same ${written.length} lines`;
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
