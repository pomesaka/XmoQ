// Which lines of a room a participant has had, so that each is handed on
// once: a line comes by the Joining FETCH and by the subscription where the
// two meet, and again by the FETCH of a subscription made anew (after the
// gatherer has started again, say).

/**
 * The numbers of the lines had, the largest `most` of them. A number below
 * all of them, where `most` are held, counts as had: it is older than every
 * line kept, and a page that shows the newest lines has no place for it.
 */
export class SeenLines {
	readonly #most: number;
	/** In ascending order, `most` at the most. */
	readonly #numbers: bigint[] = [];

	constructor(most: number) {
		if (!Number.isSafeInteger(most) || most < 1) {
			throw new RangeError(`most is a whole number from 1, not ${most}`);
		}
		this.#most = most;
	}

	/** Whether the line `number` is new; a new one is noted. */
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
