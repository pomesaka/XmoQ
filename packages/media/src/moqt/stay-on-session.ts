// Asking again on one session: where there is nothing to play yet, and
// where what was played has ended, the same is asked again after a pause.
// No I/O and no timer: the caller hands in how to wait. The pauses are the
// rule of `createAskAgain` of @xmoq/moqt, which every client asks by. WHY
// NOT in `src/core`: that orders frames without MoQ Transport.

import { createAskAgain } from "@xmoq/moqt";

export interface StayOptions<E, C> {
	/** Begins to play on the session, and resolves once it plays, with `ended` (as `KeepPlayingOptions.play`). */
	play(): Promise<{ ended: Promise<E> }>;
	/** Resolves when the session is over. */
	closed: Promise<C>;
	/** Whether a rejection of `play` only says that what is asked for is not there yet. */
	notYet(error: unknown): boolean;
	/** Resolves after `ms` milliseconds, or sooner where the caller stopped. */
	wait(ms: number): Promise<void>;
	stopped(): boolean;
	/** `DEFAULT_ASK_AGAIN_DELAY_MS` of `@xmoq/moqt` when left out. */
	againMs?: number;
	/**
	 * The pause a "not yet" names itself, in milliseconds, where it names
	 * one: a relay says with its refusal when to ask again (the Retry
	 * Interval of a REQUEST_ERROR, transport §10.6.2). Taken as
	 * `AskAgain.afterRefusal` of `@xmoq/moqt` takes it, also where it says
	 * "do not retry": whether to ask again is `notYet`'s to say.
	 * `undefined` where it names none; `againMs` then.
	 */
	againAfter?(error: unknown): number | undefined;
	/** `play` resolved: it plays, for the `times`-th time on this session. */
	onPlaying?(times: number): void;
	/** `play` said "not yet", and is asked again after the pause. */
	onNotYet?(error: unknown): void;
	/** What was played ended while the session goes on; it is asked for again after the pause. */
	onEnded?(end: E): void;
}

/**
 * Plays on one session for as long as the session lasts: where there is
 * nothing to play yet, and where what was played has ended, it asks again
 * on the same session after a pause, and does not give the session up.
 * Resolves with how the session ended, or once the caller stopped; rejects
 * with what `play` rejected with where that is no "not yet".
 *
 * WHY on the same session: a viewer that waits for a publisher has a
 * relay that can hold its request until the publisher is there, and a new
 * connection for every question is what that spares. WHY a pause all the
 * same: a relay that does not hold answers at once, and a track that has
 * just ended is refused at once for a moment by one that does.
 */
export async function stayOnSession<E, C>(
	options: StayOptions<E, C>,
): Promise<{ type: "session-ended"; close: C } | { type: "stopped" }> {
	const again = createAskAgain(
		options.againMs === undefined ? {} : { afterEndMs: options.againMs },
	);
	const over = options.closed.then((close) => ({
		type: "session-ended" as const,
		close,
	}));
	/** The pause, cut short by the end of the session. */
	const pause = async (ms: number): Promise<Awaited<typeof over> | undefined> =>
		Promise.race([over, options.wait(ms).then(() => undefined)]);
	for (let times = 1; !options.stopped(); ) {
		let ended: Promise<E>;
		try {
			({ ended } = await options.play());
		} catch (error) {
			if (options.stopped()) break;
			if (!options.notYet(error)) throw error;
			options.onNotYet?.(error);
			const closed = await pause(
				again.afterRefusal(options.againAfter?.(error)),
			);
			if (closed !== undefined) return closed;
			continue;
		}
		options.onPlaying?.(times);
		times += 1;
		again.subscribed();
		const first = await Promise.race([
			over,
			ended.then((end) => ({ type: "playing-ended" as const, end })),
		]);
		if (first.type === "session-ended") return first;
		if (options.stopped()) break;
		options.onEnded?.(first.end);
		const closed = await pause(again.afterEnd());
		if (closed !== undefined) return closed;
	}
	return { type: "stopped" };
}
