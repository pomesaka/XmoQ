// Which wall-clock time a frame or a piece of sound is sent with, from the
// time its source gave it. No I/O.

import { createFastestWay, MOST_FROM_THE_FASTEST_MS } from "./fastest-way.ts";

/**
 * How the times of a source are laid onto the wall clock.
 *
 * - `"first"`: the wall clock is read once, when the first frame comes,
 *   and the times go up from there as the source's own do.
 * - `"least"`: the wall clock is read with every frame, and the source's
 *   times are moved by the smallest difference between the two among the
 *   frames of the last minute (`createFastestWay`, the rule a player
 *   measures its publisher by). **Behind a choice and not the default
 *   until it is measured with real devices.**
 *
 * WHY `"least"` at all: a camera and a microphone count by clocks of their
 * own. With `"first"` each track's times run at its device's speed, a
 * player has one clock for both, and the track with the slower clock comes
 * later and later against the other (60 ms in an hour between
 * `performance.now()` and `Date.now()` on one machine;
 * docs/experiments/2026-10-13-an-hour-of-the-demo.md). With `"least"` both
 * tracks follow the one wall clock over minutes, and within a minute keep
 * the spacing their source gave them.
 *
 * WHY NOT the wall clock at which each frame came, as its time: what a
 * frame waited between the device and the page (milliseconds, and not the
 * same for every frame) would be in its time, and a player would play the
 * waiting. The least of a minute leaves it out.
 *
 * WHY NOT the video's times counted from the sound's: a broadcast without
 * sound has nothing to count from.
 */
export type StampClock = "first" | "least";

export interface Stamp {
	/**
	 * Takes a frame as it is handed to the encoder: the time its source
	 * gave it, in microseconds, and the wall clock now, in milliseconds.
	 */
	take(sourceUs: number, nowMs: number): void;
	/**
	 * The time on the wall clock, in microseconds since 1970, of what the
	 * encoder gave with the source's time `sourceUs`. Undefined before the
	 * first `take`. With `"least"`, never at or before the one it gave
	 * last: where the wall clock is set back the times stand still, a
	 * microsecond apart, until the source's have caught up (a decoder is
	 * given them in the order of the frames).
	 */
	of(sourceUs: number): number | undefined;
}

export function createStamp(clock: StampClock): Stamp {
	const way = createFastestWay();
	/** What is added to a time of the source, in microseconds. */
	let add: number | undefined;
	let last: number | undefined;
	return {
		take(sourceUs, nowMs) {
			if (clock === "first") {
				add ??= nowMs * 1000 - sourceUs;
				return;
			}
			// A frame that is not of the clock (the source's time jumped by
			// more than the rule allows) moves nothing; `createFastestWay`
			// begins anew by itself where they go on.
			const least = way.take(nowMs - sourceUs / 1000, nowMs);
			if (least !== undefined) add = least.fastestMs * 1000;
		},
		of(sourceUs) {
			if (add === undefined) return undefined;
			if (clock === "first") return sourceUs + add;
			const wall = sourceUs + add;
			// Held to the last one only where it is a little before it. WHY
			// not always: a frame whose time is not of the clock is as far
			// off as its source says, and the frames after it would be held
			// there for good.
			const back = last === undefined ? 0 : last - wall;
			last =
				last !== undefined &&
				back >= 0 &&
				back <= MOST_FROM_THE_FASTEST_MS * 1000
					? last + 1
					: wall;
			return last;
		},
	};
}
