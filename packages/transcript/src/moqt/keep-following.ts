// A reader of a transcript kept across sessions: when a session ends it
// connects again and follows anew, and when the relay sends GOAWAY (§10.4)
// it goes where the GOAWAY says. Section numbers are of
// draft-ietf-moq-transport-18.

import type { GoAwayNotice, MoqtConnection, SessionClose } from "@xmoq/moqt";
import {
	type FollowTranscriptOptions,
	followTranscript,
	type TranscriptReader,
} from "./follow.ts";

/**
 * How long to wait before connecting again after a session ended or could
 * not be made. **Provisional**, as the 2 seconds of `@xmoq/chat`'s
 * `keepInTheRoom` and the demo's pages.
 */
export const DEFAULT_TRANSCRIPT_AGAIN_MS = 2_000;

/** What a session of the reader needs of its connection. */
export type TranscriptConnection = Pick<
	MoqtConnection,
	"subscribe" | "fetch" | "close" | "closed" | "goAway"
>;

/** Where a reader that is kept stands. */
export type KeptTranscriptState =
	| { type: "connecting"; attempt: number }
	/** The transcript is followed on the `session`-th session that was made; `reader` is that session's. */
	| { type: "reading"; session: number; reader: TranscriptReader }
	/** The session ended. */
	| { type: "session-ended"; close: SessionClose }
	/** The relay sent GOAWAY: the session was closed with NO_ERROR, and the next is made with `goAway` for `connect`. */
	| { type: "moved"; goAway: GoAwayNotice }
	/** No session could be made. */
	| { type: "failed"; error: unknown };

export interface KeepFollowingTranscriptOptions {
	/**
	 * Makes a connection; called anew for every session, so that a caller can
	 * ask anew for a token. `goAway` is the last GOAWAY that ended a session,
	 * and **stays the same until another comes**, as for `keepPlaying` of
	 * `@xmoq/media`: §10.4 has "The client MUST use this URI for the new
	 * session if provided" (`newSessionUri`; empty where the current one is
	 * reused).
	 */
	connect(goAway?: GoAwayNotice): Promise<TranscriptConnection>;
	/** What `followTranscript` is given on each session. */
	follow: FollowTranscriptOptions;
	/** `DEFAULT_TRANSCRIPT_AGAIN_MS` when left out. */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: KeptTranscriptState): void;
}

const delay = (ms: number, signal: AbortSignal): Promise<void> =>
	new Promise((resolve) => {
		const timer = setTimeout(resolve, ms);
		signal.addEventListener(
			"abort",
			() => {
				clearTimeout(timer);
				resolve();
			},
			{ once: true },
		);
	});

/**
 * Follows the transcript for as long as `signal` lets it: connects, follows
 * (`followTranscript`), and when the session ends connects again after
 * `againMs`. Resolves once `signal` was aborted; never rejects.
 *
 * **A GOAWAY ends the session with NO_ERROR**, and the next session is made
 * with it. WHY NOT go on reading on the old session until the relay closes
 * it, as §3.6 lets a client: the relay refuses the SUBSCRIBE that follows an
 * end of `speech` (GOING_AWAY), and a full relay that sends a session away
 * (`--when-full`) closes it only after its Timeout. Each session's reader
 * fetches the latest pieces anew (`follow.latest`), so a piece the session
 * before handed on can be handed on again: the caller passes over a number
 * it had (as the demo's captions do).
 *
 * The next session after one a GOAWAY ended is made at once, once in a row,
 * as `@xmoq/media` and `@xmoq/chat` do: a relay that sends GOAWAY on every
 * session would otherwise have them made as fast as can be.
 */
export async function keepFollowingTranscript(
	options: KeepFollowingTranscriptOptions,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	const againMs = options.againMs ?? DEFAULT_TRANSCRIPT_AGAIN_MS;
	let movedBy: GoAwayNotice | undefined;
	let atOnce = false;
	let attempt = 0;
	let sessions = 0;
	while (!signal.aborted) {
		attempt += 1;
		options.onState?.({ type: "connecting", attempt });
		let moved: GoAwayNotice | undefined;
		try {
			const connection = await options.connect(movedBy);
			const stop = (): void => connection.close();
			signal.addEventListener("abort", stop, { once: true });
			const reader = followTranscript(connection, options.follow);
			try {
				if (signal.aborted) stop();
				sessions += 1;
				options.onState?.({ type: "reading", session: sessions, reader });
				const over = await Promise.race([
					connection.closed.then((close) => ({ close })),
					connection.goAway.then((notice) => ({ notice })),
				]);
				if ("notice" in over) moved = over.notice;
				// A session the caller's `signal` closed is the caller's end.
				else if (!signal.aborted) {
					options.onState?.({ type: "session-ended", close: over.close });
				}
			} finally {
				reader.stop();
				signal.removeEventListener("abort", stop);
				// NO_ERROR, also after a GOAWAY (§3.6).
				connection.close();
			}
		} catch (error) {
			if (!signal.aborted) options.onState?.({ type: "failed", error });
		}
		if (moved !== undefined) {
			movedBy = moved;
			options.onState?.({ type: "moved", goAway: moved });
		}
		atOnce = moved !== undefined && !atOnce;
		await delay(atOnce ? 0 : againMs, signal);
	}
}
