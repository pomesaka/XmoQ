// Which items of a followed track a reader has had, so that each is handed
// on once: an item comes by the Joining FETCH and by the subscription where
// the two meet, and again by the FETCH of a subscription made anew (after
// the publisher has started again, say).

/**
 * The numbers of the items had, the largest `most` of them. A number below
 * all of them, where `most` are held, counts as had: it is older than every
 * item kept, and a reader that shows the newest items has no place for it.
 */
export class SeenNumbers {
	readonly #most: number;
	/** In ascending order, `most` at the most. */
	readonly #numbers: bigint[] = [];

	constructor(most: number) {
		if (!Number.isSafeInteger(most) || most < 1) {
			throw new RangeError(`most is a whole number from 1, not ${most}`);
		}
		this.#most = most;
	}

	/** Whether the item `number` is new; a new one is noted. */
	take(number: bigint): boolean {
		const numbers = this.#numbers;
		let low = 0;
		let high = numbers.length;
		while (low < high) {
			const middle = (low + high) >>> 1;
			if ((numbers[middle] ?? 0n) < number) low = middle + 1;
			else high = middle;
		}
		if (numbers[low] === number) return false;
		if (numbers.length >= this.#most && low === 0) return false;
		numbers.splice(low, 0, number);
		if (numbers.length > this.#most) numbers.shift();
		return true;
	}
}
