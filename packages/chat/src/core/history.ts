// How many of the room's lines that were said before a participant asks
// for: the Joining FETCH of `lines` (§10.12.2), when it comes and each time
// its subscription is made again. What it asks for is `followPlan` of
// `@xmoq/tracks`.

/**
 * How many lines of the room a participant that comes asks for: the newest
 * 16. Not a setting, so that no flag of a page makes it hold more.
 * **Provisional**: what a person reads on coming into a room, not a measured
 * number.
 */
export const HISTORY_LINES = 16;

/**
 * How many of the lines it missed a participant that had lines asks for,
 * when its subscription is made again: 200 by default, what the demo's page
 * keeps of the chat (`shownLines` of `showChat`, 200 on the pages of the demo), so that a page
 * that comes back fills what it shows and no more. Decided on 2026-10-24.
 * **Provisional** as a number.
 */
export const CATCH_UP_LINES = 200;

/**
 * The most a setting may make `CATCH_UP_LINES`: 1,000, the lines one FETCH
 * of the gatherer gets by default (`--fetch-lines`). WHY a bound that no
 * setting removes: the participant reads and remembers every line it asked
 * for (`SeenLines` is made as large), so the number is what a page holds.
 * WHY that number: of a longer range the gatherer sends the latest 1,000
 * and an End of Unknown Range indicator for the rest (§11.4.4.2), so more
 * would not be had in one FETCH anyway.
 */
export const CATCH_UP_LINES_MOST = 1_000;

/** `CATCH_UP_LINES`, or `given`; a `RangeError` for a value that is no whole number from 1 to `CATCH_UP_LINES_MOST`. */
export function catchUpLines(given: number = CATCH_UP_LINES): number {
	if (
		!Number.isSafeInteger(given) ||
		given < 1 ||
		given > CATCH_UP_LINES_MOST
	) {
		throw new RangeError(
			`catchUpLines is a whole number from 1 to ${CATCH_UP_LINES_MOST}, not ${given}`,
		);
	}
	return given;
}
