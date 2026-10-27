// A meeting of three pages: each publishes a broadcast of its own
// (`publishBroadcast`, 640x360 and a tone) under `xmoq/meet/r/<id>`, and
// takes part with `createMeeting` of `@xmoq/meeting` (`attendMeeting`), as
// the participants of a meeting do (the meetings of `docs/architecture.md`).
// p1 receives the video of the first one who came alone (`mostWatched` 1),
// so that one who leaves gives the place to the next.
//
// Usage: bun meeting-pages.ts [--relay own] [--port N] [--verbose]
//
// Not through moqtail's relay: its Joining FETCH of a page's catalog asks
// the page for Group 0, where the page numbers its Groups by the clock, and
// no catalog comes (docs/experiments/2026-10-24-goaway-and-moqtail.md).
//
// What has to be seen, written before the first run (2026-10-25):
//
//   - p1 publishes and attends, then p2, then p3, each when the one before
//     has. p1 is told `came p2` before `came p3`.
//   - p1 receives the video of p2 alone, and the sound of p2 and p3: frames
//     of p2 and none of p3, pieces of sound of both. p2 and p3 receive the
//     video and the sound of the two others. Nobody is told of itself.
//   - p2 closes its session. p1 and p3 are told `left p2`, and `stopped p2
//     video` and `stopped p2 audio`; p1 then `receiving p3 video`, and frames
//     of p3 come to it.
//   - After that nothing more of p2 comes to p1 or p3: its counts of frames
//     and of sound stay the same over a second.
//
// Written on 2026-10-25, before a page of a meeting wrote for an agent
// (`createAgentBridge` of `@xmoq/meeting`): every page also writes into a
// folder of its own storage (OPFS), as it would into the one a person
// chooses. Once p1 receives the video of p2, the driver writes `/frame p2`,
// `hello` and `/frame p3` into p1's `inbox.txt`, as an agent would.
//
//   - p1's `events.jsonl` has `participant` lines, `joined` for p2 and then
//     p3; a `frame` line with `id` p2, `asked` command and a `path` under
//     `frames/`, whose file is a PNG (its bytes 1 to 3 are "PNG"); an `error`
//     `not said: this page has no chat` for `hello` (the page here has no
//     chat); and an `error` `/frame p3: no picture of p3 came`, since p1
//     receives the video of one alone.
//
// Changed when a page came to write a file of `events/` a second in place
// of `events.jsonl` (2026-10-27): the lines above are those of the files of
// p1's `events/`, in the order of their names.

import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageMeetingReport,
	XmoqPage,
} from "./xmoq-page.ts";

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

function report(page: Page): Promise<PageMeetingReport> {
	return page.evaluate(() => window.xmoqOwn.meetingReport());
}

/** Waits until the meeting of `page` has an event that begins with `line`. */
function told(page: Page, id: string, line: string): Promise<string> {
	return pollFor(
		async () =>
			(await report(page)).events.find((each) => each.startsWith(line)),
		`${id} to be told ${line}`,
	);
}

