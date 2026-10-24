// What a participant that comes asks for of the room's lines that were said
// before: the Joining FETCH of `lines` (§10.12.2).

/**
 * How many lines of the room a participant that comes asks for: the newest
 * 16. Not a setting, so that no flag of a page makes it hold more.
 * **Provisional**: what a person reads on coming into a room, not a measured
 * number.
 */
export const HISTORY_LINES = 16;

/**
 * The first Group of the Absolute Joining FETCH (§10.12.2.2) for the newest
 * `HISTORY_LINES` lines, a line a Group, the largest Group that the
 * SUBSCRIBE_OK said (LARGEST_OBJECT) among them: that Group less 15, and 0
 * where there are fewer.
 *
 * WHY Absolute and not Relative Joining with a Joining Start of 15: the
 * relay of moqtail (relay@0.15.0) refuses a Relative Joining FETCH whose
 * Joining Start is larger than the largest Group of the track, where the own
 * relay counts from Group 0; §10.12.2.1 says nothing of that case (the
 * implementation notes, "Other observations", on moqtail). The terminal
 * viewer asks the same way (`fetch_start` of `crates/watch`), so that the two
 * clients read the draft alike.
 */
export function historyStart(largestGroup: bigint): bigint {
	const back = BigInt(HISTORY_LINES - 1);
	return largestGroup > back ? largestGroup - back : 0n;
}
