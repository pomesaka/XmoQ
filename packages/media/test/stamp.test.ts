import { expect, test } from "bun:test";
import { createStamp, type StampClock } from "../src/index.ts";

/**
 * Each frame: the time its source gave it (µs), and the wall clock when
 * it was handed over (ms). What comes back: the wall-clock time each is
 * sent with (µs).
 */
function stamped(
	clock: StampClock,
	frames: [number, number][],
	notBeforeUs?: number,
): number[] {
	const stamp = createStamp(clock, notBeforeUs);
	return frames.map(([sourceUs, nowMs]) => {
		stamp.take(sourceUs, nowMs);
		return stamp.of(sourceUs) ?? Number.NaN;
	});
}

test.each<{
	name: string;
	clock: StampClock;
	frames: [number, number][];
	notBeforeUs?: number;
	got: number[];
}>([
	{
		name: "first: the wall clock is read once, and the source's times go on from there",
		clock: "first",
		// The source's clock is slow: 40 ms of the wall for 33 ms of its own.
		frames: [
			[500_000, 10_000],
			[533_000, 10_040],
			[566_000, 10_080],
		],
		got: [10_000_000, 10_033_000, 10_066_000],
	},
	{
		name: "least: a frame that waited keeps the spacing of its source",
		clock: "least",
		// The second was handed over 7 ms late; its time does not say so.
		frames: [
			[500_000, 10_000],
			[533_000, 10_040],
			[566_000, 10_066],
		],
		got: [10_000_000, 10_033_000, 10_066_000],
	},
	{
		name: "least: a frame that came sooner than any moves the times back by that",
		clock: "least",
		frames: [
			[500_000, 10_005],
			[533_000, 10_033],
		],
		got: [10_005_000, 10_033_000],
	},
	{
		name: "least: a source whose clock is slow is followed when the minute has passed",
		clock: "least",
		// 61 s on: the source counted 60.9 s. The first frame is forgotten.
		frames: [
			[0, 10_000],
			[60_900_000, 71_000],
		],
		got: [10_000_000, 71_000_000],
	},
	{
		name: "least: within the minute the first frame still holds",
		clock: "least",
		frames: [
			[0, 10_000],
			[30_000_000, 40_050],
		],
		got: [10_000_000, 40_000_000],
	},
	{
		name: "least: the times do not go back where the wall clock is set back",
		clock: "least",
		// The wall clock went back by 100 ms between the two frames.
		frames: [
			[0, 10_000],
			[33_000, 9_933],
			[66_000, 9_966],
			[200_000, 10_100],
		],
		got: [10_000_000, 10_000_001, 10_000_002, 10_100_000],
	},
	{
		name: "least: a time less than a microsecond after the one it was held to is not the same one",
		clock: "least",
		// The second is held to the first; the third is 0.3 µs after that.
		frames: [
			[0, 10_000],
			[100.4, 9_990],
			[10_101.7, 10_001],
		],
		got: [10_000_000, 10_000_001, 10_000_002],
	},
	{
		name: "first: the first frame is not laid before the caller's epoch, and the times go up from there",
		// The wall clock was set back by 5 s since the epoch was read.
		clock: "first",
		frames: [
			[500_000, 5_000],
			[533_000, 5_033],
		],
		notBeforeUs: 10_000_000,
		got: [10_000_000, 10_033_000],
	},
	{
		name: "least: a frame whose time is not of the clock moves nothing",
		clock: "least",
		// A year ahead by the source: left out by the rule of the fastest way.
		frames: [
			[0, 10_000],
			[31_536_000_000_000, 10_033],
			[66_000, 10_066],
		],
		// Its own time is as far off as its source says; the next is not held to it.
		got: [10_000_000, 31_536_010_000_000, 10_066_000],
	},
])("$name", ({ clock, frames, notBeforeUs, got }) => {
	expect(stamped(clock, frames, notBeforeUs)).toEqual(got);
});

test("nothing is stamped before the first frame was taken", () => {
	expect(createStamp("least").of(0)).toBeUndefined();
	expect(createStamp("first").of(0)).toBeUndefined();
});

test("least: a source whose clock jumped is followed after 30 frames, as the fastest way is", () => {
	const stamp = createStamp("least");
	stamp.take(0, 10_000);
	// The source begins again an hour ahead; the wall clock goes on.
	const hour = 3_600_000_000;
	const got: number[] = [];
	for (let frame = 1; frame <= 30; frame++) {
		stamp.take(hour + frame * 33_000, 10_000 + frame * 33);
		got.push(stamp.of(hour + frame * 33_000) ?? Number.NaN);
	}
	// 29 are as far off as the source says; the 30th is of the wall clock again.
	expect(got[0]).toBe(3_610_033_000);
	expect(got[28]).toBe(3_610_957_000);
	expect(got[29]).toBe(10_990_000);
});
