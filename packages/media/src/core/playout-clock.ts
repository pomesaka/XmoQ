// When a frame and a piece of audio are played: one clock and one buffer
// for both, so that picture and sound with the same time come out together.
// No I/O and no clock of its own: the caller says what time it is.

import { createFastestWay } from "./fastest-way.ts";

/**
 * Decided on 2026-10-21. WHY 150: with 1 % loss the sound broke 1 to 2
 * times in a run, and behind a link narrower than the video no piece was
 * too late; the first picture on a good link comes after about 208 ms
 * (docs/experiments/2026-10-14-a-buffer-of-100-ms-beside-50-150-and-the-automatic-one.md).
 * WHY NOT 50: 24 to 39 breaks at 1 % loss, and about three of four pieces
 * of the sound too late behind the narrow link. WHY NOT 100: 14 to 26
 * breaks, and 2 to 12 pieces too late behind the narrow link. WHY NOT the
 * automatic buffer: it settles at 40 ms on a good link, rose to its limit
 * of 400 ms in one run with loss, and nobody has listened to the sound it
 * plays at a changed speed.
 */
export const DEFAULT_PLAYOUT_BUFFER_MS = 150;
/**
 * The sound waits at least this long, also where the buffer is less. WHY:
 * a piece is given to the loudspeaker for a time, and with no buffer that
 * time is over when the piece comes: nothing sounds. The picture is drawn
 * as it comes. **Provisional**: the least of 10, 20, 30 and 40 ms with
 * which no piece was too late on one machine.
 */
export const DEFAULT_AUDIO_MIN_MS = 30;
/** What the buffer keeps for itself when a piece of audio waits for the ones before it (**provisional**). */
export const ORDER_MARGIN_MS = 20;
/**
 * The automatic buffer (**a trial, not the default**) follows how late
 * things come. It is what `autoShare` of the last AUTO_WINDOW_MS came
 * within, after the fastest, and AUTO_MARGIN_MS more. It grows at once and
 * shrinks AUTO_STEP_MS at a `tick`, which the caller gives twice a second.
 */
export const AUTO_WINDOW_MS = 10_000;
export const AUTO_MARGIN_MS = 10;
export const AUTO_STEP_MS = 10;
/** How few frames and pieces of the window the automatic buffer does not go by. */
export const AUTO_MIN_SAMPLES = 50;
/**
 * How far the automatic buffer goes. WHY a limit: with loss, a few of 100
 * come seconds late (measured: 0.8 s at 1 %, 5 s at 5 %), and a buffer that
 * waits for those is not live any more.
 */
export const DEFAULT_AUTO_MAX_MS = 400;
/** How many arrivals of the window the automatic buffer keeps at most, the newest: 10 seconds of 30 frames and 50 pieces of sound, twice over. */
export const MAX_LATENESS_SAMPLES = 1600;
export const DEFAULT_AUTO_SHARE = 0.99;

export interface PlayoutClockOptions {
	/** `DEFAULT_PLAYOUT_BUFFER_MS` when left out; where the automatic buffer begins. */
	bufferMs?: number;
	/** Whether the buffer follows how late things come. False when left out. */
	auto?: boolean;
	audioMinMs?: number;
	autoMaxMs?: number;
	autoShare?: number;
	/**
	 * **A trial; `AUTO_WINDOW_MS` when left out.** How far back the
	 * automatic buffer looks, in milliseconds. A shorter time forgets a bad
	 * moment sooner, and so comes down sooner and is down again when the
	 * next one comes.
	 */
	autoWindowMs?: number;
	/**
	 * **A trial; "step" when left out.** How the automatic buffer comes
	 * down at a `tick`: `AUTO_STEP_MS` ("step"), or a quarter of what is
	 * left to what is wanted, `AUTO_STEP_MS` at least, in steps of
	 * `AUTO_STEP_MS` ("quarter"). WHY a quarter and not all of it at once:
	 * the sound is played faster while the buffer shrinks, and a buffer
	 * that fell 360 ms in one step would have that to make up at once.
	 */
	autoDown?: "step" | "quarter";
}

