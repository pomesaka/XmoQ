import { expect, test } from "bun:test";
import { createInOrder } from "../src/core/in-order.ts";

/** Runs `steps` ("time@now" comes, "@now" is a release) and says what went on and what was dropped. */
function run(
	holdMs: number,
	steps: readonly string[],
): { next: number[]; late: number[] } {
	const next: number[] = [];
	const late: number[] = [];
	const order = createInOrder<number>({
		holdMs: () => holdMs,
		next: (item) => next.push(item),
		late: (item) => late.push(item),
	});
	for (const step of steps) {
		const [time, now] = step.split("@");
		if (time === "") order.release(Number(now));
		else order.push(Number(time), Number(time), Number(now));
	}
	return { next, late };
}

interface Case {
	name: string;
	holdMs: number;
	steps: string[];
	next: number[];
	late: number[];
}

test.each<Case>([
	{
		name: "in order: each goes on as it comes",
		holdMs: 100,
		steps: ["0@0", "20@20", "40@40", "60@60"],
		next: [0, 20, 40, 60],
		late: [],
	},
	{
		name: "one that comes after a later one is put before it",
		holdMs: 100,
		steps: ["0@0", "20@20", "60@60", "40@70", "80@80"],
		next: [0, 20, 40, 60, 80],
		late: [],
	},
	{
		name: "one after a gap waits, and goes on when the hold is over",
		holdMs: 100,
		steps: ["0@0", "20@20", "60@60", "80@80", "@159", "@160"],
		next: [0, 20, 60, 80],
		late: [],
	},
	{
		name: "what comes after its place was given up is dropped",
		holdMs: 100,
		steps: ["0@0", "20@20", "60@60", "@160", "40@170"],
		next: [0, 20, 60],
		late: [40],
	},
	{
		name: "with no hold nothing waits: as it comes, and the older one is dropped",
		holdMs: 0,
		steps: ["0@0", "20@20", "60@60", "40@70"],
		next: [0, 20, 60],
		late: [40],
	},
	{
		name: "one that comes twice is dropped the second time",
		holdMs: 100,
		steps: ["0@0", "20@20", "60@60", "60@61", "20@62"],
		next: [0, 20],
		late: [60, 20],
	},
	{
		// 60 goes on at once (no step yet); 80 sets the step to 20, so 140
		// and 120 wait for 100, which does not come.
		name: "a gap between the first two is taken for the step until two closer ones come",
		holdMs: 100,
		steps: ["0@0", "60@60", "80@80", "140@140", "120@150", "@249"],
		next: [0, 60, 80],
		late: [],
	},
	{
		name: "and what waited goes on in order when the hold of the earliest is over",
		holdMs: 100,
		steps: ["0@0", "60@60", "80@80", "140@140", "120@150", "@250"],
		next: [0, 60, 80, 120, 140],
		late: [],
	},
	{
		// 41 comes 1 after 40, as a sender's least clock can put it: the
		// step stays 20 (the median of 20, 20 and 1), so 60 and 80 do not
		// wait for anything.
		name: "two that come close once do not make the ones after them wait",
		holdMs: 100,
		steps: ["0@0", "20@20", "40@40", "41@41", "60@60", "80@80"],
		next: [0, 20, 40, 41, 60, 80],
		late: [],
	},
	{
		// The step is 20 after 200 (four of 20, one of 200): 340 is 60 after
		// 280, more than one and a half steps, and waits. A mean (56) would let
		// it on at once.
		name: "one gap does not make the step larger",
		holdMs: 100,
		steps: [
			"0@0",
			"20@20",
			"40@40",
			"60@60",
			"80@80",
			"280@280",
			"@380",
			"340@390",
		],
		next: [0, 20, 40, 60, 80, 280],
		late: [],
	},
	{
		// Twenty distances of 20, then nine of 100: of the last 16, nine are
		// 100, and the step is 100, so 1440, 140 after 1300, goes on at once.
		// Of all of them it would be 20, and 1440 would wait.
		name: "the step is of the last 16 distances alone",
		holdMs: 100,
		steps: [
			"0@0",
			"20@20",
			"40@40",
			"60@60",
			"80@80",
			"100@100",
			"120@120",
			"140@140",
			"160@160",
			"180@180",
			"200@200",
			"220@220",
			"240@240",
			"260@260",
			"280@280",
			"300@300",
			"320@320",
			"340@340",
			"360@360",
			"380@380",
			"400@400",
			"500@500",
			"600@600",
			"700@700",
			"800@800",
			"900@900",
			"1000@1000",
			"1100@1100",
			"1200@1200",
			"1300@1300",
			"1440@1440",
		],
		next: [
			0, 20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 220, 240, 260, 280, 300,
			320, 340, 360, 380, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300,
			1440,
		],
		late: [],
	},
])("$name", ({ holdMs, steps, next, late }) => {
	expect(run(holdMs, steps)).toEqual({ next, late });
});

test("nextRelease says when the earliest held item has waited its hold", () => {
	const order = createInOrder<number>({
		holdMs: () => 100,
		next: () => {},
		late: () => {},
	});
	expect(order.nextRelease()).toBeUndefined();
	order.push(0, 0, 0);
	order.push(20, 20, 20);
	order.push(60, 60, 65);
	expect(order.nextRelease()).toBe(165);
});

test("anew gives what is held to late, and the next goes on whatever its time", () => {
	const next: number[] = [];
	const late: number[] = [];
	const order = createInOrder<number>({
		holdMs: () => 100,
		next: (item) => next.push(item),
		late: (item) => late.push(item),
	});
	// 0 and 20 go on; 80 comes after a gap and is held.
	order.push(0, 0, 0);
	order.push(20, 20, 1);
	order.push(80, 80, 2);
	order.anew();
	// A time before everything so far: after `anew` it is the first.
	order.push(5, 5, 3);
	order.push(25, 25, 4);
	expect({ next, late }).toEqual({ next: [0, 20, 5, 25], late: [80] });
});

test("anew forgets the distances: the step is of the items after it alone", () => {
	const next: number[] = [];
	const order = createInOrder<number>({
		holdMs: () => 100,
		next: (item) => next.push(item),
		late: () => {},
	});
	// A step of 1 before; 20 after, so 140 goes on at once.
	for (const time of [0, 1, 2]) order.push(time, time, time);
	order.anew();
	for (const time of [100, 120, 140]) order.push(time, time, time);
	expect(next).toEqual([0, 1, 2, 100, 120, 140]);
});
