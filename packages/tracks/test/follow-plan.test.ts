import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { followPlan, followStart } from "../src/core/follow-plan.ts";

// The first Group of the Joining FETCH for the newest 16 items.
test.each([
	["no line before the largest", 0n, 0n],
	["fewer than 16 items", 14n, 0n],
	["exactly 16 items", 15n, 0n],
	["17 items", 16n, 1n],
	["many items", 1_000n, 985n],
])("%s", (_case, largestGroup, start) => {
	expect(followStart(largestGroup, 16)).toBe(start);
});

/**
 * The rows of `fixtures/chat-history/history-to-fetch.tsv`, which the Rust
 * readers read too (`past_to_fetch` of `xmoq-subscriber`); its rule is that
 * of every followed track, with the newest 16 fetched where nothing was had.
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

describe("what is fetched for a subscription: fixtures/chat-history", () => {
	test.each(sharedRows())(
		"%s",
		(_name, had, largest, catchUp, forget, start, gapAfter, gapBefore) => {
			const plan = followPlan(
				bigintOf(had),
				bigintOf(largest),
				16,
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

test.each([
	["nothing had, and no item to fetch", undefined, 9n, 0, 1, undefined],
	["items had, and none to catch up", 5n, 9n, 16, 0, undefined],
])("fetching none: %s", (_case, had, largest, recent, catchUp, start) => {
	expect(followPlan(had, largest, recent, catchUp).fetch?.start).toBe(start);
});
