import { describe, expect, test } from "bun:test";
import { SeenNumbers } from "../src/core/seen-numbers.ts";

describe("SeenNumbers", () => {
	test.each([
		["each item once", 3, [0n, 1n, 1n, 0n], [true, true, false, false]],
		[
			"the history after a live item",
			3,
			[5n, 3n, 4n, 5n],
			[true, true, true, false],
		],
		["one older than all, with all held", 2, [5n, 6n, 1n], [true, true, false]],
		[
			"one between, with all held: the oldest goes",
			2,
			[5n, 7n, 6n, 5n],
			[true, true, true, false],
		],
		[
			"one newer, with all held: the oldest goes",
			2,
			[5n, 6n, 7n, 5n],
			[true, true, true, false],
		],
	])("%s", (_case, most, numbers, taken) => {
		const seen = new SeenNumbers(most);
		expect(numbers.map((number) => seen.take(number))).toEqual(taken);
	});

	test.each([0, 1.5, -1])("%p lines held is refused", (most) => {
		expect(() => new SeenNumbers(most)).toThrow(RangeError);
	});
});
