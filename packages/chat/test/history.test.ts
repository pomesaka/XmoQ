import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
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

/**
 * The rows of `fixtures/chat-history/history-to-fetch.tsv`, which the
 * terminal viewer reads too (`past_to_fetch` of `crates/subscriber`).
 */
function sharedRows(): string[][] {
	const table = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"fixtures",
			"chat-history",
			"history-to-fetch.tsv",
		),
		"utf8",
	);
	return table
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => row.split("\t"));
}

/** A column of the table: `-` is none. */
const bigintOf = (column: string | undefined): bigint | undefined =>
	column === undefined || column === "-" ? undefined : BigInt(column);

describe("what is fetched for a subscription to lines: fixtures/chat-history", () => {
	test.each(sharedRows())(
		"%s",
		(_name, had, largest, catchUp, forget, start, gapAfter, gapBefore) => {
			const plan = historyToFetch(
				bigintOf(had),
				bigintOf(largest),
				Number(catchUp),
			);
			expect(plan.forget).toBe(forget === "yes");
			expect(plan.fetch?.start).toBe(bigintOf(start));
			expect(plan.fetch?.gap?.after).toBe(bigintOf(gapAfter));
			expect(plan.fetch?.gap?.before).toBe(bigintOf(gapBefore));
		},
	);

	test("the table has its 14 rows of 8 columns", () => {
		expect(sharedRows().map((row) => row.length)).toEqual([
			8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
		]);
	});
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
