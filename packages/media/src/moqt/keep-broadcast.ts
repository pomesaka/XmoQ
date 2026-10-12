// A publisher and a viewer that go on over more than one session: when the
// session ends (the relay went, the link broke), the next is made after a
// pause and the tracks are published or played anew. The loop is
// `keepSession` of `src/core`; here it meets a connection of @xmoq/moqt.

import {
	type MoqtConnection,
	type PeerFetch,
	type PeerSubscribe,
	RequestErrorCode,
	type SessionClose,
} from "@xmoq/moqt";
import { keepSession, stayOnSession } from "../core/keep-session.ts";

/** A track that answers the requests for it: a publisher of this package. */
export interface ServedTrack {
	/** Whether the SUBSCRIBE was for this track, and is answered. */
	subscribe(request: PeerSubscribe): boolean;
	fetch(request: PeerFetch): boolean;
}

const waitFor = (ms: number, signal: AbortSignal): Promise<void> =>
	new Promise((resolve) => {
		if (signal.aborted) {
			resolve();
			return;
		}
		const timer = setTimeout(done, ms);
		function done(): void {
			clearTimeout(timer);
			signal.removeEventListener("abort", done);
			resolve();
		}
		signal.addEventListener("abort", done, { once: true });
	});

/** Where a publisher that is kept stands. */
export type PublishingState =
	| { type: "connecting"; attempt: number }
	/** The namespace is announced on the `session`-th session that was made. */
	| { type: "publishing"; session: number; connection: MoqtConnection }
	| { type: "session-ended"; close: SessionClose }
	/** No session could be made, or the namespace was not taken. */
	| { type: "failed"; error: unknown };

export interface KeepPublishingOptions {
	/** Makes a connection; called anew for every session. */
	connect(): Promise<MoqtConnection>;
	trackNamespace: Uint8Array[];
	/** The tracks of the namespace. **They go on between sessions**: the encoders are the caller's and are not stopped here. */
	tracks: readonly ServedTrack[];
	/** `DEFAULT_RECONNECT_DELAY_MS` when left out. */
	delayMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: PublishingState): void;
}

/**
 * Publishes a namespace for as long as `signal` lets it: connects, answers
 * the requests for its tracks, announces the namespace (PUBLISH_NAMESPACE),
 * and does so again on a new session when the one before has ended.
 * Resolves once `signal` was aborted; never rejects.
 *
 * - A request for a track that is none of `tracks` is refused with
 *   DOES_NOT_EXIST.
 * - **What the tracks were asked to send while no session was there is not
 *   sent later**: a publisher of this package writes to the subscription it
 *   has, and has none then. A viewer gets the next key frame of the new
 *   session.
 * - The Groups go on counting over the break, as the publishers count
 *   them: the relay gets a track it knew with later Groups.
 */
export function keepPublishing(options: KeepPublishingOptions): Promise<void> {
	let sessions = 0;
	const signal = options.signal ?? new AbortController().signal;
	return keepSession<SessionClose>({
		async run() {
			const connection = await options.connect();
			const stop = (): void => connection.close();
			signal.addEventListener("abort", stop, { once: true });
			try {
				if (signal.aborted) stop();
				connection.serve({
					subscribe(request) {
						if (options.tracks.some((track) => track.subscribe(request)))
							return;
						request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
					},
					fetch(request) {
						if (options.tracks.some((track) => track.fetch(request))) return;
						request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
					},
				});
				await connection.publishNamespace(options.trackNamespace);
				sessions += 1;
				options.onState?.({
					type: "publishing",
					session: sessions,
					connection,
				});
				return await connection.closed;
			} finally {
				signal.removeEventListener("abort", stop);
				// Also a session on which nothing could be announced is not left open.
				connection.close();
			}
		},
		wait: (ms) => waitFor(ms, signal),
		stopped: () => signal.aborted,
		...(options.delayMs === undefined ? {} : { delayMs: options.delayMs }),
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			options.onState?.(
				outcome.type === "ended"
					? { type: "session-ended", close: outcome.result }
					: { type: "failed", error: outcome.error },
			);
		},
	});
}

/** Where a viewer that is kept stands. `E` is what `play` says of its end. */
export type PlayingState<E> =
	| { type: "connecting"; attempt: number }
	/** `connect` gave no connection: nothing to play now. */
	| { type: "nothing-to-play" }
	/** The tracks are played on the `session`-th session that was made. */
	| { type: "playing"; session: number; connection: MoqtConnection }
	| { type: "session-ended"; close: SessionClose }
	/** What is played ended while the session went on: its publisher went. */
	| { type: "playing-ended"; end: E }
	/** With `stay`: there is nothing to play yet, and it is asked for again on the session. */
	| { type: "not-yet"; error: unknown }
	/** No session could be made, or nothing could be played on it. */
	| { type: "failed"; error: unknown };

