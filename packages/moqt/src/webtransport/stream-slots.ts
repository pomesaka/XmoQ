/**
 * The places for the streams of one kind that are read at once. The loop
 * that takes streams from the transport waits for a place before it takes
 * the next one, so the streams over the limit stay with the transport.
 */
export interface StreamSlots {
	/** Resolves once a place is free, or the slots were closed. */
	free(): Promise<void>;
	/** A stream was taken and holds a place. */
	take(): void;
	/** A stream that held a place has ended. */
	release(): void;
	/**
	 * Nothing more is taken: whoever waits for a place is let go, and has to
	 * look at what ended the session before it takes a stream.
	 */
	close(): void;
}

export function createStreamSlots(limit: number): StreamSlots {
	let taken = 0;
	let closed = false;
	/**
	 * Whoever waits for a place: the loop that takes streams of this kind.
	 * WHY a list where there is one loop: a second caller would otherwise
	 * replace the first, which then waits for good.
	 */
	let waiting: (() => void)[] = [];
	const wakeWaiting = (): void => {
		const woken = waiting;
		waiting = [];
		for (const wake of woken) wake();
	};
	return {
		async free() {
			while (!closed && taken >= limit) {
				await new Promise<void>((resolve) => {
					waiting.push(resolve);
				});
			}
		},
		take() {
			taken += 1;
		},
		release() {
			taken -= 1;
			wakeWaiting();
		},
		close() {
			closed = true;
			wakeWaiting();
		},
	};
}
