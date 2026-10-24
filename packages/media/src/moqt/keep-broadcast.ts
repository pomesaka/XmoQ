// A publisher and a viewer that go on over more than one session: when the
// session ends (the relay went, the link broke), the next is made after a
// pause and the tracks are published or played anew. The loop is
// `keepSession` of `src/core`; here it meets a connection of @xmoq/moqt.

import {
	DEFAULT_ANSWER_WAIT_MS,
	type GoAwayNotice,
	type MoqtConnection,
	type PeerFetch,
	type PeerSubscribe,
	RequestErrorCode,
	RequestFailedError,
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

/**
 * The GOAWAY behind a refusal with GOING_AWAY, or undefined where `error`
 * is no such refusal or no GOAWAY comes. Transport §10.4: requests "that
 * arrive after the GOAWAY, MUST be rejected with REQUEST_ERROR using error
 * code GOING_AWAY"; the GOAWAY travels on the control stream and the refusal
 * on the request's, between which QUIC keeps no order, so the refusal can be
 * read first. WHY a limit, the time an answer gets (`DEFAULT_ANSWER_WAIT_MS`
 * of `@xmoq/moqt`): GOING_AWAY also comes from a peer that received a GOAWAY
 * (§10.6.2) and sends none. The Rust clients wait in the same way.
 */
async function goAwayBehind(
	connection: MoqtConnection,
	error: unknown,
): Promise<GoAwayNotice | undefined> {
	if (
		!(error instanceof RequestFailedError) ||
		error.errorCode !== RequestErrorCode.GOING_AWAY
	) {
		return undefined;
	}
	const given = new AbortController();
	try {
		return await Promise.race([
			connection.goAway,
			connection.closed.then(() => undefined),
			waitFor(DEFAULT_ANSWER_WAIT_MS, given.signal).then(() => undefined),
		]);
	} finally {
		given.abort();
	}
}

/** Where a publisher that is kept stands. */
export type PublishingState =
	| { type: "connecting"; attempt: number }
	/** The namespace is announced on the `session`-th session that was made. */
	| { type: "publishing"; session: number; connection: MoqtConnection }
	| { type: "session-ended"; close: SessionClose }
	/** The relay sent GOAWAY: as for `keepPlaying` (`PlayingState`). */
	| { type: "moved"; goAway: GoAwayNotice }
	/** No session could be made, or the namespace was not taken. */
	| { type: "failed"; error: unknown };

export interface KeepPublishingOptions {
	/** Makes a connection; called anew for every session, with the GOAWAY that ended the one before (as `KeepPlayingOptions.connect`). */
	connect(goAway?: GoAwayNotice): Promise<MoqtConnection>;
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
	type Over =
		| { type: "session-ended"; close: SessionClose }
		| { type: "moved"; goAway: GoAwayNotice };
	/** The GOAWAY that ended the session before, for the next `connect`. */
	let movedBy: GoAwayNotice | undefined;
	return keepSession<Over>({
		async run() {
			const goAway = movedBy;
			movedBy = undefined;
			const connection = await options.connect(goAway);
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
				// As `keepPlaying`: a GOAWAY ends the session with NO_ERROR,
				// also while the namespace waits for its answer. WHY NOT
				// announce on the new session first and stop here later, as
				// transport §9.5.1 has a publisher do: one publisher in two
				// sessions is two sources of its tracks at the relay.
				const moved = connection.goAway.then(
					(notice): Over => ({ type: "moved", goAway: notice }),
				);
				const announced = await Promise.race([
					connection.publishNamespace(options.trackNamespace),
					moved,
				]);
				if (announced !== undefined) return announced;
				sessions += 1;
				options.onState?.({
					type: "publishing",
					session: sessions,
					connection,
				});
				return await Promise.race([
					connection.closed.then(
						(close): Over => ({ type: "session-ended", close }),
					),
					moved,
				]);
			} catch (error) {
				const notice = await goAwayBehind(connection, error);
				if (notice === undefined) throw error;
				return { type: "moved", goAway: notice };
			} finally {
				signal.removeEventListener("abort", stop);
				// Also a session on which nothing could be announced is not left
				// open. NO_ERROR, also after a GOAWAY (§3.6).
				connection.close();
			}
		},
		wait: (ms) => waitFor(movedBy === undefined ? ms : 0, signal),
		stopped: () => signal.aborted,
		...(options.delayMs === undefined ? {} : { delayMs: options.delayMs }),
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			if (outcome.type === "ended" && outcome.result.type === "moved") {
				movedBy = outcome.result.goAway;
			}
			options.onState?.(
				outcome.type === "ended"
					? outcome.result
					: { type: "failed", error: outcome.error },
			);
		},
	});
}

