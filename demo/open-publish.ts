// Opens the demo's publishing page in a headless Chromium and keeps it open
// until SIGTERM or SIGINT, for `scripts/agent-demo.sh`, which has no window to
// open it in.
//
// Usage: bun demo/open-publish.ts --port 33080 [--voice FILE.wav | --tone]
//
// With `--voice` (the script's default) the page sends that file as its
// microphone (`?audio=mic`), which Chromium plays over and over, with 1.5 s
// of silence put after it; with `--tone` the page's own tone, in which the
// transcriber hears no words.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "playwright-core";

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "8080" },
		voice: { type: "string" },
		tone: { type: "boolean", default: false },
	},
});
if (options.tone === (options.voice !== undefined)) {
	throw new Error("give one of --voice FILE.wav and --tone");
}

/** The silence put after the voice: as the interop's `publishVoice` sends it. */
const SILENCE_S = 1.5;

/**
 * `wav` with `seconds` of silence after its samples: a WAV file of PCM whose
 * data chunk is its last.
 *
 * WHY: Chromium plays the file again with no gap, and the transcriber ends a
 * piece after 600 ms without a voice; without a pause every piece was cut at
 * its longest, 15 s (docs/experiments/2026-10-25-a-voice-longer-than-a-piece.md).
 */
function withSilence(wav: Buffer, seconds: number): Buffer {
	if (wav.toString("ascii", 0, 4) !== "RIFF" || wav.toString("ascii", 8, 12) !== "WAVE") {
		throw new Error("--voice is no WAV file");
	}
	let bytesPerSecond: number | undefined;
	let blockAlign = 1;
	for (let at = 12; at + 8 <= wav.length; ) {
		const id = wav.toString("ascii", at, at + 4);
		const size = wav.readUInt32LE(at + 4);
		if (id === "fmt ") {
			bytesPerSecond = wav.readUInt32LE(at + 16);
			blockAlign = wav.readUInt16LE(at + 20);
		}
		if (id === "data") {
			if (bytesPerSecond === undefined) throw new Error("--voice has no fmt chunk before its data");
			if (at + 8 + size !== wav.length) throw new Error("--voice has a chunk after its data");
			const silence = Math.round((bytesPerSecond * seconds) / blockAlign) * blockAlign;
			const out = Buffer.concat([wav, Buffer.alloc(silence)]);
			out.writeUInt32LE(out.length - 8, 4);
			out.writeUInt32LE(size + silence, at + 4);
			return out;
		}
		at += 8 + size + (size % 2);
	}
	throw new Error("--voice has no data chunk");
}

// The file with silence, removed when this process ends, however it ends
// (`exit` comes after SIGTERM and SIGINT too, by `close` below).
let voiceFile: string | undefined;
if (options.voice !== undefined) {
	const dir = mkdtempSync(join(tmpdir(), "xmoq-open-publish-"));
	process.on("exit", () => rmSync(dir, { recursive: true, force: true }));
	voiceFile = join(dir, "voice.wav");
	writeFileSync(voiceFile, withSilence(readFileSync(options.voice), SILENCE_S));
}
const browser = await chromium.launch({
	headless: true,
	// As interop/harness.ts launches it: the full browser that
	// `playwright-core install chromium` puts in.
	channel: "chromium",
	// WHY off: closing the browser here, before this process ends, is what
	// leaves no Chromium behind when `scripts/agent-demo.sh stop` ends this
	// process by its PID.
	handleSIGINT: false,
	handleSIGTERM: false,
	args:
		voiceFile === undefined
			? []
			: [
					"--use-fake-ui-for-media-stream",
					"--use-fake-device-for-media-stream",
					`--use-file-for-fake-audio-capture=${voiceFile}`,
				],
});
let closing = false;
const close = async (): Promise<void> => {
	closing = true;
	await browser.close();
	process.exit(0);
};
process.on("SIGTERM", close);
process.on("SIGINT", close);
browser.on("disconnected", () => {
	if (closing) return;
	console.log("the browser is gone");
	process.exit(1);
});
const page = await browser.newPage();
page.on("console", (message) => console.log(`[publish] ${message.text()}`));
page.on("pageerror", (error) => console.log(`[publish] pageerror: ${error.message}`));
const url = `http://localhost:${options.port}/publish${options.voice === undefined ? "" : "?audio=mic"}`;
await page.goto(url);
console.log(`opened ${url} in Chromium ${browser.version()}`);
