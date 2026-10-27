// Where a decoded piece of audio begins on the clock of what sounds it, so
// that it joins the piece before it without a click. No I/O.

/** How far from its own time a piece may begin so as to follow the one before it without a seam, in seconds: less than half a piece of 20 ms (**provisional**). */
export const SEAM_SECONDS = 0.008;
/** How much slower or faster a piece is played to get back to its own time (**provisional**): 20 ms of sound make up 1 ms. */
export const STRETCH = 0.05;
/** How far from its own time the sound is brought back by speed alone, in seconds (**provisional**). */
export const ABSORB_SECONDS = 0.3;

export type PiecePlacement =
	/** Its time is over: it is not sounded. */
	| { type: "too-late" }
	| {
			type: "sound";
			/** When it begins, on the clock of what sounds it, in seconds. */
			begin: number;
			/** How fast it is played; 1 as it is. */
			playbackRate: number;
			/** Where it ends: `soundUntil` for the next piece. */
			until: number;
			/** Whether it follows the piece before it to the sample; false for the first, and for one after a gap. */
			follows: boolean;
			/** Whether it is played slower or faster. */
			stretched: boolean;
	  };

/**
 * Says where a piece begins. All times in seconds on one clock: the one of
 * the `AudioContext` that sounds it.
 *
 * - `wanted`: its own time, with the buffer.
 * - `soundUntil`: where the piece before it ends; undefined for the first.
 * - `now`: the clock now.
 * - `duration`: how long the piece is.
 *
 * **A piece follows the one before it to the sample**, as long as that one
 * still sounds and ends within `ABSORB_SECONDS` of the piece's own time.
 * WHY not its own time each time: a page's clock counts whole
 * milliseconds, and a decoder gives times a microsecond off. A piece that
 * begins a fraction of a millisecond late leaves a gap of some samples,
 * and one that begins early sounds over the end of the one before: a click
 * at every piece.
 *
 * **Where the two are more than `SEAM_SECONDS` apart, the piece is played
 * a little slower or faster** (`STRETCH` at most), piece after piece, until
 * the sound is at its own time again. WHY: a buffer that moves moves the
 * time of every piece, and a piece that jumps there leaves a gap or sounds
 * over the one before. A changed speed is a changed pitch for a moment,
 * and no click. Beyond `ABSORB_SECONDS`, or where nothing sounds any more,
 * the piece begins at its own time: that is a pause, not a drift.
 */
export function placePiece(piece: {
	wanted: number;
	soundUntil: number | undefined;
	now: number;
	duration: number;
}): PiecePlacement {
	const { wanted, soundUntil, now, duration } = piece;
	const ahead =
		soundUntil !== undefined && soundUntil >= now
			? wanted - soundUntil
			: undefined;
	const follows = ahead !== undefined && Math.abs(ahead) <= ABSORB_SECONDS;
	const begin = follows && soundUntil !== undefined ? soundUntil : wanted;
	if (begin < now) return { type: "too-late" };
	// How much longer (or shorter) this piece sounds than it is.
	const stretch =
		!follows || ahead === undefined || Math.abs(ahead) <= SEAM_SECONDS
			? 0
			: Math.max(-STRETCH, Math.min(STRETCH, ahead / duration));
	return {
		type: "sound",
		begin,
		playbackRate: 1 / (1 + stretch),
		until: begin + duration * (1 + stretch),
		follows,
		stretched: stretch !== 0,
	};
}
