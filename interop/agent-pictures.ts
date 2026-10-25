// What the interop runs with `xmoq-agent` check of its pictures: its
// `frame` lines, the PNG files they name, and what a page decodes of them.
// `agent-frames.ts` and `agent-meeting.ts` use them.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import type { Page } from "playwright-core";
import { type LoggedProcess, pollFor } from "./harness.ts";
import { BROADCAST } from "./xmoq-page.ts";

/** A `frame` line of the reader, as it is read here. */
export type FrameLine = {
	asked: string;
	path: string;
	width: number;
	height: number;
	group: number;
	object: number;
	timestamp_us: number;
};

/** The `frame` lines the reader wrote on stdout. */
export function frameLines(agent: LoggedProcess): FrameLine[] {
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
export async function askForFrame(
	agent: LoggedProcess,
	before: number,
	command = "/frame",
): Promise<FrameLine> {
	agent.child.stdin.write(`${command}\n`);
	return pollFor(
		() => frameLines(agent)[before],
		`xmoq-agent to answer ${command}`,
		// WHY more than 10 s: the reader gives up at 10 s, and its error line
		// is in the log then.
		12_000,
	);
}

/** The width and height in the IHDR of a PNG file. */
export function pngSize(file: Uint8Array): string {
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
export async function looked(
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
export async function checkPicture(
	page: Page,
	line: FrameLine,
	framesDir: string,
	which: string,
): Promise<{ grey: number[]; green: number }> {
	const wanted = `${BROADCAST.width}x${BROADCAST.height}`;
	if (
		line.asked !== "command" ||
		`${line.width}x${line.height}` !== wanted ||
		line.path !== inRun(framesDir, fileName(line.path)) ||
		!line.path.endsWith(".png")
	) {
		throw new Error(`${which} frame line: ${JSON.stringify(line)}`);
	}
	const file = new Uint8Array(
		readFileSync(join(framesDir, fileName(line.path))),
	);
	const size = pngSize(file);
	if (size !== wanted) {
		throw new Error(`${which} PNG is ${size}`);
	}
	const seen = await looked(page, file);
	if (
		seen.grey.length !== 3 ||
		seen.grey.some((value) => Math.abs(value - 0x44) > 16) ||
		// WHY 1 to 40 and not near 40: the square moves 4 pixels a frame to
		// the right and is cut at the edge (`publishBroadcast` in page.ts
		// paints it at x = painted * 4 mod 640), so a frame can show 4 of
		// its 40 (seen in the full run of 2026-10-25).
		seen.green < 1 ||
		seen.green > 40
	) {
		throw new Error(
			`what the page saw in ${which} PNG: ${JSON.stringify(seen)}`,
		);
	}
	return seen;
}

/** The name of the file a `path` of a `frame` line names. */
function fileName(path: string): string {
	return path.split("/").at(-1) ?? "";
}

/**
 * The `path` a `frame` line has for `file` of the run in `framesDir`: from
 * `--out`, with `/` (`crates/agent/README.md`, `fixtures/agent-lines/`).
 */
function inRun(framesDir: string, file: string): string {
	return `frames/${basename(framesDir)}/${file}`;
}

/** Holds the PNG files in `framesDir` to be `paths`, in any order. */
export function expectFiles(
	paths: string[],
	framesDir: string,
	when: string,
): void {
	const there = pictures(framesDir).sort();
	const wanted = paths.map(fileName).sort();
	if (JSON.stringify(there) !== JSON.stringify(wanted)) {
		throw new Error(
			`the files ${when}: ${JSON.stringify(there)}, not ${JSON.stringify(wanted)}`,
		);
	}
}

/**
 * The directory of the reader's run in `frames`: there is one, named by
 * when the run began (`<out>/frames/<ms>/`).
 */
export function runDirectory(frames: string): string {
	const runs = readdirSync(frames);
	const [run] = runs;
	if (runs.length !== 1 || run === undefined || !/^\d+$/.test(run)) {
		throw new Error(`${frames} has ${JSON.stringify(runs)}, not one run`);
	}
	return join(frames, run);
}

export function pictures(directory: string): string[] {
	return existsSync(directory)
		? readdirSync(directory).filter((name) => name.endsWith(".png"))
		: [];
}
