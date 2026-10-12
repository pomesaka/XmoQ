import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createFastestWay } from "../src/index.ts";

/** What each Object that took so long gives: the fastest way, "anew N" where the clock began anew, "out" where it is left out. */
function taken(took: number[]): string[] {
	const way = createFastestWay();
	return took.map((each) => {
		// All in one second: nothing is forgotten in these rows.
		const got = way.take(each, 0);
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

// The rows of `fixtures/playout/least.tsv`, which `crates/watch` reads too:
// when each Object came and how long it took, and the fastest way after it
// ("-" where it is left out).
const LEAST = readFileSync(
	join(import.meta.dir, "..", "..", "..", "fixtures", "playout", "least.tsv"),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => row.split("\t"));

test("the table of the least has the rows both players count on", () => {
	expect(LEAST.length).toBe(8);
});

test.each(LEAST)(
	"the fastest way of the last minute: %s",
	(_name, came, after) => {
		const way = createFastestWay();
		const got = (came ?? "").split(" ").map((each) => {
			const [at, took] = each.split(":").map(Number);
			const taken = way.take(took ?? Number.NaN, at ?? Number.NaN);
			return taken === undefined ? "-" : String(taken.fastestMs);
		});
		expect(got.join(" ")).toBe(after ?? "");
	},
);
