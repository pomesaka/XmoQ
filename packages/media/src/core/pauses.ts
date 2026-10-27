// The time a sound stood still, put back into the times of its chunks. No
// I/O.

/**
 * The pauses of a source whose times an encoder counts by the samples it
 * was given (`createAudioTrackEncoder`): after a pause the encoder goes on
 * from where it stopped, and the source's own time from where it is.
 * `resumed` notes how far the two are apart when the source comes back,
 * and `shift` says what is added to a counted time from then on.
 *
 * WHY: a player lays the times of a track onto its clock by the least of
 * how late the Objects came (`createFastestWay`). Sound that came back
 * after a pause with times that did not move on would be as late as the
 * pause was long, and not be played (too late) until the player's least
 * of a minute had let go of the times before; with the pause in its times,
 * it is played when it is due, after a silence.
 */
export function createPauses(): {
	/** The source came back: its first sample, at `sourceUs` of its clock, was to be at `countedUs`. */
	resumed(countedUs: number, sourceUs: number): void;
	/** What is added to `countedUs`; asked in the order of the times. */
	shift(countedUs: number): number;
} {
	/** From which counted time on how much is added, in the order of the times. */
	const shifts: { fromUs: number; addUs: number }[] = [];
	let total = 0;
	return {
		resumed(countedUs, sourceUs) {
			const stood = sourceUs - (countedUs + total);
			// A source whose time did not move on as far as its samples (one
			// that runs slow) stood still for no time.
			if (stood <= 0) return;
			total += stood;
			shifts.push({ fromUs: countedUs, addUs: total });
		},
		shift(countedUs) {
			while (
				shifts.length > 1 &&
				(shifts[1]?.fromUs ?? Number.POSITIVE_INFINITY) <= countedUs
			) {
				shifts.shift();
			}
			const first = shifts[0];
			if (first === undefined || first.fromUs > countedUs) {
				// Before the first pause, or where the times before it are
				// still asked for.
				return 0;
			}
			return first.addUs;
		},
	};
}
