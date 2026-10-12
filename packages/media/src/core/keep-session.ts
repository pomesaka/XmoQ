// One session after the other: when a session ends, or cannot be made,
// the next is tried after a pause, until the caller says stop. No I/O and
// no timer: the caller hands in how to wait.

/** **Provisional**: what the demo had. Long enough that a relay that is gone is not asked many times a second, and short for a person who waits. */
export const DEFAULT_RECONNECT_DELAY_MS = 2000;

export type SessionOutcome<R> =
	/** The session was made, and ended; `result` is what `run` resolved with. */
	| { type: "ended"; attempt: number; result: R }
	/** `run` rejected: the session could not be made, or broke with an error. */
	| { type: "failed"; attempt: number; error: unknown };

export interface KeepSessionOptions<R> {
	/**
	 * Makes a session and resolves when it is over. `attempt` counts from 1.
	 * A caller that stops ends the session itself.
	 */
	run(attempt: number): Promise<R>;
	/** Resolves after `ms` milliseconds, or sooner where the caller stopped. */
	wait(ms: number): Promise<void>;
	/** `DEFAULT_RECONNECT_DELAY_MS` when left out. The same after every session: nothing backs off. */
	delayMs?: number;
	/** Asked after every session and every pause: true ends the trying. WHY no `AbortSignal`: this file is checked without the types of a browser. */
	stopped?(): boolean;
	onAttempt?(attempt: number): void;
	onOutcome?(outcome: SessionOutcome<R>): void;
}

/**
 * Runs sessions one after the other, with a pause between. Resolves once
 * `stopped` says so and the session that ran then is over; never rejects.
 * Without `stopped` it does not resolve.
 *
 * WHY the same pause after a session that ended and one that failed: a
 * session ends when its relay goes, and the next one fails until the relay
 * is back; the two are one break.
 */
export async function keepSession<R>(
	options: KeepSessionOptions<R>,
): Promise<void> {
	const delayMs = options.delayMs ?? DEFAULT_RECONNECT_DELAY_MS;
	const stopped = options.stopped ?? (() => false);
	for (let attempt = 1; !stopped(); attempt++) {
		options.onAttempt?.(attempt);
		let outcome: SessionOutcome<R>;
		try {
			outcome = {
				type: "ended",
				attempt,
				result: await options.run(attempt),
			};
		} catch (error) {
			outcome = { type: "failed", attempt, error };
		}
		options.onOutcome?.(outcome);
		if (stopped()) return;
		await options.wait(delayMs).catch(() => {});
	}
}

/**
 * **Provisional**: how long after a "not yet", and after what was played
 * ended, the same is asked again on the session. Long enough that a relay
 * which refuses at once is not asked many times a second; a relay that
 * holds the request (RENDEZVOUS_TIMEOUT) has had its own wait before.
 */
export const DEFAULT_ASK_AGAIN_DELAY_MS = 1000;

/**
 * **Provisional**: the shortest pause after a "not yet" that names its own
 * (`StayOptions.againAfter`). WHY any: a peer that says "at once" and then
 * refuses at once would be asked in a loop without a pause.
 */
export const MIN_ASK_AGAIN_DELAY_MS = 100;

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
	/** `DEFAULT_ASK_AGAIN_DELAY_MS` when left out. */
	againMs?: number;
	/**
	 * The pause a "not yet" names itself, in milliseconds, where it names
	 * one: a relay says with its refusal when to ask again (the Retry
	 * Interval of a REQUEST_ERROR, transport §10.6.2). Used in place of
	 * `againMs` for that one pause, and no shorter than
	 * `MIN_ASK_AGAIN_DELAY_MS`. `undefined` where it names none; `againMs`
	 * then.
	 *
	 * WHY asked again where the refusal says "do not retry" (a Retry
	 * Interval of 0): whether to ask again is `notYet`'s to say. A relay
	 * that gives every refusal a 0 (moqtail's does) would otherwise have no
	 * viewer wait at it.
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
	const againMs = options.againMs ?? DEFAULT_ASK_AGAIN_DELAY_MS;
	const over = options.closed.then((close) => ({
		type: "session-ended" as const,
		close,
	}));
	/** The pause, cut short by the end of the session. */
	const pause = async (
		ms: number = againMs,
	): Promise<Awaited<typeof over> | undefined> =>
		Promise.race([over, options.wait(ms).then(() => undefined)]);
	for (let times = 1; !options.stopped(); ) {
		let ended: Promise<E>;
		try {
			({ ended } = await options.play());
		} catch (error) {
			if (options.stopped()) break;
			if (!options.notYet(error)) throw error;
			options.onNotYet?.(error);
			const named = options.againAfter?.(error);
			const closed = await pause(
				named === undefined ? againMs : Math.max(named, MIN_ASK_AGAIN_DELAY_MS),
			);
			if (closed !== undefined) return closed;
			continue;
		}
		options.onPlaying?.(times);
		times += 1;
		const first = await Promise.race([
			over,
			ended.then((end) => ({ type: "playing-ended" as const, end })),
		]);
		if (first.type === "session-ended") return first;
		if (options.stopped()) break;
		options.onEnded?.(first.end);
		const closed = await pause();
		if (closed !== undefined) return closed;
	}
	return { type: "stopped" };
}
