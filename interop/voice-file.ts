// The voice of `fixtures/speech/`, as the pages take it (`publishVoice`,
// `publishBroadcast` with a voice). `transcriber.ts` and
// `agent-meeting-speech.ts` use it.

import { readFileSync } from "node:fs";
import { join } from "node:path";

/** The voice that says "Hello from the relay." */
export const VOICE_FILE = join(
	import.meta.dir,
	"..",
	"fixtures",
	"speech",
	"hello-from-the-relay.wav",
);

/** The samples of a WAV file, its `data` chunk, in base64. */
export function samplesOf(file: string): string {
	const bytes = readFileSync(file);
	let at = 12;
	while (at + 8 <= bytes.length) {
		const id = bytes.toString("latin1", at, at + 4);
		const size = bytes.readUInt32LE(at + 4);
		if (id === "data") {
			return bytes.subarray(at + 8, at + 8 + size).toString("base64");
		}
		at += 8 + size + (size % 2);
	}
	throw new Error(`no data chunk in ${file}`);
}