export interface KeepPlayingOptions<E> {
	/**
	 * Makes a connection; called anew for every session, so that a caller
	 * can ask anew where the broadcast is. Undefined where there is nothing
	 * to connect to now: it is asked again after the pause.
	 */
	connect(): Promise<MoqtConnection | undefined>;
	/**
	 * Begins to play on the connection (`playVideoTrack`, `playAudioTrack`),
	 * and resolves once it plays, with `ended`, which resolves when what is
	 * played has ended. **Where it goes on to the same `createPlayout`, the
	 * picture and the sound of the next session follow those of the one
	 * before by the one clock.**
	 */
	play(
		connection: MoqtConnection,
		signal: AbortSignal,
	): Promise<{ ended: Promise<E> }>;
	/** `DEFAULT_RECONNECT_DELAY_MS` when left out. */
	delayMs?: number;
	/**
	 * **A trial; left out, nothing changes.** With it the session is kept
	 * where there is nothing to play yet (`play` rejects, and `notYet` says
	 * so of what it rejected with) and where what was played has ended:
	 * `play` is called again on the same connection after `againMs`
	 * (`DEFAULT_ASK_AGAIN_DELAY_MS`). For a viewer whose requests a relay
	 * holds until a publisher is there (RENDEZVOUS_TIMEOUT, transport
	 * §10.2.6), where "not yet" is the relay's REQUEST_ERROR `TIMEOUT`. A
	 * session that ends, and a `play` that fails in another way, are
	 * followed by a new session as without it.
	 */
	stay?: { notYet(error: unknown): boolean; againMs?: number };
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	onState?(state: PlayingState<E>): void;
}

/**
 * Plays for as long as `signal` lets it: connects, plays, and when the
 * session ends, or what is played does, closes the session and does so
 * again after the pause. Resolves once `signal` was aborted; never rejects.
 *
 * WHY a new session also where only the tracks ended: their publisher
 * went, and where it comes back is the caller's to find out (`connect`).
 */
export function keepPlaying<E>(options: KeepPlayingOptions<E>): Promise<void> {
	let sessions = 0;
	const signal = options.signal ?? new AbortController().signal;
	type Over =
		| { type: "nothing-to-play" }
		| { type: "session-ended"; close: SessionClose }
		| { type: "playing-ended"; end: E };
	return keepSession<Over>({
		async run() {
			const connection = await options.connect();
			if (connection === undefined) return { type: "nothing-to-play" };
			const stop = (): void => connection.close();
			signal.addEventListener("abort", stop, { once: true });
			try {
				if (signal.aborted) stop();
				const { stay } = options;
				if (stay !== undefined) {
					const session = sessions + 1;
					const over = await stayOnSession<E, SessionClose>({
						play: () => options.play(connection, signal),
						closed: connection.closed,
						notYet: stay.notYet,
						wait: (ms) => waitFor(ms, signal),
						stopped: () => signal.aborted,
						...(stay.againMs === undefined ? {} : { againMs: stay.againMs }),
						onPlaying() {
							sessions = session;
							options.onState?.({ type: "playing", session, connection });
						},
						onNotYet: (error) => options.onState?.({ type: "not-yet", error }),
						onEnded: (end) => options.onState?.({ type: "playing-ended", end }),
					});
					// The caller stopped: its signal closes the session.
					return over.type === "session-ended"
						? over
						: await connection.closed.then(
								(close): Over => ({ type: "session-ended", close }),
							);
				}
				const { ended } = await options.play(connection, signal);
				sessions += 1;
				options.onState?.({ type: "playing", session: sessions, connection });
				return await Promise.race([
					connection.closed.then(
						(close): Over => ({ type: "session-ended", close }),
					),
					ended.then((end): Over => ({ type: "playing-ended", end })),
				]);
			} finally {
				signal.removeEventListener("abort", stop);
				// Also a session on which nothing could be played is not left open.
				connection.close();
			}
		},
		wait: (ms) => waitFor(ms, signal),
		stopped: () => signal.aborted,
		...(options.delayMs === undefined ? {} : { delayMs: options.delayMs }),
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			options.onState?.(
				outcome.type === "ended"
					? outcome.result
					: { type: "failed", error: outcome.error },
			);
		},
	});
}
