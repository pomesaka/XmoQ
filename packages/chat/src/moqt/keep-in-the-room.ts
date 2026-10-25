// A participant of the chat kept across sessions: when a session ends it
// connects again and joins anew, and when the relay sends GOAWAY (§10.4) it
// goes where the GOAWAY says. Section numbers are of
// draft-ietf-moq-transport-18.

import type { GoAwayNotice, MoqtConnection, SessionClose } from "@xmoq/moqt";
import {
	type ChatParticipant,
	type JoinChatOptions,
	joinChat,
} from "./participant.ts";

/**
 * How long to wait before connecting again after a session ended or could
 * not be made. **Provisional**, as the 2 seconds the demo's pages wait.
 */
export const DEFAULT_ROOM_AGAIN_MS = 2_000;

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
	 * and **stays the same until another comes**, as for `keepPlaying` of
	 * `@xmoq/media`: §10.4 has "The client MUST use this URI for the new
	 * session if provided" (`newSessionUri`; empty where the current one is
	 * reused), and the relay that sent it refuses what comes (GOING_AWAY).
	 */
	connect(goAway?: GoAwayNotice): Promise<ChatConnection>;
	/**
	 * The options of `joinChat` for each session, asked anew each time, so
	 * that `lastHad` can be the newest line shown. Its `signal` is this one's.
	 */
	join(): Omit<JoinChatOptions, "publishNamespaceToken" | "signal">;
	/** `DEFAULT_ROOM_AGAIN_MS` when left out. */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: RoomState): void;
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
 * Keeps the participant in the room for as long as `signal` lets it:
 * connects, joins (`joinChat`), and when the session ends connects again
 * after `againMs`. Resolves once `signal` was aborted; never rejects.
 *
 * **A GOAWAY ends the session with NO_ERROR**, also one that comes while the
 * namespace waits for its answer, and the next session is made with it, as
 * `keepPublishing` of `@xmoq/media` does. WHY NOT announce on the new session
 * first and leave the old later, as §9.5.1 has a publisher do: one name in
 * two sessions is two publishers of `said` at the relay, and the gatherer
 * would follow both. WHY NOT wait for the relay to close: a full relay that
 * sends a session away (`--when-full`) closes it only after its Timeout.
 *
 * The next session after one a GOAWAY ended is made at once, once in a row,
 * as `@xmoq/media` and the terminal viewer do: a relay that sends GOAWAY on
 * every session would otherwise have them made as fast as can be.
 */
export async function keepInTheRoom(
	options: KeepInTheRoomOptions,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	const againMs = options.againMs ?? DEFAULT_ROOM_AGAIN_MS;
	let movedBy: GoAwayNotice | undefined;
	let atOnce = false;
	let attempt = 0;
	let sessions = 0;
	while (!signal.aborted) {
		attempt += 1;
		options.onState?.({ type: "connecting", attempt });
		let moved: GoAwayNotice | undefined;
		try {
			const { connection, publishNamespaceToken } =
				await options.connect(movedBy);
			let goneAway: GoAwayNotice | undefined;
			const goAway = connection.goAway.then((notice) => {
				goneAway = notice;
				return notice;
			});
			const stop = (): void => connection.close();
			signal.addEventListener("abort", stop, { once: true });
			try {
				if (signal.aborted) stop();
				const joined = await Promise.race([
					joinChat(connection, {
						...options.join(),
						...(publishNamespaceToken === undefined
							? {}
							: { publishNamespaceToken }),
						signal,
					}).then((participant) => ({ participant })),
					goAway.then((notice) => ({ notice })),
				]);
				if ("notice" in joined) {
					moved = joined.notice;
				} else {
					sessions += 1;
					options.onState?.({
						type: "joined",
						session: sessions,
						participant: joined.participant,
					});
					const over = await Promise.race([
						connection.closed.then((close) => ({ close })),
						goAway.then((notice) => ({ notice })),
					]);
					if ("notice" in over) moved = over.notice;
					// A session the caller's `signal` closed is the caller's end.
					else if (!signal.aborted) {
						options.onState?.({ type: "session-ended", close: over.close });
					}
				}
			} catch (error) {
				// After a GOAWAY the requests of the connection reject, the
				// PUBLISH_NAMESPACE among them: that is the move, not a failure.
				if (goneAway === undefined) throw error;
				moved = goneAway;
			} finally {
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
