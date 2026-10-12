// Which of several versions of a video (the tracks of one alternate group
// of a catalog) a viewer plays: a lower one when the picture comes late,
// a higher one again when it has come in time for a while. No I/O and no
// clock: the caller says how late the picture is, and when.
//
// **A trial.** Every value is provisional and from no measurement but the
// record that came with it.

/** The picture is late by more than this, in milliseconds: the next lower version is taken. */
export const DEFAULT_DOWN_LATE_MS = 300;
/** The picture is in time when it is late by less than this. */
export const DEFAULT_UP_LATE_MS = 100;
/** How long the picture has to be in time before the next higher version is tried. */
export const DEFAULT_UP_AFTER_MS = 5000;
/** How long after a change nothing is changed: the frames of the version before are still on their way. */
export const DEFAULT_HOLD_MS = 2000;
/** The most the wait before trying higher grows to (`createQualitySwitch`). */
export const MAX_UP_AFTER_FACTOR = 8;
/** A try that has brought no picture after this many times `tryMs` has failed. */
export const TRY_WITHOUT_A_PICTURE_FACTOR = 3;

export interface QualitySwitchOptions {
	/** How many versions there are; 0 is the best, and `levels - 1` the one that takes the least room. */
	levels: number;
	/** Where to begin; 0 when left out. */
	level?: number;
	downLateMs?: number;
	upLateMs?: number;
	upAfterMs?: number;
	holdMs?: number;
	/**
	 * **A trial; off when left out.** A higher version is not played at
	 * once: it is tried for this long, in milliseconds, while the version
	 * before goes on being played (`trying`), and played only when its
	 * picture came in time all the while. WHY: that a higher version fits
	 * the link is only found by taking it, and taking it on a link without
	 * room has the picture fall behind and stand still twice, going and
	 * coming back.
	 */
	tryMs?: number;
}

export interface QualitySwitch {
	/**
	 * Takes how late the picture is now (how long after the fastest frame
	 * the frames of the last while came, in milliseconds; `undefined` where
	 * none came at all), and says which version to play. `now` in
	 * milliseconds, of a clock that only goes on. Called at a steady pace,
	 * a few times a second.
	 */
	tick(now: number, lateMs: number | undefined, triedLateMs?: number): number;
	level(): number;
	/**
	 * With `tryMs`: the version the caller is to receive next to the one it
	 * plays, without playing it, and to tell `tick` the lateness of
	 * (`triedLateMs`, `undefined` while no picture of it came); `undefined`
	 * when none is tried. When the try has held, `tick` says that version;
	 * when it has not, this is `undefined` again and `tick` says the
	 * version as before.
	 */
	trying(): number | undefined;
}

/**
 * - **Down** at once when the picture is late by more than `downLateMs`,
 *   or no frame comes: one version lower.
 * - **Up** when it has been late by less than `upLateMs` for `upAfterMs`
 *   without a break: one version higher.
 * - After a change nothing changes for `holdMs`.
 * - **A try that fails makes the next one wait longer**: when the viewer
 *   has to go down within `upAfterMs` of going up, the wait before the
 *   next try is doubled, up to `MAX_UP_AFTER_FACTOR` times; a version that
 *   holds for that long sets it back. WHY: at the lower version the
 *   picture is in time whatever room the link has above it, so that the
 *   higher one fits is only found by trying, and a link that stays narrow
 *   would have the picture stand still every few seconds.
 *
 * - With `tryMs`: **up is a try first.** The higher version is received
 *   and not played. The try fails when its picture, or that of the version
 *   that is played, is late by more than `downLateMs`, and when no picture
 *   of it came after `TRY_WITHOUT_A_PICTURE_FACTOR` times `tryMs`; it
 *   holds when `tryMs` have passed since its first picture without that.
 *   WHY `downLateMs` and not `upLateMs`: both versions are on the link
 *   while the try runs, which is more than the higher one alone will be.
 *   **A try that fails does not make the next one wait longer**: nobody
 *   saw it, so it costs little to come again, and a wait that doubled
 *   would keep the viewer low long after the link has room.
 *
 * Throws a `RangeError` for `levels` that is no whole number from 1 and a
 * `level` outside of them.
 */
export function createQualitySwitch(
	options: QualitySwitchOptions,
): QualitySwitch {
	const { levels } = options;
	if (!Number.isInteger(levels) || levels < 1) {
		throw new RangeError(`levels is a whole number from 1, not ${levels}`);
	}
	let level = options.level ?? 0;
	if (!Number.isInteger(level) || level < 0 || level >= levels) {
		throw new RangeError(`level is from 0 to ${levels - 1}, not ${level}`);
	}
	const downLateMs = options.downLateMs ?? DEFAULT_DOWN_LATE_MS;
	const upLateMs = options.upLateMs ?? DEFAULT_UP_LATE_MS;
	const upAfterMs = options.upAfterMs ?? DEFAULT_UP_AFTER_MS;
	const holdMs = options.holdMs ?? DEFAULT_HOLD_MS;
	/** When the version was last changed, and whether that was upwards. */
	let changed: { at: number; up: boolean } | undefined;
	/** Since when the picture has been in time without a break. */
	let inTimeSince: number | undefined;
	/** How many times `upAfterMs` is waited before trying higher. */
	let factor = 1;
	/** The try that is under way: since when, and since when its picture comes. */
	let trial: { since: number; pictureSince: number | undefined } | undefined;
	const tryMs = options.tryMs ?? 0;
	return {
		level: () => level,
		trying: () => (trial === undefined ? undefined : level - 1),
		tick(now, lateMs, triedLateMs) {
			const late = lateMs ?? Number.POSITIVE_INFINITY;
			if (trial !== undefined) {
				if (triedLateMs !== undefined) trial.pictureSince ??= now;
				const failed =
					late > downLateMs ||
					(triedLateMs ?? 0) > downLateMs ||
					(trial.pictureSince === undefined &&
						now - trial.since >= tryMs * TRY_WITHOUT_A_PICTURE_FACTOR);
				if (failed) {
					trial = undefined;
					// As after a change: what the try put on the link is still
					// on its way.
					changed = { at: now, up: false };
					inTimeSince = undefined;
				} else if (
					trial.pictureSince !== undefined &&
					now - trial.pictureSince >= tryMs
				) {
					trial = undefined;
					level -= 1;
					changed = { at: now, up: true };
					inTimeSince = undefined;
				}
				return level;
			}
			if (late >= upLateMs) inTimeSince = undefined;
			else inTimeSince ??= now;
			if (changed !== undefined && now - changed.at < holdMs) return level;
			if (late > downLateMs && level < levels - 1) {
				if (changed?.up && now - changed.at < upAfterMs) {
					factor = Math.min(MAX_UP_AFTER_FACTOR, factor * 2);
				}
				level += 1;
				changed = { at: now, up: false };
				inTimeSince = undefined;
				return level;
			}
			if (changed?.up && now - changed.at >= upAfterMs * factor) factor = 1;
			if (
				level > 0 &&
				inTimeSince !== undefined &&
				now - inTimeSince >= upAfterMs * factor
			) {
				if (options.tryMs !== undefined) {
					trial = { since: now, pictureSince: undefined };
					return level;
				}
				level -= 1;
				changed = { at: now, up: true };
				inTimeSince = undefined;
			}
			return level;
		},
	};
}