/** Where a track kept on one session stands (`keepTrackOnSession`). */
export type KeptTrackState<E> =
	/** It plays, for the `times`-th time on the session. */
	| { type: "playing"; times: number }
	/** Its subscription ended while the session goes on; it is asked for again after the pause. */
	| { type: "ended"; end: E }
	/** It was refused, and is asked for again after the pause, or the Retry Interval of the refusal. */
	| { type: "refused"; error: unknown }
	/** `play` failed in another way: it is not asked for again on this session. */
	| { type: "failed"; error: unknown };

export interface KeepTrackOnSessionOptions<E> {
	/** The session; what is done here ends with it. */
	connection: MoqtConnection;
	/** Subscribes to the track anew (`playAudioTrack`), as `KeepPlayingOptions.play`. */
	play(): Promise<{ ended: Promise<E> }>;
	/** `DEFAULT_ASK_AGAIN_DELAY_MS` when left out. */
	againMs?: number;
	/** Aborting it ends the asking; the subscription that plays is the caller's to stop. */
	signal?: AbortSignal;
	onState?(state: KeptTrackState<E>): void;
}

/**
 * Plays one track on one session for as long as the session lasts: when its
 * subscription ends, or a SUBSCRIBE for it is refused (a REQUEST_ERROR), it
 * is asked for again on the same session, by the rules of `stay` on
 * `keepPlaying` (`stayOnSession`): after `againMs`, or after the Retry
 * Interval of the refusal (transport §10.6.2). Resolves when the session
 * ends, the caller stops, or `play` fails in another way; never rejects.
 *
 * For the sound beside the video: `keepPlaying` follows the `ended` that
 * `play` gives it, which is the video's, and a relay may end the
 * subscription of the sound alone while the video goes on. WHY NOT a new
 * session then, as for the video: the picture would stop for the sake of
 * the sound.
 */
export async function keepTrackOnSession<E>(
	options: KeepTrackOnSessionOptions<E>,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	try {
		await stayOnSession<E, SessionClose>({
			play: options.play,
			closed: options.connection.closed,
			notYet: (error) => error instanceof RequestFailedError,
			againAfter: (error) =>
				error instanceof RequestFailedError ? error.retryWaitMs : undefined,
			wait: (ms) => waitFor(ms, signal),
			stopped: () => signal.aborted,
			...(options.againMs === undefined ? {} : { againMs: options.againMs }),
			onPlaying: (times) => options.onState?.({ type: "playing", times }),
			onNotYet: (error) => options.onState?.({ type: "refused", error }),
			onEnded: (end) => options.onState?.({ type: "ended", end }),
		});
	} catch (error) {
		options.onState?.({ type: "failed", error });
	}
}

/** Where a viewer that is kept stands. `E` is what `play` says of its end. */
export type PlayingState<E> =
	| { type: "connecting"; attempt: number }
	/** `connect` gave no connection: nothing to play now. */
	| { type: "nothing-to-play" }
	/** The tracks are played on the `session`-th session that was made. */
	| { type: "playing"; session: number; connection: MoqtConnection }
	| { type: "session-ended"; close: SessionClose }
	/**
	 * The relay sent GOAWAY (transport §10.4): the session was closed with
	 * NO_ERROR, and the next is made at once, with `goAway` for `connect`.
	 */
	| { type: "moved"; goAway: GoAwayNotice }
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
	 * to connect to now: it is asked again after the pause. `goAway` is the
	 * GOAWAY that ended the session before: transport §10.4 has "The client
	 * MUST use this URI for the new session if provided" (`newSessionUri`,
	 * empty where the current one is reused).
	 */
	connect(goAway?: GoAwayNotice): Promise<MoqtConnection | undefined>;
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
	 * (`DEFAULT_ASK_AGAIN_DELAY_MS`), or, where "not yet" is a
	 * REQUEST_ERROR that says when to ask again (its Retry Interval,
	 * transport §10.6.2), after that long. For a viewer whose requests a relay
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
 *
 * A GOAWAY of the relay (transport §3.6, §10.4) ends the session as well,
 * with NO_ERROR, and the next is made without the pause. WHY the
 * subscriptions end before the new session is made, where §3.6 has the
 * client make it "in the background" and close the old one when no
 * subscription is left on it: two sessions would play the same tracks at
 * once into one playout, and the gap is the time of one connection. WHY NOT
 * wait for the relay to close: it may wait for as long as its Timeout says.
 */