export interface PlayoutClock {
	/**
	 * When something with this time (microseconds of the publisher's clock)
	 * would be played with no buffer, on the caller's clock (`now`, in
	 * milliseconds). Called once for each frame and piece as it comes: how
	 * late it came is what the automatic buffer goes by.
	 *
	 * **Undefined for one that is not of this clock** (`createFastestWay`:
	 * its time is more than 10 seconds from where the fastest one puts it):
	 * the caller leaves it out. WHY: the time is the publisher's to say,
	 * and one that is years off would have everything after it played years
	 * late, or be held for years itself.
	 */
	baseOf(timestampUs: number, now: number): number | undefined;
	/** How long after its base a frame is shown. */
	bufferMs(): number;
	/** How long after its base a piece of audio sounds: the buffer, and `audioMinMs` at least. */
	audioBufferMs(): number;
	/** How long a piece of audio may wait for the ones before it (`holdForOrderMs` of `playAudioTrack`). */
	holdForOrderMs(): number;
	isAuto(): boolean;
	/** Sets the buffer, and ends the automatic one. */
	setBuffer(ms: number): void;
	setAuto(): void;
	/** Moves the automatic buffer; does nothing without it. */
	tick(now: number): void;
	/** What `autoShare` of the frames and pieces of the window came within, after the fastest; 0 without any. */
	lateness(): number;
}

export function createPlayoutClock(
	options: PlayoutClockOptions = {},
): PlayoutClock {
	const audioMinMs = options.audioMinMs ?? DEFAULT_AUDIO_MIN_MS;
	const autoMaxMs = options.autoMaxMs ?? DEFAULT_AUTO_MAX_MS;
	const autoShare = options.autoShare ?? DEFAULT_AUTO_SHARE;
	const autoWindowMs = options.autoWindowMs ?? AUTO_WINDOW_MS;
	const autoDown = options.autoDown ?? "step";
	let bufferMs = options.bufferMs ?? DEFAULT_PLAYOUT_BUFFER_MS;
	let auto = options.auto ?? false;
	const way = createFastestWay();
	/** How long after the fastest each frame and piece of the window came, and when. */
	let late: { at: number; ms: number }[] = [];
	const forget = (now: number): void => {
		const since = now - autoWindowMs;
		if (late[0] !== undefined && late[0].at <= since) {
			late = late.filter((each) => each.at > since);
		}
	};
	const lateness = (): number => {
		const sorted = late.map((each) => each.ms).sort((a, b) => a - b);
		return (
			sorted[
				Math.min(sorted.length - 1, Math.floor(sorted.length * autoShare))
			] ?? 0
		);
	};
	const audioBufferMs = (): number => Math.max(bufferMs, audioMinMs);
	return {
		baseOf(timestampUs, now) {
			const taken = way.take(now - timestampUs / 1000, now);
			if (taken === undefined) return undefined;
			// What came by the clock before says nothing of this one.
			if (taken.anew) late = [];
			const base = timestampUs / 1000 + taken.fastestMs;
			forget(now);
			late.push({ at: now, ms: now - base });
			// WHY a limit beside the window: how many come in the window is
			// the publisher's to decide, and the list is put in order at
			// every tick.
			if (late.length > MAX_LATENESS_SAMPLES) late.shift();
			return base;
		},
		bufferMs: () => bufferMs,
		audioBufferMs,
		holdForOrderMs: () => Math.max(0, audioBufferMs() - ORDER_MARGIN_MS),
		isAuto: () => auto,
		setBuffer(ms) {
			bufferMs = ms;
			auto = false;
		},
		setAuto() {
			auto = true;
		},
		tick(now) {
			forget(now);
			if (!auto || late.length < AUTO_MIN_SAMPLES) return;
			const wanted = Math.min(
				autoMaxMs,
				Math.ceil((lateness() + AUTO_MARGIN_MS) / AUTO_STEP_MS) * AUTO_STEP_MS,
			);
			if (wanted > bufferMs) bufferMs = wanted;
			else if (wanted < bufferMs) {
				const down =
					autoDown === "quarter"
						? Math.ceil((bufferMs - wanted) / 4 / AUTO_STEP_MS) * AUTO_STEP_MS
						: AUTO_STEP_MS;
				bufferMs -= Math.max(AUTO_STEP_MS, down);
			}
		},
		lateness,
	};
}
