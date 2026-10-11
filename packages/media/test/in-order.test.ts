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
