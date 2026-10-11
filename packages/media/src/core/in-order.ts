/**
 * Puts what comes out of order back in order, by its time, and waits a
 * limited while for what is missing.
 *
 * An item goes on at once when it is the next: its time is within one and
 * a half steps of the last one that went on, where a step is the smallest
 * distance seen between two items that went on one after the other. An
 * item after a gap is held for what is missing, at most `holdMs()` from
 * when it came, and then goes on; what comes with a time that is not after
 * the last one that went on is dropped.
 *
 * WHY by the time and not by the Group and the Object ID: nothing says
 * where a Group ends, so the first Object of the next Group cannot be told
 * from one after a gap by its IDs. The times of audio are evenly spaced.
 *
 * WHY the first two items go on without waiting: no step is known yet. A
 * gap between them is taken for the step, and corrected by the first two
 * that are closer.
 */
export interface InOrder<T> {
	/** `time` in the units of the items; `now` in milliseconds. */
	push(time: number, item: T, now: number): void;
	/** Lets on what has waited long enough. */
	release(now: number): void;
	/** When `release` has something to do next, in milliseconds; undefined when nothing is held. */
	nextRelease(): number | undefined;
}

/** How many items are held at most; the earliest goes on when one more comes. */
export const MAX_HELD_IN_ORDER = 64;

export function createInOrder<T>(options: {
	holdMs(): number;
	/** Called in the order of the times. */
	next(item: T): void;
	/** Called with an item whose time is not after the last one that went on, or that came twice. */
	late(item: T): void;
}): InOrder<T> {
	let last: number | undefined;
	let step: number | undefined;
	const held: { time: number; item: T; came: number }[] = [];
	const pass = (now: number): void => {
		for (;;) {
			const head = held[0];
			if (head === undefined) return;
			const isNext =
				last === undefined ||
				step === undefined ||
				head.time - last <= step * 1.5;
			if (
				!isNext &&
				head.came + options.holdMs() > now &&
				held.length <= MAX_HELD_IN_ORDER
			) {
				return;
			}
			held.shift();
			if (last !== undefined) {
				step = Math.min(step ?? Number.POSITIVE_INFINITY, head.time - last);
			}
			last = head.time;
			options.next(head.item);
		}
	};
	return {
		push(time, item, now) {
			if (
				(last !== undefined && time <= last) ||
				held.some((each) => each.time === time)
			) {
				options.late(item);
				return;
			}
			const index = held.findIndex((each) => each.time > time);
			held.splice(index === -1 ? held.length : index, 0, {
				time,
				item,
				came: now,
			});
			pass(now);
		},
		release: pass,
		nextRelease() {
			const head = held[0];
			return head === undefined ? undefined : head.came + options.holdMs();
		},
	};
}
