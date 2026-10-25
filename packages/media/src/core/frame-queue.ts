// The decoded frames of a player that wait for their time. No I/O.

/** How many decoded frames may wait to be shown: a second of them at 30 frames. */
export const DEFAULT_MAX_WAITING_FRAMES = 30;

export interface WaitingFrame<T> {
	frame: T;
	/** When it would be shown with no buffer, and when it came, on the caller's clock in milliseconds. */
	base: number;
	came: number;
}

export interface FrameQueue<T> {
	/**
	 * Takes a frame, and gives the ones that go for it, the oldest first:
	 * the caller closes them.
	 *
	 * WHY a limit: a tab in the background is not repainted, and the frames
	 * it decodes would wait without end, each with its picture. The picture
	 * catches up at once when the tab is looked at again.
	 */
	push(frame: WaitingFrame<T>): WaitingFrame<T>[];
	/**
	 * The frames whose time has come, in the order they came, and no longer
	 * held: **the last one is shown**, and the ones before it are passed
	 * over for it. The caller closes all of them. **A frame that came before
	 * one whose time has come goes with it, whatever its own time**: the
	 * picture never goes back.
	 *
	 * WHY the buffer is given here and not when the frame came: it can be
	 * moved while watching, and the frames that wait follow it at once.
	 *
	 * WHY the frames before go too: a frame's time is fixed when it comes,
	 * by the fastest way of that moment (`PlayoutClock.baseOf`), and the
	 * frames come in the order of their Timestamps. When the fastest way
	 * becomes much shorter between two frames, as where the start of a
	 * Group comes late by a Joining FETCH and the next frames on time by
	 * the subscription, the later frame can be due before the earlier one;
	 * taken each by its own time, the later was drawn first and the earlier
	 * after it (seen once through two relays: 85, 87, 86, 88). WHY NOT keep
	 * the times from going down in the order of the frames: the clock would
	 * be held back for the frames after such a change, with a rule of its
	 * own beside the fastest way. WHY NOT work the times of the frames that
	 * wait out again when the fastest way changes: every frame that waits
	 * would move, for a case that passing over one frame settles.
	 */
	due(now: number, bufferMs: number): WaitingFrame<T>[];
	waiting(): number;
}

export function createFrameQueue<T>(
	maxWaiting: number = DEFAULT_MAX_WAITING_FRAMES,
): FrameQueue<T> {
	let frames: WaitingFrame<T>[] = [];
	return {
		push(frame) {
			const gone: WaitingFrame<T>[] = [];
			while (frames.length >= maxWaiting) {
				const oldest = frames.shift();
				if (oldest === undefined) break;
				gone.push(oldest);
			}
			frames.push(frame);
			return gone;
		},
		due(now, bufferMs) {
			let through = 0;
			for (const [index, each] of frames.entries()) {
				if (each.base + bufferMs <= now) through = index + 1;
			}
			const due = frames.slice(0, through);
			frames = frames.slice(through);
			return due;
		},
		waiting: () => frames.length,
	};
}
