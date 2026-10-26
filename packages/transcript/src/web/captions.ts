// Captions in a page: what a transcriber read of a voice, from its track
// `speech` of `xmoq/transcript/<prefix>` (`keepFollowingTranscript`).
//
// A caption is shown when its piece comes, and goes `shownMs` later or when
// the next comes. WHY when it comes, and not at the moment of the picture
// its Timestamps name: the words come about a second after the voice ended
// (`docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md`), later
// than a page plays the picture, so at the picture's moment there is
// nothing yet to show.

import type { GoAwayNotice } from "@xmoq/moqt";
import {
	keepFollowingTranscript,
	type TranscriptConnection,
} from "../moqt/keep-following.ts";

export interface ShowCaptionsOptions {
	/** The prefix the transcriber listens under, its fields. */
	listened: readonly string[];
	/**
	 * A connection for the transcript, with the GOAWAY that ended the session
	 * before, where one did. WHY a connection of its own: the transcript
	 * goes on while the page connects again for what it plays.
	 */
	connect(goAway?: GoAwayNotice): Promise<TranscriptConnection>;
	/** How long a caption stays when no other comes, and how old a piece of the history may be to be shown. */
	shownMs: number;
}

/**
 * Shows the captions of the transcript of `listened` in `shown`, connecting
 * again after a session closed, for as long as the page is open. Its
 * `title` says how the transcript goes.
 */
export function showCaptions(
	shown: HTMLElement,
	{ listened, connect, shownMs }: ShowCaptionsOptions,
): void {
	let clear: ReturnType<typeof setTimeout> | undefined;
	let newest = -1n;
	void keepFollowingTranscript({
		connect,
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
				// `at` is the transcriber's clock: one that runs apart from the
				// page's shows too much of the history, or too little.
				if (speech.how === "history" && Date.now() - speech.at > shownMs)
					return;
				newest = speech.number;
				shown.textContent = speech.text;
				clearTimeout(clear);
				clear = setTimeout(() => {
					shown.textContent = "";
				}, shownMs);
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
