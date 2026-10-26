import { describe, expect, test } from "bun:test";
import { catchUpLines } from "../src/core/history.ts";

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
