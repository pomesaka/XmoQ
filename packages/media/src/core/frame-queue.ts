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
	 * over for it. The caller closes all of them.
	 *
	 * WHY the buffer is given here and not when the frame came: it can be
	 * moved while watching, and the frames that wait follow it at once.
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
			const due = frames.filter((each) => each.base + bufferMs <= now);
			frames = frames.filter((each) => each.base + bufferMs > now);
			return due;
		},
		waiting: () => frames.length,
	};
}
