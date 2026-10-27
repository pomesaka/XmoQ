import { expect, test } from "bun:test";
import { createPauses } from "../src/core/pauses.ts";

test.each<{
	name: string;
	/** In order: `resumed` with a counted time and a source time, or `shift` of a counted time. */
	steps: ({ resumed: [number, number] } | { shift: number })[];
	/** What each `shift` gave. */
	shifts: number[];
}>([
	{
		name: "without a pause nothing is added",
		steps: [{ shift: 0 }, { shift: 20_000 }],
		shifts: [0, 0],
	},
	{
		name: "after a pause of 3 s the times from where it came back are 3 s later, and those before are not",
		steps: [
			{ resumed: [100_000, 3_100_000] },
			{ shift: 80_000 },
			{ shift: 100_000 },
			{ shift: 120_000 },
		],
		shifts: [0, 3_000_000, 3_000_000],
	},
	{
		name: "two pauses add up",
		steps: [
			{ resumed: [100_000, 1_100_000] },
			{ resumed: [200_000, 2_200_000] },
			{ shift: 150_000 },
			{ shift: 200_000 },
		],
		shifts: [1_000_000, 2_000_000],
	},
	{
		name: "a source that came back no later than its samples stood still for no time",
		steps: [{ resumed: [100_000, 99_000] }, { shift: 100_000 }],
		shifts: [0],
	},
])("$name", ({ steps, shifts }) => {
	const pauses = createPauses();
	const given: number[] = [];
	for (const step of steps) {
		if ("resumed" in step) pauses.resumed(...step.resumed);
		else given.push(pauses.shift(step.shift));
	}
	expect(given).toEqual(shifts);
});
