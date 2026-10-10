/**
 * A bounded queue between stream pumps and one consumer, exposed as a
 * `ReadableStream`.
 *
 * WHY its own list of items, and not the queue inside the `ReadableStream`:
 * `controller.error()` throws away what that queue holds, so a stream that
 * has to fail would lose the items its consumer has not read yet. Here the
 * stream is given one item per read, and it closes or fails only when a read
 * finds the list empty.
 */
export interface ObjectQueue<T> {
	readonly readable: ReadableStream<T>;
	/**
	 * Queues an item. The promise resolves at once while the queue has room,
	 * and otherwise when the consumer has read enough for it to take more.
	 * Callers stop reading their source until then. An item pushed after the
	 * end is dropped.
	 */
	push(item: T): Promise<void>;
	/**
	 * No more items will come. The consumer still reads every item queued so
	 * far; after the last one the stream closes, or fails with `error`.
	 */
	end(error?: Error): void;
	/** Closes the stream now and drops the items that were not read. */
	discard(): void;
}

/**
 * `onCancel` runs when the consumer cancels the stream.
 */
export function createObjectQueue<T>(
	highWaterMark: number,
	onCancel: () => Promise<void>,
): ObjectQueue<T> {
	let controller: ReadableStreamDefaultController<T> | undefined;
	let items: T[] = [];
	/** `end` was called: the stream settles once `items` is read. */
	let ending: { error: Error | undefined } | undefined;
	/** The stream is closed, failed or cancelled. */
	let settled = false;
	// WHY a list: several subgroup streams feed one subscription, and each of
	// them can be waiting at the same time.
	let waitingPushes: (() => void)[] = [];
	/** A read that found the queue empty and waits for an item or the end. */
	let wakeRead: (() => void) | undefined;

	const releasePushes = (): void => {
		const release = waitingPushes;
		waitingPushes = [];
		for (const resolve of release) resolve();
	};

	const wake = (): void => {
		wakeRead?.();
		wakeRead = undefined;
	};

	/** Gives a waiting read the next item, or the end of the stream. */
	const serveRead = (): void => {
		if (settled || controller === undefined) return;
		if (items.length > 0) {
			const [item, ...rest] = items;
			items = rest;
			// `items` had an element, so `item` is one of the caller's items.
			if (item !== undefined) controller.enqueue(item);
			if (items.length < highWaterMark) releasePushes();
			return;
		}
		if (ending === undefined) return;
		settled = true;
		if (ending.error === undefined) controller.close();
		else controller.error(ending.error);
	};

	const readable = new ReadableStream<T>(
		{
			start(streamController) {
				controller = streamController;
			},
			// With a high-water mark of 0 this runs only while a read is waiting.
			pull() {
				if (items.length > 0 || ending !== undefined) {
					serveRead();
					return undefined;
				}
				return new Promise<void>((resolve) => {
					wakeRead = resolve;
				}).then(serveRead);
			},
			cancel() {
				settled = true;
				items = [];
				wake();
				releasePushes();
				return onCancel();
			},
		},
		{ highWaterMark: 0 },
	);

	return {
		readable,
		push(item) {
			if (settled || ending !== undefined) return Promise.resolve();
			items.push(item);
			wake();
			if (items.length < highWaterMark) return Promise.resolve();
			return new Promise((resolve) => {
				waitingPushes.push(resolve);
			});
		},
		end(error) {
			if (settled || ending !== undefined) return;
			ending = { error };
			wake();
			releasePushes();
		},
		discard() {
			if (settled) return;
			settled = true;
			items = [];
			controller?.close();
			wake();
			releasePushes();
		},
	};
}
