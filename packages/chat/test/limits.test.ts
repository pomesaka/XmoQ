import { describe, expect, test } from "bun:test";
import {
	chatLimits,
	keepBack,
	keepBackReaction,
	RateWindow,
} from "../src/core/limits.ts";

describe("RateWindow", () => {
	// The cases of `no_more_than_the_limit_is_taken_in_any_second` of
	// `crates/chat/src/rate.rs`, so that a page keeps back what the gatherer
	// would drop.
	test.each([
		["five at once", 5, [0, 0, 0, 0, 0], [true, true, true, true, true]],
		[
			"the sixth at once",
			5,
			[0, 0, 0, 0, 0, 0],
			[true, true, true, true, true, false],
		],
		[
			"a second after the first",
			2,
			[0, 500, 999, 1_000, 1_499, 1_500],
			[true, true, false, true, false, true],
		],
		[
			"a dropped line is not counted",
			1,
			[0, 10, 990, 1_000],
			[true, false, false, true],
		],
		["slow enough", 1, [0, 1_000, 2_000, 3_000], [true, true, true, true]],
		["none at all", 0, [0, 5_000], [false, false]],
	])("%s", (_case, most, times, taken) => {
		const window = new RateWindow(most);
		expect(times.map((time) => window.take(time))).toEqual(taken);
	});
});

describe("keepBack", () => {
	test.each([
		["512 bytes", "x".repeat(512), 512, undefined],
		["513 bytes", "x".repeat(513), 512, "too-long"],
		// "é" is 2 bytes of UTF-8 and 1 unit of UTF-16: the bytes count.
		["257 characters of 2 bytes", "é".repeat(257), 512, "too-long"],
		["a lone surrogate", "a\ud800", 512, "not-well-formed"],
		["a pair of surrogates", "😀", 4, undefined],
	])("%s", (_case, text, textBytes, why) => {
		const limits = chatLimits({ textBytes });
		const kept: string | undefined = keepBack(
			text,
			limits,
			new RateWindow(5),
			0,
		);
		expect(kept).toBe(why);
	});

	test("a line too long is not counted in the rate", () => {
		const limits = chatLimits({ textBytes: 1, linesPerSecond: 1 });
		const window = new RateWindow(limits.linesPerSecond);
		expect([
			keepBack("xx", limits, window, 0),
			keepBack("x", limits, window, 0),
			keepBack("x", limits, window, 0),
		]).toEqual(["too-long", undefined, "too-fast"]);
	});

	test("a reaction is kept back for its form before its rate, as a line is", () => {
		const window = new RateWindow(1);
		expect([
			keepBackReaction("", window, 0),
			keepBackReaction("👨‍👩‍👧‍👦", window, 0),
			keepBackReaction("\ud800", window, 0),
			keepBackReaction("👍", window, 0),
			keepBackReaction("👍", window, 0),
			keepBackReaction("👍", window, 1_000),
		]).toEqual([
			"not-an-emoji",
			"not-an-emoji",
			"not-well-formed",
			undefined,
			"too-fast",
			undefined,
		]);
	});
});

describe("chatLimits", () => {
	test("the defaults are the gatherer's", () => {
		expect(chatLimits()).toEqual({
			textBytes: 512,
			linesPerSecond: 5,
			reactionsPerSecond: 5,
		});
	});

	test.each([
		["no text", { textBytes: 0 }],
		["more than 8,192 bytes", { textBytes: 8_193 }],
		["a fraction", { textBytes: 1.5 }],
		["not a number", { textBytes: Number.NaN }],
		["no line a second", { linesPerSecond: 0 }],
		["more than 1,000 lines a second", { linesPerSecond: 1_001 }],
		["no reaction a second", { reactionsPerSecond: 0 }],
		["more than 1,000 reactions a second", { reactionsPerSecond: 1_001 }],
	])("%s is refused", (_case, given) => {
		expect(() => chatLimits(given)).toThrow(RangeError);
	});

	test.each([
		[
			"the most bytes",
			{ textBytes: 8_192 },
			{ textBytes: 8_192, linesPerSecond: 5, reactionsPerSecond: 5 },
		],
		[
			"one line a second",
			{ linesPerSecond: 1 },
			{ textBytes: 512, linesPerSecond: 1, reactionsPerSecond: 5 },
		],
		[
			"one reaction a second",
			{ reactionsPerSecond: 1 },
			{ textBytes: 512, linesPerSecond: 5, reactionsPerSecond: 1 },
		],
	])("%s is taken", (_case, given, limits) => {
		expect(chatLimits(given)).toEqual(limits);
	});
});
