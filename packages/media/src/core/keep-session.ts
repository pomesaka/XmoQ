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
