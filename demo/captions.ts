// The captions under the picture of the watching page: what the transcriber
// (`xmoq-transcriber`, which `serve.ts` starts beside the relay) read of the
// broadcast's voice, from its track `speech` of
// `xmoq/transcript/<the broadcast's namespace>` (`followTranscript` of
// `@xmoq/transcript`).
//
// A caption is shown when its piece comes, and goes `SHOWN_MS` later or
// when the next comes. WHY when it comes, and not at the moment of the
// picture its Timestamps name: the words come about a second after the
// voice ended (`docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md`),
// later than the page plays the picture (150 ms behind), so at the
// picture's moment there is nothing yet to show.

import type { GoAwayNotice } from "@xmoq/moqt";
import {
	keepFollowingTranscript,
	type TranscriptConnection,
} from "@xmoq/transcript";

declare const XMOQ_DEMO: { transcript: boolean };

/** How long a caption stays when no other comes. Not a flag: a person reads a line of 15 s of speech in that time. */
const SHOWN_MS = 8_000;

/**
 * Shows the captions of the transcript of `listened` on connections of
 * `connectForCaptions` (with the GOAWAY that ended the session before, where
 * one did), connecting again after one closed, for as long as the page is
 * open (`keepFollowingTranscript` of `@xmoq/transcript`). Does nothing where
 * `serve.ts` runs no transcriber.
 *
 * WHY a connection of its own, as the chat has: the transcript goes on
 * while the page connects again for the broadcast.
 */
export function startCaptions(
	listened: readonly string[],
	connectForCaptions: (goAway?: GoAwayNotice) => Promise<TranscriptConnection>,
): void {
	const shown = document.getElementById("caption");
	if (!XMOQ_DEMO.transcript || shown === null) return;
	let clear: ReturnType<typeof setTimeout> | undefined;
	let newest = -1n;
	void keepFollowingTranscript({
		connect: connectForCaptions,
		follow: {
			listened,
			// The latest piece alone: a page that comes shows what was said last.
			latest: 1,
			onSpeech(speech) {
				// A piece older than the one shown (the FETCH's, after a live one,
				// or the one a new session fetches again) is not shown.
				if (speech.number <= newest) return;
				// Nor a piece of the FETCH that was read long ago: the
				// transcript keeps what was said minutes before.
				// `at` is the transcriber's clock, the same machine's in the demo.
				if (speech.how === "history" && Date.now() - speech.at > SHOWN_MS)
					return;
				newest = speech.number;
				shown.textContent = speech.text;
				clearTimeout(clear);
				clear = setTimeout(() => {
					shown.textContent = "";
				}, SHOWN_MS);
			},
			onStatus(status) {
				shown.title = status.following
					? "captions: following the transcript"
					: `captions: not following the transcript (${status.lastEnd ?? "not yet"})`;
			},
		},
		onState(state) {
			if (state.type === "moved")
				shown.title = `captions: the relay sent them to ${state.goAway.newSessionUri || "itself"}`;
			if (state.type === "failed")
				shown.title = `captions: ${String(state.error).slice(0, 120)}`;
		},
	});
}
