// A participant of a meeting kept across sessions: when a session ends it
// connects again and attends anew, keeping who was there, and when the
// relay sends GOAWAY (draft 18 §10.4) it goes where the GOAWAY says. The
// loop is `keepMoving` of @xmoq/moqt.

import {
	type GoAwayNotice,
	keepMoving,
	type MoqtConnection,
	type MovingConnection,
	type NamespaceSubscriptionEnd,
} from "@xmoq/moqt";

/** Where a participant that is kept stands. */
export type AttendingState<C = MoqtConnection> =
	| { type: "connecting"; attempt: number }
	/** It attends on the `session`-th session whose SUBSCRIBE_NAMESPACE was taken, as `joined` of `@xmoq/chat` counts. */
	| { type: "attending"; session: number; connection: C }
	/** The SUBSCRIBE_NAMESPACE ended, and the session with it: `ended` says how. */
	| { type: "attending-ended"; ended: NamespaceSubscriptionEnd }
	/** The relay sent GOAWAY: the session was closed with NO_ERROR, and the next is made with `goAway` for `connect`. */
	| { type: "moved"; goAway: GoAwayNotice }
	/** No session could be made, or the meeting could not be attended on it. */
	| { type: "failed"; error: unknown };

export interface KeepAttendingOptions<
	C extends MovingConnection = MoqtConnection,
> {
	/** The meeting of `createMeeting` (`Meeting`); it keeps who was there between sessions. */
	meeting: {
		attend(
			connection: C,
			signal: AbortSignal,
			onTaken: () => void,
		): Promise<NamespaceSubscriptionEnd>;
	};
	/**
	 * Makes a connection; called anew for every session, so that a caller can
	 * ask the room anew for a seat and its tokens. `goAway` is the last
	 * GOAWAY that ended a session, and **stays the same until another comes**
	 * (`KeepMovingOptions.connect` of `@xmoq/moqt`): §10.4 has "The client
	 * MUST use this URI for the new session if provided" (`newSessionUri`;
	 * empty where the current one is reused).
	 */
	connect(goAway?: GoAwayNotice): Promise<C>;
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` when left out. */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: AttendingState<C>): void;
}

/**
 * Attends the meeting for as long as `signal` lets it: connects, attends
 * (`Meeting.attend`), and when that ends connects again. Resolves once
 * `signal` was aborted; never rejects.
 *
 * **A GOAWAY ends the session with NO_ERROR**, and the next session is made
 * with it, at once, once in a row, as `keepMoving` does for every client.
 * WHY NOT stay on the old session until the relay closes it, as §3.6 lets a
 * client: §10.4 has the relay refuse every request after its GOAWAY
 * (GOING_AWAY), so the sound and the video of one who comes after it could
 * not be subscribed to, and a full relay that sends a session away
 * (`--when-full`) closes it only after its Timeout.
 *
 * WHY a new session also where only the SUBSCRIBE_NAMESPACE ended: the
 * relay that ended it can be told of nobody on that session any more, and
 * whoever was there is followed on the next at once (`Meeting.attend`).
 */
export async function keepAttending<
	C extends MovingConnection = MoqtConnection,
>(options: KeepAttendingOptions<C>): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	let sessions = 0;
	await keepMoving<{ connection: C }, NamespaceSubscriptionEnd>({
		connect: async (goAway) => ({ connection: await options.connect(goAway) }),
		run({ connection }, running) {
			return options.meeting.attend(connection, running, () => {
				sessions += 1;
				options.onState?.({ type: "attending", session: sessions, connection });
			});
		},
		...(options.againMs === undefined ? {} : { againMs: options.againMs }),
		signal,
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			// A session the caller's `signal` closed is the caller's end.
			if (outcome.type !== "moved" && signal.aborted) return;
			options.onState?.(
				outcome.type === "done"
					? { type: "attending-ended", ended: outcome.result }
					: outcome,
			);
		},
	});
}
