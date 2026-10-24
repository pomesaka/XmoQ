import { expect, test } from "bun:test";
import { historyStart } from "../src/core/history.ts";

// The first Group of the Joining FETCH for the newest 16 lines.
test.each([
	["no line before the largest", 0n, 0n],
	["fewer than 16 lines", 14n, 0n],
	["exactly 16 lines", 15n, 0n],
	["17 lines", 16n, 1n],
	["many lines", 1_000n, 985n],
])("%s", (_case, largestGroup, start) => {
	expect(historyStart(largestGroup)).toBe(start);
});
