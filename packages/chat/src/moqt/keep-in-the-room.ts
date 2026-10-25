// A participant of the chat kept across sessions: when a session ends it
// connects again and joins anew, and when the relay sends GOAWAY (§10.4) it
// goes where the GOAWAY says. The loop is `keepMoving` of @xmoq/moqt.
// Section numbers are of draft-ietf-moq-transport-18.

import {
	type GoAwayNotice,
	keepMoving,
	type MoqtConnection,
	type SessionClose,
	sessionMovingOutcome,
} from "@xmoq/moqt";
import {
	type ChatParticipant,
	type JoinChatOptions,
	joinChat,
} from "./participant.ts";

/** A connection for the chat, and the token for its PUBLISH_NAMESPACE (`JoinChatOptions.publishNamespaceToken`). */
export interface ChatConnection {
	connection: Pick<
		MoqtConnection,
		| "serve"
		| "publishNamespace"
		| "subscribe"
		| "fetch"
		| "close"
		| "closed"
		| "goAway"
	>;
	publishNamespaceToken?: Uint8Array;
}

/** Where a participant that is kept stands. */
export type RoomState =
	| { type: "connecting"; attempt: number }
	/** Joined on the `session`-th session that was joined. */
	| { type: "joined"; session: number; participant: ChatParticipant }
	/** The session ended; `participant` of the last `joined` says no more. */
	| { type: "session-ended"; close: SessionClose }
	/** The relay sent GOAWAY: the session was closed with NO_ERROR, and the next is made with `goAway` for `connect`. */
	| { type: "moved"; goAway: GoAwayNotice }
	/** No session could be made, or the room could not be joined on it. */
	| { type: "failed"; error: unknown };

export interface KeepInTheRoomOptions {
	/**
	 * Makes a connection; called anew for every session, so that a caller can
	 * ask anew for a token. `goAway` is the last GOAWAY that ended a session,
	 * and **stays the same until another comes** (`KeepMovingOptions.connect`
	 * of `@xmoq/moqt`): §10.4 has "The client MUST use this URI for the new
	 * session if provided" (`newSessionUri`; empty where the current one is
	 * reused), and the relay that sent it refuses what comes (GOING_AWAY).
	 */
	connect(goAway?: GoAwayNotice): Promise<ChatConnection>;
	/**
	 * The options of `joinChat` for each session, asked anew each time, so
	 * that `lastHad` can be the newest line shown. Its `signal` is this one's.
	 */
	join(): Omit<JoinChatOptions, "publishNamespaceToken" | "signal">;
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` when left out (`KeepMovingOptions.againMs`). */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: RoomState): void;
}

/**
 * Keeps the participant in the room for as long as `signal` lets it:
 * connects, joins (`joinChat`), and when the session ends connects again
 * after `againMs`. Resolves once `signal` was aborted; never rejects.
 *
 * **A GOAWAY ends the session with NO_ERROR**, also one that comes while the
 * namespace waits for its answer and one read after a refusal with
 * GOING_AWAY, and the next session is made with it, at once, once in a row,
 * as `keepMoving` does for every client. WHY NOT announce on the new session
 * first and leave the old later, as §9.5.1 has a publisher do: one name in
 * two sessions is two publishers of `said` at the relay, and the gatherer
 * would follow both.
 */
export async function keepInTheRoom(
	options: KeepInTheRoomOptions,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	let sessions = 0;
	await keepMoving<ChatConnection, SessionClose>({
		connect: options.connect,
		async run({ connection, publishNamespaceToken }) {
			const participant = await joinChat(connection, {
				...options.join(),
				...(publishNamespaceToken === undefined
					? {}
					: { publishNamespaceToken }),
				signal,
			});
			sessions += 1;
			options.onState?.({ type: "joined", session: sessions, participant });
			return connection.closed;
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
