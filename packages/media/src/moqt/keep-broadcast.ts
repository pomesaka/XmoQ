// A publisher and a viewer that go on over more than one session: when the
// session ends (the relay went, the link broke), the next is made after a
// pause and the tracks are published or played anew. The loop is
// `keepMoving` of @xmoq/moqt, which follows GOAWAY.

import {
	type GoAwayNotice,
	keepSessions,
	type MoqtConnection,
	type MovingOutcome,
	type PeerFetch,
	type PeerSubscribe,
	RequestErrorCode,
	RequestFailedError,
	type SessionClose,
	sessionMovingOutcome,
	waitFor,
} from "@xmoq/moqt";
import { stayOnSession } from "./stay-on-session.ts";

/** A track that answers the requests for it: a publisher of this package. */
export interface ServedTrack {
	/** Whether the SUBSCRIBE was for this track, and is answered. */
	subscribe(request: PeerSubscribe): boolean;
	fetch(request: PeerFetch): boolean;
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
	/** Makes a connection; called anew for every session, with the last GOAWAY that ended one (as `KeepPlayingOptions.connect`). */
	connect(goAway?: GoAwayNotice): Promise<MoqtConnection>;
	trackNamespace: Uint8Array[];
	/** The tracks of the namespace. **They go on between sessions**: the encoders are the caller's and are not stopped here. */
	tracks: readonly ServedTrack[];
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` when left out (`KeepMovingOptions.againMs`). */
	againMs?: number;
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
	return keepSessions<{ connection: MoqtConnection }, SessionClose>({
		connect: async (goAway) => ({ connection: await options.connect(goAway) }),
		async run({ connection }, _signal, began) {
			connection.serve({
				subscribe(request) {
					if (options.tracks.some((track) => track.subscribe(request))) return;
					request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				},
				fetch(request) {
					if (options.tracks.some((track) => track.fetch(request))) return;
					request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				},
			});
			// A GOAWAY while the namespace waits for its answer ends the
			// session as well (`keepMoving`). WHY NOT announce on the new
			// session first and stop here later, as transport §9.5.1 has a
			// publisher do: one publisher in two sessions is two sources of
			// its tracks at the relay.
			await connection.publishNamespace(options.trackNamespace);
			options.onState?.({ type: "publishing", session: began(), connection });
			return connection.closed;
		},
		...(options.againMs === undefined ? {} : { againMs: options.againMs }),
		...(options.signal === undefined ? {} : { signal: options.signal }),
		onConnecting: (attempt) =>
			options.onState?.({ type: "connecting", attempt }),
		onOutcome: (outcome) => options.onState?.(sessionMovingOutcome(outcome)),
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
	 * last GOAWAY that ended a session: transport §10.4 has "The client MUST
	 * use this URI for the new session if provided" (`newSessionUri`, empty
	 * where the current one is reused). **It stays the same for every try
	 * until another GOAWAY comes**, also after a session or a try on the new
	 * URI failed. WHY: the relay that sent it is going away and refuses what
	 * comes (`GOING_AWAY`); a viewer that went back to it after one failure
	 * at the new URI was left there (an interop run, 2026-10-24).
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
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` when left out (`KeepMovingOptions.againMs`). */
	againMs?: number;
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
	/**
	 * The GOAWAY that ended what was played, where it was one: a GOAWAY on the
	 * stream of a subscription (transport §10.4; `playVideoTrack` ends with
	 * `moved`). The session is then left as at a GOAWAY on the control
	 * stream, and `connect` is handed it. WHY a new session also where it
	 * names no URI, which §10.4 has sent again on "the current session": the
	 * tracks of one broadcast are played from one session, by one clock.
	 */
	movedBy?(end: E): GoAwayNotice | undefined;
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
	type Over =
		| { type: "session-ended"; close: SessionClose }
		| { type: "playing-ended"; end: E }
		/** What was played ended for a GOAWAY on its stream. */
		| { type: "track-moved"; goAway: GoAwayNotice };
	return keepSessions<{ connection: MoqtConnection }, Over>({
		async connect(goAway) {
			const connection = await options.connect(goAway);
			if (connection === undefined) throw new NothingToPlay();
			return { connection };
		},
		async run({ connection }, signal, began) {
			/** Resolved when what was played ended for a GOAWAY on its stream. */
			let trackMoved: (notice: GoAwayNotice) => void = () => {};
			const moved = new Promise<GoAwayNotice>((resolve) => {
				trackMoved = resolve;
			});
			const play = async (): Promise<{ ended: Promise<E> }> => {
				const { ended } = await options.play(connection, signal);
				return {
					ended: ended.then((end) => {
						const notice = options.movedBy?.(end);
						if (notice !== undefined) trackMoved(notice);
						return end;
					}),
				};
			};
			const gone: Promise<Over> = Promise.race([
				connection.closed.then(
					(close): Over => ({ type: "session-ended", close }),
				),
				moved.then((goAway): Over => ({ type: "track-moved", goAway })),
			]);
			const { stay } = options;
			if (stay !== undefined) {
				const over = await stayOnSession<E, Over>({
					play,
					closed: gone,
					notYet: stay.notYet,
					againAfter: (error) =>
						error instanceof RequestFailedError ? error.retryWaitMs : undefined,
					wait: (ms) => waitFor(ms, signal),
					stopped: () => signal.aborted,
					...(stay.againMs === undefined ? {} : { againMs: stay.againMs }),
					onPlaying() {
						options.onState?.({
							type: "playing",
							session: began(),
							connection,
						});
					},
					onNotYet: (error) => options.onState?.({ type: "not-yet", error }),
					onEnded: (end) => options.onState?.({ type: "playing-ended", end }),
				});
				// The caller stopped: its signal closes the session.
				return over.type === "session-ended" ? over.close : await gone;
			}
			const { ended } = await play();
			options.onState?.({ type: "playing", session: began(), connection });
			return await Promise.race([
				gone,
				ended.then((end): Over => ({ type: "playing-ended", end })),
			]);
		},
		movedBy(over) {
			switch (over.type) {
				case "track-moved":
					return over.goAway;
				case "playing-ended":
					return options.movedBy?.(over.end);
				case "session-ended":
					return undefined;
			}
		},
		...(options.againMs === undefined ? {} : { againMs: options.againMs }),
		...(options.signal === undefined ? {} : { signal: options.signal }),
		onConnecting: (attempt) =>
			options.onState?.({ type: "connecting", attempt }),
		onOutcome(outcome) {
			options.onState?.(playingStateOf(outcome));
		},
	});
}

function playingStateOf<E>(
	outcome: MovingOutcome<
		| { type: "session-ended"; close: SessionClose }
		| { type: "playing-ended"; end: E }
		| { type: "track-moved"; goAway: GoAwayNotice }
	>,
): PlayingState<E> {
	switch (outcome.type) {
		case "done":
			return outcome.result.type === "track-moved"
				? { type: "moved", goAway: outcome.result.goAway }
				: outcome.result;
		case "moved":
			return outcome;
		case "failed":
			return outcome.error instanceof NothingToPlay
				? { type: "nothing-to-play" }
				: outcome;
	}
}

/**
 * `KeepPlayingOptions.connect` gave no connection. WHY a rejection of
 * `connect`, and no outcome of `keepMoving` of its own: no other client has
 * nothing to connect to, and it is paused for as a failed try is.
 */
class NothingToPlay extends Error {
	constructor() {
		super("there is nothing to connect to now");
	}
}