/** Waits until `page` has had frames and sound of `of`, as `want` says. */
function received(
	page: Page,
	id: string,
	of: string,
	want: { frames: boolean },
): Promise<PageMeetingReport> {
	return pollFor(
		async () => {
			const now = await report(page);
			const sound = (now.audio[of] ?? 0) > 0;
			const frames = (now.frames[of] ?? 0) > 0;
			return sound && (!want.frames || frames) ? now : undefined;
		},
		`${id} to receive ${want.frames ? "the video and " : ""}the sound of ${of}`,
	);
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const pages = new Map<string, Page>();
		const page = (id: string): Page => {
			const found = pages.get(id);
			if (found === undefined) throw new Error(`no page of ${id}`);
			return found;
		};
		for (const [index, id] of ["p1", "p2", "p3"].entries()) {
			const opened = await openConnectedPage(
				context,
				`page:${id}`,
				"own-browser",
			);
			pages.set(id, opened);
			await withTimeout(
				opened.evaluate(
					([namespace]) => window.xmoqOwn.publishBroadcast(namespace),
					[`xmoq/meet/r/${id}`] satisfies [string],
				),
				`${id} to publish its broadcast`,
			);
			await opened.evaluate(
				([self, most]) => window.xmoqOwn.attendMeeting("r", self, most),
				[id, id === "p1" ? 1 : 8] satisfies [string, number],
			);
			// Each comes once the ones before it are there for p1.
			if (index > 0) await told(page("p1"), "p1", `came ${id}`);
		}

		await received(page("p1"), "p1", "p2", { frames: true });
		await received(page("p1"), "p1", "p3", { frames: false });
		for (const [id, others] of [
			["p2", ["p1", "p3"]],
			["p3", ["p1", "p2"]],
		] as const) {
			for (const other of others) {
				await received(page(id), id, other, { frames: true });
			}
		}
		const before = await report(page("p1"));
		if ((before.frames.p3 ?? 0) !== 0) {
			throw new Error(
				`p1 had frames of p3 while it watched p2: ${JSON.stringify(before)}`,
			);
		}
		for (const [id, at] of pages) {
			const events = (await report(at)).events;
			if (events.some((line) => line === `came ${id}`)) {
				throw new Error(`${id} was told of itself: ${JSON.stringify(events)}`);
			}
		}
		const order = before.events.filter((line) => line.startsWith("came "));
		if (JSON.stringify(order) !== JSON.stringify(["came p2", "came p3"])) {
			throw new Error(`p1 was told of them as ${JSON.stringify(order)}`);
		}

		// An agent asks p1 for pictures and to say a line, through its folder.
		for (const line of ["/frame p2", "hello", "/frame p3"]) {
			await page("p1").evaluate(([said]) => window.xmoqOwn.agentInbox(said), [
				line,
			] satisfies [string]);
		}
		const folder = await pollFor(async () => {
			const now = await page("p1").evaluate(() =>
				window.xmoqOwn.agentFolderReport(),
			);
			return now.lines.some((line) => line.includes("no picture of p3"))
				? now
				: undefined;
		}, "p1 to write what its inbox asked for");
		const joined = folder.lines
			.filter((line) => line.includes('"type":"participant"'))
			.map((line) =>
				/"id":"([^"]+)","state":"([^"]+)"/.exec(line)?.slice(1).join(" "),
			);
		if (JSON.stringify(joined) !== JSON.stringify(["p2 joined", "p3 joined"])) {
			throw new Error(`p1 wrote the participants as ${JSON.stringify(joined)}`);
		}
		const frame = folder.lines.find((line) => line.includes('"type":"frame"'));
		const path =
			frame === undefined
				? undefined
				: /"path":"(frames\/[^"]+)"/.exec(frame)?.[1];
		if (
			frame === undefined ||
			path === undefined ||
			!frame.includes('"id":"p2"') ||
			!frame.includes('"asked":"command"') ||
			!(folder.pictures[path] ?? "").endsWith(" PNG")
		) {
			throw new Error(`p1 wrote no picture of p2: ${JSON.stringify(folder)}`);
		}
		if (
			!folder.lines.some((line) =>
				line.includes("not said: this page has no chat"),
			)
		) {
			throw new Error(
				`p1 did not say why hello was not said: ${JSON.stringify(folder.lines)}`,
			);
		}

		await page("p2").evaluate(() => window.xmoq.disconnect());
		for (const id of ["p1", "p3"]) {
			await told(page(id), id, "left p2");
			await told(page(id), id, "stopped p2 video");
			await told(page(id), id, "stopped p2 audio");
		}
		await told(page("p1"), "p1", "receiving p3 video");
		await pollFor(
			async () => ((await report(page("p1"))).frames.p3 ?? 0) > 0 || undefined,
			"p1 to receive frames of p3",
		);
		const settled = await Promise.all(
			["p1", "p3"].map((id) => report(page(id))),
		);
		await new Promise((resolve) => setTimeout(resolve, 1_000));
		const after = await Promise.all(["p1", "p3"].map((id) => report(page(id))));
		for (const [index, id] of ["p1", "p3"].entries()) {
			const was = settled[index];
			const is = after[index];
			if (
				was === undefined ||
				is === undefined ||
				was.frames.p2 !== is.frames.p2 ||
				was.audio.p2 !== is.audio.p2
			) {
				throw new Error(
					`${id} went on receiving p2: ${JSON.stringify({ was, is })}`,
				);
			}
		}
		const last = after[0];
		summary = `through the ${options.relay} relay, three pages met in xmoq/meet/r: p1 (one video) received the video of p2 and the sound of p2 and p3, p2 and p3 the video and sound of both others; when p2 closed its session p1 and p3 stopped its video and sound, and p1 received the video of p3 in its place (its folder for an agent: ${folder.lines.length} lines, the picture of p2 at ${path} of ${folder.pictures[path] ?? ""}) (p1 had ${JSON.stringify(last?.frames)} frames and ${JSON.stringify(last?.audio)} pieces of sound)`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ relay: options.relay })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED meeting-pages: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
