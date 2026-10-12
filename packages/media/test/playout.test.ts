import { describe, expect, test } from "bun:test";
import {
	createFrameQueue,
	createPlayoutClock,
	type PlayoutClockOptions,
	placePiece,
} from "../src/index.ts";

describe("createPlayoutClock", () => {
	test("the base of a time is counted from the fastest one that came", () => {
		const clock = createPlayoutClock();
		const bases = [
			[1_000_000, 5000],
			[1_040_000, 5060],
			[1_080_000, 5070],
		].map(([timestampUs = 0, now = 0]) => clock.baseOf(timestampUs, now));
		expect(bases).toEqual([5000, 5040, 5070]);
	});

	test.each<{
		options: PlayoutClockOptions;
		bufferMs: number;
		audioBufferMs: number;
		holdForOrderMs: number;
	}>([
		{ options: {}, bufferMs: 50, audioBufferMs: 50, holdForOrderMs: 30 },
		{
			options: { bufferMs: 0 },
			bufferMs: 0,
			audioBufferMs: 30,
			holdForOrderMs: 10,
		},
		{
			options: { bufferMs: 150 },
			bufferMs: 150,
			audioBufferMs: 150,
			holdForOrderMs: 130,
		},
		{
			options: { bufferMs: 10, audioMinMs: 10 },
			bufferMs: 10,
			audioBufferMs: 10,
			holdForOrderMs: 0,
		},
	])("the buffers with $options", ({ options, ...buffers }) => {
		const clock = createPlayoutClock(options);
		expect({
			bufferMs: clock.bufferMs(),
			audioBufferMs: clock.audioBufferMs(),
			holdForOrderMs: clock.holdForOrderMs(),
		}).toEqual(buffers);
	});

	test.each<{
		name: string;
		options: PlayoutClockOptions;
		/** How many came, 10 ms apart; the first as fast as any, the others `lateMs` after it. */
		samples: number;
		lateMs: number;
		ticks: number;
		bufferMs: number;
	}>([
		{
			name: "grows at once to what 99 of 100 came within, and 10 ms more",
			options: { bufferMs: 0, auto: true },
			samples: 100,
			lateMs: 35,
			ticks: 1,
			bufferMs: 50,
		},
		{
			name: "shrinks 10 ms at a tick",
			options: { bufferMs: 150, auto: true },
			samples: 100,
			lateMs: 35,
			ticks: 2,
			bufferMs: 130,
		},
		{
			name: "goes no further than its limit",
			options: { bufferMs: 50, auto: true },
			samples: 100,
			lateMs: 1000,
			ticks: 1,
			bufferMs: 400,
		},
		{
			name: "a limit that is set",
			options: { bufferMs: 50, auto: true, autoMaxMs: 200 },
			samples: 100,
			lateMs: 1000,
			ticks: 1,
			bufferMs: 200,
		},
		{
			name: "does not move by fewer than 50",
			options: { bufferMs: 0, auto: true },
			samples: 49,
			lateMs: 35,
			ticks: 1,
			bufferMs: 0,
		},
		{
			name: "does not move where it is not automatic",
			options: { bufferMs: 0 },
			samples: 100,
			lateMs: 35,
			ticks: 1,
			bufferMs: 0,
		},
	])(
		"the automatic buffer $name",
		({ options, samples, lateMs, ticks, bufferMs }) => {
			const clock = createPlayoutClock(options);
			let now = 1000;
			for (let index = 0; index < samples; index++) {
				now = 1000 + index * 10 + (index === 0 ? 0 : lateMs);
				clock.baseOf(index * 10_000, now);
			}
			for (let tick = 0; tick < ticks; tick++) clock.tick(now + 1);
			expect(clock.bufferMs()).toBe(bufferMs);
		},
	);

	test("what came more than 10 seconds ago is forgotten, and a buffer that is set is not automatic", () => {
		const clock = createPlayoutClock({ bufferMs: 50, auto: true });
		for (let index = 0; index < 100; index++) {
			clock.baseOf(index * 10_000, 1000 + index * 10 + (index === 0 ? 0 : 200));
		}
		clock.tick(2200);
		expect(clock.bufferMs()).toBe(210);
		// 11 seconds later, 100 that come as fast as the first.
		for (let index = 0; index < 100; index++) {
			clock.baseOf(13_000_000 + index * 10_000, 14_000 + index * 10);
		}
		clock.tick(15_000);
		expect(clock.lateness()).toBe(0);
		expect(clock.bufferMs()).toBe(200);
		clock.setBuffer(150);
		clock.tick(15_001);
		expect({ bufferMs: clock.bufferMs(), auto: clock.isAuto() }).toEqual({
			bufferMs: 150,
			auto: false,
		});
	});
});

