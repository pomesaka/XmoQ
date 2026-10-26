// One track kept on one session: when its subscription ends, or a SUBSCRIBE
// for it is refused, it is asked for again on the same session.

import {
	type MoqtConnection,
	RequestFailedError,
	type SessionClose,
	waitFor,
} from "@xmoq/moqt";
import { stayOnSession } from "./stay-on-session.ts";

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
 * What a kept track asks again after, and how long it waits then, beside
 * the pauses of `createAskAgain` (`fixtures/kept-track/`): a refusal
 * (REQUEST_ERROR) is asked again after its Retry Interval, or the pause
 * where it names none. Any other failure of `play` (a SUBSCRIBE not sent,
 * or not answered, in time) gives the track up on the session, the `give-up`
 * of the table. WHY: the sound beside the video is all that keeps a track
 * this way, and the next session asks for it anyway, as the Rust viewers
 * give their sound up (`again_after_failure` of `xmoq-subscriber`). A
 * followed track (`followTrack`) asks again after every failure, the
 * `again` of the table.
 */
export const keptTrackRules = {
	notYet: (error: unknown): boolean => error instanceof RequestFailedError,
	againAfter: (error: unknown): number | undefined =>
		error instanceof RequestFailedError ? error.retryWaitMs : undefined,
};

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
			...keptTrackRules,
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
