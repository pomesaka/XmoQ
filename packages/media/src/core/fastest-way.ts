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
/**
 * How far back the fastest way is looked for, in milliseconds: it is the
 * fastest of the Objects that came in this second of the caller's clock
 * and the 59 before it. **Provisional**; `crates/watch` has the same, and
 * `fixtures/playout/least.tsv` what both go by.
 *
 * WHY it is forgotten at all: the publisher's clock and the player's run
 * at speeds that differ, by 10 to 100 parts in a million between two
 * machines as a rule. A fastest way that was kept for good had a player
 * whose clock is the faster one find every Object later and later, 360 ms
 * in an hour at 100 ppm, until all of the sound was too late and stayed
 * so (docs/experiments/2026-10-13-a-publisher-and-a-viewer-whose-clocks-
 * run-at-different-speeds.md). One Object with a time too far ahead did
 * the same at once, and for good.
 *
 * WHY a minute. Shorter follows the clocks more closely: the player is
 * behind the fastest way by what the clocks moved in the window, 6 ms at
 * 100 ppm, which any buffer holds. Longer believes a bad spell later: a
 * link that is bad for longer than the window has its delay taken for the
 * way itself, and when it is good again everything comes sooner than the
 * fastest way at once, which the sound has to make up. A minute is longer
 * than the bad spells that were run and holds what real clocks do.
 *
 * WHY whole seconds and not every Object: what is kept is then 60 numbers
 * however many Objects a publisher sends.
 */
export const FASTEST_WINDOW_MS = 60_000;

export interface FastestWay {
	/**
	 * Takes how long an Object took: when it came less the time it carries,
	 * in milliseconds, of two clocks that need not agree; and when it came
	 * (`now`, in milliseconds of the caller's clock). Gives the fastest way
	 * of the last `FASTEST_WINDOW_MS`, this one counted; `anew` says that
	 * the clock began anew with it, and that what was kept by the clock
	 * before is to be let go. **Undefined for an Object that is not of the
	 * clock: it is to be left out**, and it moves nothing.
	 */
	take(
		tookMs: number,
		now: number,
	): { fastestMs: number; anew: boolean } | undefined;
}

/**
 * The smallest difference between when an Object came and the time it
 * carries, among the Objects of the last minute, is taken for the least an
 * Object needs on the way. WHY: the two clocks need not agree, and the one
 * that came fastest took at least no time.
 *
 * An Object more than `MOST_FROM_THE_FASTEST_MS` from that is not of the
 * clock, and after `ANEW_AFTER` such in a row the clock begins anew: a
 * publisher whose clock jumped is followed after that many Objects, and
 * not after a minute. Where nothing of the clock came for a minute, the
 * next Object is the clock's first.
 */
export function createFastestWay(): FastestWay {
	/** The fastest of each second in which an Object of the clock came, the oldest first. */
	let seconds: { second: number; fastestMs: number }[] = [];
	let strays = 0;
	return {
		take(tookMs, now) {
			// A time that is no number is of no clock, and is not counted.
			if (!Number.isFinite(tookMs) || !Number.isFinite(now)) return undefined;
			// A clock that was set back does not open a second before the
			// last one: the Object counts for the newest.
			const second = Math.max(
				Math.floor(now / 1000),
				seconds.at(-1)?.second ?? Number.NEGATIVE_INFINITY,
			);
			const since = second - FASTEST_WINDOW_MS / 1000;
			if (seconds[0] !== undefined && seconds[0].second <= since) {
				seconds = seconds.filter((each) => each.second > since);
			}
			let fastestMs: number | undefined;
			for (const each of seconds) {
				fastestMs = Math.min(fastestMs ?? each.fastestMs, each.fastestMs);
			}
			let anew = false;
			if (
				fastestMs !== undefined &&
				Math.abs(tookMs - fastestMs) > MOST_FROM_THE_FASTEST_MS
			) {
				strays += 1;
				if (strays < ANEW_AFTER) return undefined;
				seconds = [];
				fastestMs = undefined;
				anew = true;
			}
			strays = 0;
			const last = seconds.at(-1);
			if (last !== undefined && last.second === second) {
				last.fastestMs = Math.min(last.fastestMs, tookMs);
			} else {
				seconds.push({ second, fastestMs: tookMs });
			}
			return { fastestMs: Math.min(fastestMs ?? tookMs, tookMs), anew };
		},
	};
}