describe("createFrameQueue", () => {
	test("the oldest goes when more wait than may", () => {
		const queue = createFrameQueue<string>(2);
		const gone = ["a", "b", "c"].map((frame, index) =>
			queue.push({ frame, base: index, came: index }).map((each) => each.frame),
		);
		expect(gone).toEqual([[], [], ["a"]]);
		expect(queue.waiting()).toBe(2);
	});

	test.each<{ now: number; bufferMs: number; due: number[]; waiting: number }>([
		{ now: 190, bufferMs: 50, due: [100, 133], waiting: 1 },
		{ now: 190, bufferMs: 0, due: [100, 133, 166], waiting: 0 },
		{ now: 190, bufferMs: 150, due: [], waiting: 3 },
		{ now: 150, bufferMs: 50, due: [100], waiting: 2 },
	])(
		"at $now with a buffer of $bufferMs the frames of $due are due",
		({ now, bufferMs, due, waiting }) => {
			const queue = createFrameQueue<number>();
			for (const base of [100, 133, 166]) {
				queue.push({ frame: base, base, came: base });
			}
			expect(queue.due(now, bufferMs).map((each) => each.frame)).toEqual(due);
			expect(queue.waiting()).toBe(waiting);
		},
	);
});

describe("placePiece", () => {
	test.each<{
		name: string;
		wanted: number;
		soundUntil: number | undefined;
		now: number;
		begin: number;
		playbackRate: number;
		until: number;
		follows: boolean;
		stretched: boolean;
	}>([
		{
			name: "the first piece begins at its own time",
			wanted: 1.05,
			soundUntil: undefined,
			now: 1.0,
			begin: 1.05,
			playbackRate: 1,
			until: 1.07,
			follows: false,
			stretched: false,
		},
		{
			name: "a piece a millisecond off follows the one before it to the sample",
			wanted: 1.071,
			soundUntil: 1.07,
			now: 1.03,
			begin: 1.07,
			playbackRate: 1,
			until: 1.09,
			follows: true,
			stretched: false,
		},
		{
			name: "a piece whose time is 20 ms later follows, and is played slower",
			wanted: 1.09,
			soundUntil: 1.07,
			now: 1.03,
			begin: 1.07,
			playbackRate: 0.952381,
			until: 1.091,
			follows: true,
			stretched: true,
		},
		{
			name: "a piece whose time is 20 ms earlier follows, and is played faster",
			wanted: 1.05,
			soundUntil: 1.07,
			now: 1.03,
			begin: 1.07,
			playbackRate: 1.052632,
			until: 1.089,
			follows: true,
			stretched: true,
		},
		{
			name: "a piece whose own time is over follows the one that still sounds",
			wanted: 1.02,
			soundUntil: 1.07,
			now: 1.03,
			begin: 1.07,
			playbackRate: 1.052632,
			until: 1.089,
			follows: true,
			stretched: true,
		},
		{
			name: "a piece more than 0.3 s from the one before begins at its own time",
			wanted: 1.5,
			soundUntil: 1.07,
			now: 1.03,
			begin: 1.5,
			playbackRate: 1,
			until: 1.52,
			follows: false,
			stretched: false,
		},
		{
			name: "a piece after one that has ended begins at its own time",
			wanted: 1.05,
			soundUntil: 1.0,
			now: 1.03,
			begin: 1.05,
			playbackRate: 1,
			until: 1.07,
			follows: false,
			stretched: false,
		},
	])(
		"$name",
		({
			wanted,
			soundUntil,
			now,
			begin,
			playbackRate,
			until,
			follows,
			stretched,
		}) => {
			const placed = placePiece({ wanted, soundUntil, now, duration: 0.02 });
			if (placed.type !== "sound") throw new Error("not sounded");
			expect(placed.begin).toBeCloseTo(begin, 6);
			expect(placed.playbackRate).toBeCloseTo(playbackRate, 5);
			expect(placed.until).toBeCloseTo(until, 6);
			expect({ follows: placed.follows, stretched: placed.stretched }).toEqual({
				follows,
				stretched,
			});
		},
	);

	test.each<{ name: string; wanted: number; soundUntil: number | undefined }>([
		{ name: "the first", wanted: 0.99, soundUntil: undefined },
		{ name: "after one that has ended", wanted: 0.99, soundUntil: 0.98 },
	])(
		"a piece whose time is over is not sounded: $name",
		({ wanted, soundUntil }) => {
			expect(
				placePiece({ wanted, soundUntil, now: 1.0, duration: 0.02 }),
			).toEqual({
				type: "too-late",
			});
		},
	);
});