export function keepPlaying<E>(options: KeepPlayingOptions<E>): Promise<void> {
	let sessions = 0;
	const signal = options.signal ?? new AbortController().signal;
	type Over =
		| { type: "nothing-to-play" }
		| { type: "session-ended"; close: SessionClose }
		| { type: "moved"; goAway: GoAwayNotice }
		| { type: "playing-ended"; end: E };
	/** The GOAWAY that ended the session before, for the next `connect`. */
	let movedBy: GoAwayNotice | undefined;
	return keepSession<Over>({
		async run() {
			const goAway = movedBy;
			movedBy = undefined;
			const connection = await options.connect(goAway);
			if (connection === undefined) return { type: "nothing-to-play" };
			const moved = connection.goAway.then(
				(notice): Over => ({ type: "moved", goAway: notice }),
			);
			const gone: Promise<Over> = Promise.race([
				connection.closed.then(
					(close): Over => ({ type: "session-ended", close }),
				),
				moved,
			]);
			const stop = (): void => connection.close();
			signal.addEventListener("abort", stop, { once: true });
			try {
				if (signal.aborted) stop();
				const { stay } = options;
				if (stay !== undefined) {
					const session = sessions + 1;
					// Raced with the GOAWAY: `play` may wait for an answer the
					// relay that goes away does not give. Closing the session
					// below ends what is left of it.
					const over = await Promise.race([
						moved.then((close) => ({ type: "session-ended" as const, close })),
						stayOnSession<E, Over>({
							play: () => options.play(connection, signal),
							closed: gone,
							notYet: stay.notYet,
							againAfter: (error) =>
								error instanceof RequestFailedError
									? error.retryWaitMs
									: undefined,
							wait: (ms) => waitFor(ms, signal),
							stopped: () => signal.aborted,
							...(stay.againMs === undefined ? {} : { againMs: stay.againMs }),
							onPlaying() {
								sessions = session;
								options.onState?.({ type: "playing", session, connection });
							},
							onNotYet: (error) =>
								options.onState?.({ type: "not-yet", error }),
							onEnded: (end) =>
								options.onState?.({ type: "playing-ended", end }),
						}),
					]);
					// The caller stopped: its signal closes the session.
					return over.type === "session-ended" ? over.close : await gone;
				}
				const started = await Promise.race([
					options.play(connection, signal),
					moved,
				]);
				if (!("ended" in started)) return started;
				const { ended } = started;
				sessions += 1;
				options.onState?.({ type: "playing", session: sessions, connection });
				return await Promise.race([
					gone,
					ended.then((end): Over => ({ type: "playing-ended", end })),
				]);
			} catch (error) {
				const notice = await goAwayBehind(connection, error);
				if (notice === undefined) throw error;
				return { type: "moved", goAway: notice };
			} finally {
				signal.removeEventListener("abort", stop);
				// Also a session on which nothing could be played is not left open.
				// NO_ERROR, also after a GOAWAY (§3.6).
				connection.close();
			}
		},
		wait: (ms) => waitFor(movedBy === undefined ? ms : 0, signal),
		stopped: () => signal.aborted,
		...(options.delayMs === undefined ? {} : { delayMs: options.delayMs }),
		onAttempt: (attempt) => options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			if (outcome.type === "ended" && outcome.result.type === "moved") {
				movedBy = outcome.result.goAway;
			}
			options.onState?.(
				outcome.type === "ended"
					? outcome.result
					: { type: "failed", error: outcome.error },
			);
		},
	});
}
