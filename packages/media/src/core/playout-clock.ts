// When a frame and a piece of audio are played: one clock and one buffer
// for both, so that picture and sound with the same time come out together.
// No I/O and no clock of its own: the caller says what time it is.

/** **Provisional**: on one machine 99 of 100 frames and pieces came within 17 to 24 ms of the fastest, and with 50 ms none was late (docs/experiments/2026-10-11-where-the-delay-of-the-demo-comes-from.md). */
export const DEFAULT_PLAYOUT_BUFFER_MS = 50;
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
export const DEFAULT_AUTO_SHARE = 0.99;

export interface PlayoutClockOptions {
	/** `DEFAULT_PLAYOUT_BUFFER_MS` when left out; where the automatic buffer begins. */
	bufferMs?: number;
	/** Whether the buffer follows how late things come. False when left out. */
	auto?: boolean;
	audioMinMs?: number;
	autoMaxMs?: number;
	autoShare?: number;
}

export interface PlayoutClock {
	/**
	 * When something with this time (microseconds of the publisher's clock)
	 * would be played with no buffer, on the caller's clock (`now`, in
	 * milliseconds). Called once for each frame and piece as it comes: how
	 * late it came is what the automatic buffer goes by.
	 *
	 * WHY the smallest difference between the two clocks is taken as "no
	 * delay": they need not agree, and the one that came fastest took at
	 * least no time.
	 */
	baseOf(timestampUs: number, now: number): number;
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
	let bufferMs = options.bufferMs ?? DEFAULT_PLAYOUT_BUFFER_MS;
	let auto = options.auto ?? false;
	let fastestMs = Number.POSITIVE_INFINITY;
	/** How long after the fastest each frame and piece of the window came, and when. */
	let late: { at: number; ms: number }[] = [];
	const forget = (now: number): void => {
		const since = now - AUTO_WINDOW_MS;
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
			fastestMs = Math.min(fastestMs, now - timestampUs / 1000);
			const base = timestampUs / 1000 + fastestMs;
			forget(now);
			late.push({ at: now, ms: now - base });
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
			else if (wanted < bufferMs) bufferMs -= AUTO_STEP_MS;
		},
		lateness,
	};
}
