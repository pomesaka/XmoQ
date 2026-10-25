// A reader of a transcript kept across sessions: when a session ends it
// connects again and follows anew, and when the relay sends GOAWAY (§10.4)
// it goes where the GOAWAY says. The loop is `keepMoving` of @xmoq/moqt.
// Section numbers are of draft-ietf-moq-transport-18.

import {
	type GoAwayNotice,
	keepMoving,
	type MoqtConnection,
	type SessionClose,
	sessionMovingOutcome,
} from "@xmoq/moqt";
import {
	type FollowTranscriptOptions,
	followTranscript,
	type TranscriptReader,
} from "./follow.ts";

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
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` when left out (`KeepMovingOptions.againMs`). */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: KeptTranscriptState): void;
}

/**
 * Follows the transcript for as long as `signal` lets it: connects, follows
 * (`followTranscript`), and when the session ends connects again after
 * `againMs`. Resolves once `signal` was aborted; never rejects.
 *
 * **A GOAWAY ends the session with NO_ERROR**, and the next session is made
 * with it, at once, once in a row, as `keepMoving` does for every client.
 * WHY NOT go on reading on the old session until the relay closes it, as
 * §3.6 lets a client: the relay refuses the SUBSCRIBE that follows an end
 * of `speech` (GOING_AWAY), and a full relay that sends a session away
 * (`--when-full`) closes it only after its Timeout. Each session's reader
 * fetches the latest pieces anew (`follow.latest`), so a piece the session
 * before handed on can be handed on again: the caller passes over a number
 * it had (as the demo's captions do).
 */
export async function keepFollowingTranscript(
	options: KeepFollowingTranscriptOptions,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	let sessions = 0;
	await keepMoving<{ connection: TranscriptConnection }, SessionClose>({
		connect: async (goAway) => ({ connection: await options.connect(goAway) }),
		async run({ connection }) {
			const reader = followTranscript(connection, options.follow);
			try {
				sessions += 1;
				options.onState?.({ type: "reading", session: sessions, reader });
				return await connection.closed;
			} finally {
				reader.stop();
			}
		},
		...(options.againMs === undefined ? {} : { againMs: options.againMs }),
		signal,
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			const state = sessionMovingOutcome(outcome);
			// A session the caller's `signal` closed is the caller's end.
			if (state.type !== "moved" && signal.aborted) return;
			options.onState?.(state);
		},
	});
}
