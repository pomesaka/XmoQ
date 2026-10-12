// Which Objects are of the clock a player counts by. The time an Object
// carries is the publisher's to say, and a player that believes any of
// them can be made to wait for years. No I/O.

/**
 * How far from the fastest way an Object may be, sooner or later, and
 * still be of the clock: 10 seconds. **Provisional**, and the value of the
 * viewer in a terminal (`crates/watch/src/clock.rs`).
 *
 * WHY a limit at all: an Object whose time is years ahead would be the
 * fastest way ever seen, and every Object after it years late; one whose
 * time is years behind would be held for years. Neither says anything
 * about the path. 10 seconds is far beyond what a buffer holds and what a
 * viewer waits for a live picture.
 */
export const MOST_FROM_THE_FASTEST_MS = 10_000;
/**
 * After so many Objects in a row that are not of the clock, the clock
 * begins anew with the next: the publisher's clock has moved (it began
 * again), or the first Object was the one that was wrong. **Provisional**,
 * and the value of `crates/watch`: a second of video, more or less.
 */
export const ANEW_AFTER = 30;

export interface FastestWay {
	/**
	 * Takes how long an Object took: when it came less the time it carries,
	 * in milliseconds, of two clocks that need not agree. Gives the fastest
	 * way so far, this one counted; `anew` says that the clock began anew
	 * with it, and that what was kept by the clock before is to be let go.
	 * **Undefined for an Object that is not of the clock: it is to be left
	 * out**, and it moves nothing.
	 */
	take(tookMs: number): { fastestMs: number; anew: boolean } | undefined;
}

/**
 * The smallest difference between when an Object came and the time it
 * carries is taken for the least an Object needs on the way. WHY: the two
 * clocks need not agree, and the one that came fastest took at least no
 * time.
 */
export function createFastestWay(): FastestWay {
	let fastestMs: number | undefined;
	let strays = 0;
	return {
		take(tookMs) {
			// A time that is no number is of no clock, and is not counted.
			if (!Number.isFinite(tookMs)) return undefined;
			let anew = false;
			if (
				fastestMs !== undefined &&
				Math.abs(tookMs - fastestMs) > MOST_FROM_THE_FASTEST_MS
			) {
				strays += 1;
				if (strays < ANEW_AFTER) return undefined;
				fastestMs = undefined;
				anew = true;
			}
			strays = 0;
			fastestMs = Math.min(fastestMs ?? tookMs, tookMs);
			return { fastestMs, anew };
		},
	};
}
