// Pictures of a broadcast for an agent: a page publishes its broadcast
// (`publishBroadcast`: a catalog, 640x360 H.264 and Opus) through the own
// relay, and `xmoq-agent --namespace xmoq/watch` is asked for a picture with
// `/frame` on its stdin, twice, and once more after it moved to another
// relay. Two own relays, as in `watch-rust.ts --watch goaway`: the run's,
// started with `--redirect-uri` naming the second, and a page publishing
// through each.
//
// Usage: bun agent-frames.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-25), with
// `--frames-kept 1` and one relay then:
//
//   - The reader finds the video track in the catalog and subscribes to it:
//     `XMOQ_AGENT_VIDEO track=video` on its stderr.
//   - Unasked, it takes no picture: 3 s after that line, no `frame` line,
//     and no file in `<out>/frames`.
//   - `/frame` is answered within 10 s with a `frame` line, `"asked":"command"`,
//     640x360, whose `path` is in `<out>/frames` and ends in `.png`. The file
//     is a PNG of 640x360 (its IHDR). A page decodes it: the pixel at (20, 20)
//     is the grey the page paints (#444, 68 each) within 16, and the row 315
//     has 30 or more pixels of the green square (#3a7: green at least 50 over
//     red), which is 40 wide. (Changed on 2026-10-25 to 1 to 40, after a
//     run whose third picture had 4: the square is cut at the right edge.)
//   - A second `/frame`, 2 s later: another `path`, a later `timestamp_us`,
//     and with `--frames-kept 1` the first file is removed and the second is
//     there.
//   - At its end (SIGTERM), `XMOQ_AGENT_FRAMES ... taken=2 ...` on stderr.
//
// Added once the reader kept one `Frames` for its run (`begin_again`),
// written before the first run of this part, with `--frames-kept 2` in
// place of 1 (the two pictures above are then both kept):
//
//   - The run's relay is sent SIGTERM and sends GOAWAY naming the second;
//     the reader moves there, and says `XMOQ_AGENT_VIDEO track=video` a
//     second time.
//   - A third `/frame` is answered on the second relay with a picture of
//     that page's broadcast, as the first is checked, under a name none of
//     the two before had; the first file (of the first session) is removed
//     for it, and the second and the third are there.
//   - At its end, the last `XMOQ_AGENT_FRAMES` says `taken=3`: the counts go
//     on from one session to the next.
//
// Changed once the reader put the pictures of a run in a directory of its
// own (2026-10-25): `<out>/frames` above is `<out>/frames/<ms>/`, the one
// directory there, made when the reader begins.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	askForFrame,
	checkPicture,
	expectFiles,
	frameLines,
	pictures,
	runDirectory,
} from "./agent-pictures.ts";
import {
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
import { BROADCAST, type OwnPageExtras, type XmoqPage } from "./xmoq-page.ts";

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

const NAMESPACE = "xmoq/watch";

/** The Timeout of the run's relay's GOAWAY (`--goaway-timeout-ms`), as `watch-rust.ts --watch goaway` has it. */
const GOAWAY_TIMEOUT_MS = 10_000;

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-agent-frames-"));
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
		const publish = async (label: string, relayUrl: string): Promise<Page> => {
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
			return page;
		};
		const publisher = await publish("publisher:first", context.relayUrl);
		await publish("publisher:second", second.url);
		const agent = spawnAgent(context, "agent:claude", [
			"--namespace",
			NAMESPACE,
			"--chat-room",
			"r",
			"--out",
			out,
			"--frames-kept",
			"2",
			"--seconds",
			"60",
		]);
		await agent.waitForLine(/^XMOQ_AGENT_VIDEO track=video /);
		const framesDir = runDirectory(join(out, "frames"));

		await new Promise((resolve) => setTimeout(resolve, 3000));
		if (frameLines(agent).length !== 0 || pictures(framesDir).length !== 0) {
			throw new Error(
				`unasked, the reader took a picture: ${frameLines(agent).length} lines, files ${JSON.stringify(pictures(framesDir))}`,
			);
		}

		const first = await askForFrame(agent, 0);
		const seen = await checkPicture(publisher, first, framesDir, "the first");
		await new Promise((resolve) => setTimeout(resolve, 2000));
		const later = await askForFrame(agent, 1);
		await checkPicture(publisher, later, framesDir, "the second");
		if (later.path === first.path || later.timestamp_us <= first.timestamp_us) {
			throw new Error(
				`the second frame line, after ${JSON.stringify(first)}: ${JSON.stringify(later)}`,
			);
		}
		expectFiles([first.path, later.path], framesDir, "after the second");

		context.signalRelay("SIGTERM");
		// WHY the chat's line as well: while a session waits for its chat,
		// the reader answers every line of stdin with an error, `/frame`
		// with "frame: waiting for the chat" (`Reader::unless_ended`; seen
		// in the first run of this part).
		await pollFor(
			() =>
				agent.matches(/^XMOQ_AGENT_VIDEO track=video /).length >= 2 &&
				agent.matches(/^XMOQ_AGENT_CHAT /).length >= 2
					? true
					: undefined,
			"xmoq-agent to subscribe to the video and the chat on the second relay",
		);
		const moved = await askForFrame(agent, 2);
		const seenMoved = await checkPicture(
			publisher,
			moved,
			framesDir,
			"the third, on the second relay",
		);
		if (moved.path === first.path || moved.path === later.path) {
			throw new Error(
				`the third frame line has the name of one before: ${JSON.stringify(moved)}`,
			);
		}
		expectFiles([later.path, moved.path], framesDir, "after the third");

		agent.child.kill("SIGTERM");
		await withTimeout(agent.exited, "xmoq-agent to leave");
		const counts =
			agent.matches(/^XMOQ_AGENT_FRAMES .*$/).at(-1)?.[0] ?? "none";
		if (!/ taken=3 /.test(`${counts} `)) {
			throw new Error(`the reader's last counts: ${counts}`);
		}
		summary = `through the own relay, xmoq-agent found the page's video in its catalog and took no picture unasked for 3 s; /frame was answered with a PNG of ${BROADCAST.width}x${BROADCAST.height} (the grey ${JSON.stringify(seen.grey)} at (20, 20), ${seen.green} pixels of the green square on its row), and a second /frame 2 s later with a later frame (Group ${later.group} Object ${later.object}); after the relay's GOAWAY it moved to the second relay, and a third /frame there gave another file (the grey ${JSON.stringify(seenMoved.grey)}, ${seenMoved.green} green), for which --frames-kept 2 removed the first file of the first session; ${counts}`;
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
		`FAILED agent-frames: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
