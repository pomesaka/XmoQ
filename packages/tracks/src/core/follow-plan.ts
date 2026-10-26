// What a reader of a followed track asks for of the items that came before:
// the Joining FETCH (§10.12.2) when it subscribes, and each time its
// subscription is made again. A followed track has one numbered item a
// Group, its number the Group ID, as the chat's `lines` and a transcript's
// `speech` have. No I/O.

/**
 * The first Group of the Absolute Joining FETCH (§10.12.2.1) for the newest
 * `items` items, the largest Group that the SUBSCRIBE_OK said
 * (LARGEST_OBJECT) among them: that Group less `items - 1`, and 0 where
 * there are fewer.
 *
 * WHY Absolute and not Relative Joining with a Joining Start of `items - 1`:
 * the relay of moqtail (relay@0.15.0) refuses a Relative Joining FETCH whose
 * Joining Start is larger than the largest Group of the track, where the own
 * relay counts from Group 0; §10.12.2.1 says nothing of that case (the
 * implementation notes, "Other observations", on moqtail). The Rust readers
 * ask the same way (`past_to_fetch` of `xmoq-subscriber`), so that the two
 * read the draft alike.
 */
export function followStart(largestGroup: bigint, items: number): bigint {
	const back = BigInt(items - 1);
	return largestGroup > back ? largestGroup - back : 0n;
}

/** Items that were not fetched: those after `after` and before `before`, the first item fetched. */
export interface FollowGap {
	after: bigint;
	before: bigint;
}

/** What to ask for of the items before a subscription. */
export interface FollowPlan {
	/**
	 * Whether the numbers had are to be forgotten: the track was numbered
	 * anew, and a number had names another item now.
	 */
	forget: boolean;
	/** The Joining FETCH, from the Group `start` to the Joining Location; undefined where nothing is to be fetched. */
	fetch: { start: bigint; gap: FollowGap | undefined } | undefined;
}

/**
 * What to fetch for a subscription whose SUBSCRIBE_OK said `largest` (the
 * Group of LARGEST_OBJECT, undefined where it had none), where the reader
 * had items up to `had` (undefined where it had none): the newest `recent`
 * where it had none, and `catchUp` of those it missed at the most where it
 * had some. 0 for either fetches nothing in that case.
 *
 * - Nothing had: the newest `recent`.
 * - Items had: from the item after `had` to `largest`; where they are more
 *   than `catchUp`, the newest `catchUp` and a gap before them. Nothing
 *   where `had` is `largest` already: an Absolute Joining FETCH starting
 *   above the Largest Object is refused with INVALID_RANGE (§10.12.3).
 * - `largest` below `had`, or none where items were had: the track was
 *   numbered anew, and it is as if nothing was had (`forget`). WHY: a
 *   publisher that resumes a track numbers it on, as §2.4 has it do, so its
 *   Groups only go up; a smaller Largest comes only of one whose numbers
 *   name other items, and what was had says nothing of which of them were
 *   missed. WHY NOT go on with what was had: the new items with numbers
 *   under it would be passed over as had, and the track would look silent
 *   until it passed the old number.
 */
export function followPlan(
	had: bigint | undefined,
	largest: bigint | undefined,
	recent: number,
	catchUp: number,
): FollowPlan {
	if (had === undefined || largest === undefined || largest < had) {
		return {
			forget: had !== undefined,
			fetch:
				largest === undefined || recent === 0
					? undefined
					: { start: followStart(largest, recent), gap: undefined },
		};
	}
	if (largest === had || catchUp === 0) {
		return { forget: false, fetch: undefined };
	}
	const newest = followStart(largest, catchUp);
	if (newest <= had + 1n) {
		return { forget: false, fetch: { start: had + 1n, gap: undefined } };
	}
	return {
		forget: false,
		fetch: { start: newest, gap: { after: had, before: newest } },
	};
}
