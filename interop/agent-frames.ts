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
//     red), which is 40 wide.
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

import {
	existsSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
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

/** A `frame` line of the reader, as it is read here. */
type FrameLine = {
	asked: string;
	path: string;
	width: number;
	height: number;
	group: number;
	object: number;
	timestamp_us: number;
};

/** The `frame` lines the reader wrote on stdout. */
function frameLines(agent: LoggedProcess): FrameLine[] {
	return agent.matches(/^\{.*"type":"frame".*\}$/).map((match) => {
		const parsed: unknown = JSON.parse(match[0]);
		if (
			typeof parsed !== "object" ||
			parsed === null ||
			!("asked" in parsed) ||
			typeof parsed.asked !== "string" ||
			!("path" in parsed) ||
			typeof parsed.path !== "string" ||
			!("width" in parsed) ||
			typeof parsed.width !== "number" ||
			!("height" in parsed) ||
			typeof parsed.height !== "number" ||
			!("group" in parsed) ||
			typeof parsed.group !== "number" ||
			!("object" in parsed) ||
			typeof parsed.object !== "number" ||
			!("timestamp_us" in parsed) ||
			typeof parsed.timestamp_us !== "number"
		) {
			throw new Error(`a frame line without what it has to have: ${match[0]}`);
		}
		return {
			asked: parsed.asked,
			path: parsed.path,
			width: parsed.width,
			height: parsed.height,
			group: parsed.group,
			object: parsed.object,
			timestamp_us: parsed.timestamp_us,
		};
	});
}

/** Asks for a picture, and gives the `frame` line that answers it. */
async function askForFrame(
	agent: LoggedProcess,
	before: number,
): Promise<FrameLine> {
	agent.child.stdin.write("/frame\n");
	return pollFor(
		() => frameLines(agent)[before],
		"xmoq-agent to answer /frame",
		// WHY more than 10 s: the reader gives up at 10 s, and its error line
		// is in the log then.
		12_000,
	);
}

/** The width and height in the IHDR of a PNG file. */
function pngSize(file: Uint8Array): string {
	const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
	if (signature.some((byte, at) => file[at] !== byte)) {
		throw new Error("the file is not a PNG");
	}
	const view = new DataView(file.buffer, file.byteOffset, file.byteLength);
	return `${view.getUint32(16)}x${view.getUint32(20)}`;
}

/**
 * What a page sees in a PNG: the pixel at (20, 20), and the green pixels of
 * the row 315. WHY the page's decoder: it is not the `png` crate that wrote
 * the file. WHY the script as a string: the interop scripts are type-checked
 * without the DOM's types, and the page's own code (`page.ts`) has no
 * decoder of images to call.
 */
async function looked(
	page: Page,
	file: Uint8Array,
): Promise<{ grey: number[]; green: number }> {
	const base64 = Buffer.from(file).toString("base64");
	const seen: unknown = await page.evaluate(`(async () => {
		const bytes = Uint8Array.from(atob(${JSON.stringify(base64)}), (char) => char.charCodeAt(0));
		const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
		const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
		const context = canvas.getContext("2d");
		context.drawImage(bitmap, 0, 0);
		const grey = Array.from(context.getImageData(20, 20, 1, 1).data.slice(0, 3));
		const row = context.getImageData(0, 315, bitmap.width, 1).data;
		let green = 0;
		for (let at = 0; at < row.length; at += 4) {
			if (row[at + 1] >= row[at] + 50) green += 1;
		}
		return { grey, green };
	})()`);
	if (
		typeof seen !== "object" ||
		seen === null ||
		!("grey" in seen) ||
		!Array.isArray(seen.grey) ||
		!("green" in seen) ||
		typeof seen.green !== "number"
	) {
		throw new Error(`the page's look at the PNG: ${JSON.stringify(seen)}`);
	}
	return {
		grey: seen.grey.map((value: unknown) =>
			typeof value === "number" ? value : -1,
		),
		green: seen.green,
	};
}

/**
 * Holds a `frame` line and its file to what every picture of the page's
 * broadcast has to be, and gives what the page saw in it.
 */
async function checkPicture(
	page: Page,
	line: FrameLine,
	framesDir: string,
	which: string,
): Promise<{ grey: number[]; green: number }> {
	const wanted = `${BROADCAST.width}x${BROADCAST.height}`;
	if (
		line.asked !== "command" ||
		`${line.width}x${line.height}` !== wanted ||
		dirname(line.path) !== framesDir ||
		!line.path.endsWith(".png")
	) {
		throw new Error(`${which} frame line: ${JSON.stringify(line)}`);
	}
	const file = new Uint8Array(readFileSync(line.path));
	const size = pngSize(file);
	if (size !== wanted) {
		throw new Error(`${which} PNG is ${size}`);
	}
	const seen = await looked(page, file);
	if (
		seen.grey.length !== 3 ||
		seen.grey.some((value) => Math.abs(value - 0x44) > 16) ||
		seen.green < 30
	) {
		throw new Error(
			`what the page saw in ${which} PNG: ${JSON.stringify(seen)}`,
		);
	}
	return seen;
}

/** Holds the PNG files in `framesDir` to be `paths`, in any order. */
function expectFiles(paths: string[], framesDir: string, when: string): void {
	const there = pictures(framesDir).sort();
	const wanted = paths.map((path) => path.slice(framesDir.length + 1)).sort();
	if (JSON.stringify(there) !== JSON.stringify(wanted)) {
		throw new Error(
			`with --frames-kept 2, the files ${when}: ${JSON.stringify(there)}, not ${JSON.stringify(wanted)}`,
		);
	}
}

/**
 * The directory of the reader's run in `frames`: there is one, named by
 * when the run began (`<out>/frames/<ms>/`).
 */
function runDirectory(frames: string): string {
	const runs = readdirSync(frames);
	const [run] = runs;
	if (runs.length !== 1 || run === undefined || !/^\d+$/.test(run)) {
		throw new Error(`${frames} has ${JSON.stringify(runs)}, not one run`);
	}
	return join(frames, run);
}

function pictures(directory: string): string[] {
	return existsSync(directory)
		? readdirSync(directory).filter((name) => name.endsWith(".png"))
		: [];
}

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
