import { describe, expect, test } from "bun:test";
import {
	catchUpLines,
	historyStart,
	historyToFetch,
} from "../src/core/history.ts";

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

describe("what is fetched for a subscription to lines", () => {
	// had, the Group of LARGEST_OBJECT, the lines it catches up at the most;
	// then whether what was had is forgotten, the first Group fetched, and
	// the gap before it.
	test.each([
		[
			"nothing had: the newest 16",
			undefined,
			1_000n,
			200,
			false,
			985n,
			undefined,
			undefined,
		],
		[
			"nothing had, in a room of 3 lines",
			undefined,
			2n,
			200,
			false,
			0n,
			undefined,
			undefined,
		],
		[
			"nothing had, in a room without lines",
			undefined,
			undefined,
			200,
			false,
			undefined,
			undefined,
			undefined,
		],
		[
			"had up to the largest already",
			50n,
			50n,
			200,
			false,
			undefined,
			undefined,
			undefined,
		],
		["a few lines missed", 50n, 55n, 200, false, 51n, undefined, undefined],
		["one line missed", 50n, 51n, 200, false, 51n, undefined, undefined],
		[
			"missed exactly as many as it catches up",
			50n,
			250n,
			200,
			false,
			51n,
			undefined,
			undefined,
		],
		[
			"missed one more than it catches up: the newest, and a gap",
			50n,
			251n,
			200,
			false,
			52n,
			50n,
			52n,
		],
		["missed many more", 50n, 10_000n, 200, false, 9_801n, 50n, 9_801n],
		[
			"catching up one line at the most",
			50n,
			1_000n,
			1,
			false,
			1_000n,
			50n,
			1_000n,
		],
		[
			"catching up the most a setting allows",
			0n,
			5_000n,
			1_000,
			false,
			4_001n,
			0n,
			4_001n,
		],
		[
			"a largest below what was had: numbered anew, the newest 16",
			50n,
			30n,
			200,
			true,
			15n,
			undefined,
			undefined,
		],
		[
			"numbered anew, fewer than 16 lines",
			50n,
			3n,
			200,
			true,
			0n,
			undefined,
			undefined,
		],
		[
			"numbered anew, without lines yet",
			50n,
			undefined,
			200,
			true,
			undefined,
			undefined,
			undefined,
		],
	])(
		"%s",
		(_case, had, largest, catchUp, forget, start, gapAfter, gapBefore) => {
			const plan = historyToFetch(had, largest, catchUp);
			expect(plan.forget).toBe(forget);
			expect(plan.fetch?.start).toBe(start);
			expect(plan.fetch?.gap?.after).toBe(gapAfter);
			expect(plan.fetch?.gap?.before).toBe(gapBefore);
		},
	);
});

describe("the lines a participant catches up at the most", () => {
	test.each([
		["left out: what a page keeps", undefined, 200],
		["the fewest", 1, 1],
		["the most: the gatherer's lines of one FETCH", 1_000, 1_000],
	])("%s", (_case, given, lines) => {
		expect(catchUpLines(given)).toBe(lines);
	});

	test.each([
		["none", 0],
		["over the most", 1_001],
		["not whole", 1.5],
		["not a number", Number.NaN],
		["below zero", -1],
	])("%s is refused", (_case, given) => {
		expect(() => catchUpLines(given)).toThrow(RangeError);
	});
});
