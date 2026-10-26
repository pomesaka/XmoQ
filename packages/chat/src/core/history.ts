// What a participant asks for of the room's lines that were said before:
// the Joining FETCH of `lines` (§10.12.2), when it comes and each time its
// subscription is made again.

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

/**
 * The first Group of the Absolute Joining FETCH (§10.12.2.1) for the newest
 * `lines` lines, a line a Group, the largest Group that the SUBSCRIBE_OK
 * said (LARGEST_OBJECT) among them: that Group less `lines - 1`, and 0 where
 * there are fewer.
 *
 * WHY Absolute and not Relative Joining with a Joining Start of 15: the
 * relay of moqtail (relay@0.15.0) refuses a Relative Joining FETCH whose
 * Joining Start is larger than the largest Group of the track, where the own
 * relay counts from Group 0; §10.12.2.1 says nothing of that case (the
 * implementation notes, "Other observations", on moqtail). The terminal
 * viewer asks the same way (`fetch_start` of `crates/watch`), so that the two
 * clients read the draft alike.
 */
export function historyStart(
	largestGroup: bigint,
	lines: number = HISTORY_LINES,
): bigint {
	const back = BigInt(lines - 1);
	return largestGroup > back ? largestGroup - back : 0n;
}

/** Lines that were not fetched: those after `after` and before `before`, the first line fetched. */
export interface LinesGap {
	after: bigint;
	before: bigint;
}

/** What to ask for of the lines before a subscription to `lines`. */
export interface HistoryPlan {
	/**
	 * Whether the numbers had are to be forgotten: the room was numbered
	 * anew, and a number had names another line now.
	 */
	forget: boolean;
	/** The Joining FETCH, from the Group `start` to the Joining Location; undefined where nothing is to be fetched. */
	fetch: { start: bigint; gap: LinesGap | undefined } | undefined;
}

/**
 * What to fetch for a subscription to `lines` whose SUBSCRIBE_OK said
 * `largest` (the Group of LARGEST_OBJECT, undefined where it had none),
 * where the participant had lines up to `had` (undefined where it had
 * none), asking for `catchUp` of the lines it missed at the most.
 *
 * - Nothing had: the newest `HISTORY_LINES`.
 * - Lines had: from the line after `had` to `largest`; where they are more
 *   than `catchUp`, the newest `catchUp` and a gap before them. Nothing
 *   where `had` is `largest` already: an Absolute Joining FETCH starting
 *   above the Largest Object is refused with INVALID_RANGE (§10.12.3).
 * - `largest` below `had`, or none where lines were had: the room was
 *   numbered anew, and it is as if nothing was had (`forget`). WHY: the
 *   gatherer numbers a room on from what its store holds, and keeps every
 *   line (`crates/chat/README.md`), so its Groups only go up, as §2.4 has a
 *   publisher that resumes a track do. A smaller Largest comes only of a
 *   gatherer with another store, whose numbers name other lines; what was
 *   had says nothing of which of them were missed. WHY NOT go on with what
 *   was had: the new lines with numbers under it would be passed over as
 *   had, and the room would look silent until it passed the old number.
 *   A new store that has come up to the very number had is not told apart.
 */
export function historyToFetch(
	had: bigint | undefined,
	largest: bigint | undefined,
	catchUp: number,
): HistoryPlan {
	if (had === undefined || largest === undefined || largest < had) {
		return {
			forget: had !== undefined,
			fetch:
				largest === undefined
					? undefined
					: { start: historyStart(largest), gap: undefined },
		};
	}
	if (largest === had) return { forget: false, fetch: undefined };
	const newest = historyStart(largest, catchUp);
	if (newest <= had + 1n) {
		return { forget: false, fetch: { start: had + 1n, gap: undefined } };
	}
	return {
		forget: false,
		fetch: { start: newest, gap: { after: had, before: newest } },
	};
}
