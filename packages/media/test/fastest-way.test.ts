import { expect, test } from "bun:test";
import { createFastestWay } from "../src/index.ts";

/** What each Object that took so long gives: the fastest way, "anew N" where the clock began anew, "out" where it is left out. */
function taken(took: number[]): string[] {
	const way = createFastestWay();
	return took.map((each) => {
		const got = way.take(each);
		if (got === undefined) return "out";
		return `${got.anew ? "anew " : ""}${got.fastestMs}`;
	});
}

test.each<{ name: string; took: number[]; got: string[] }>([
	{
		name: "the fastest way is the smallest so far",
		took: [1000, 1020, 990, 1005],
		got: ["1000", "1000", "990", "990"],
	},
	{
		name: "an Object whose time is far ahead is left out and moves nothing",
		// A time a year ahead: it would be the fastest way by far.
		took: [1000, -31_536_000_000, 1010],
		got: ["1000", "out", "1000"],
	},
	{
		name: "an Object whose time is far behind is left out",
		took: [1000, 31_536_000_000, 1010],
		got: ["1000", "out", "1000"],
	},
	{
		name: "just within 10 seconds, and just beyond, either way",
		// The one just within on the early side is the fastest from then on.
		took: [1000, 11_000, 11_001, -9001, -9000],
		got: ["1000", "1000", "out", "out", "-9000"],
	},
	{
		name: "a time that is no number is left out, the first as well",
		took: [Number.NaN, 1000, Number.POSITIVE_INFINITY],
		got: ["out", "1000", "out"],
	},
])("$name", ({ took, got }) => {
	expect(taken(took)).toEqual(got);
});

test("after 30 in a row that are not of the clock, it begins anew with the thirtieth", () => {
	// The first Object carries a time a year ahead; what follows is right.
	const got = taken([
		-31_536_000_000,
		...Array.from({ length: 31 }, () => 1000),
	]);
	expect(got.slice(0, 30)).toEqual([
		"-31536000000",
		...Array.from({ length: 29 }, () => "out"),
	]);
	expect(got.slice(30)).toEqual(["anew 1000", "1000"]);
});

test("one that is of the clock between them begins the count of 30 again", () => {
	const strays = Array.from({ length: 29 }, () => 60_000);
	const got = taken([1000, ...strays, 1005, ...strays, 1010]);
	expect(got.filter((each) => each !== "out")).toEqual([
		"1000",
		"1000",
		"1000",
	]);
});
