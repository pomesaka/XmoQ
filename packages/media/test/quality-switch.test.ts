import { expect, test } from "bun:test";
import { createQualitySwitch } from "../src/index.ts";

test.each<{
	name: string;
	levels: number;
	level: number;
	/** `[now, how late the picture is]`, each in milliseconds; null where no frame came. */
	ticks: [number, number | null][];
	played: number[];
}>([
	{
		name: "a picture that is late goes one version down at once",
		levels: 2,
		level: 0,
		ticks: [
			[0, 50],
			[500, 400],
		],
		played: [0, 1],
	},
	{
		name: "no frame at all is late",
		levels: 2,
		level: 0,
		ticks: [[0, null]],
		played: [1],
	},
	{
		name: "after a change nothing changes for 2 s, and the lowest version stays",
		levels: 3,
		level: 0,
		ticks: [
			[0, 400],
			[500, 400],
			[2000, 400],
			[4000, 400],
		],
		played: [1, 1, 2, 2],
	},
	{
		name: "a picture between in time and late changes nothing",
		levels: 2,
		level: 0,
		ticks: [
			[0, 200],
			[6000, 200],
		],
		played: [0, 0],
	},
	{
		name: "a picture in time for 5 s goes one version up",
		levels: 2,
		level: 1,
		ticks: [
			[0, 50],
			[4500, 50],
			[5000, 50],
		],
		played: [1, 1, 0],
	},
	{
		name: "a moment that is not in time begins the 5 s anew",
		levels: 2,
		level: 1,
		ticks: [
			[0, 50],
			[3000, 150],
			[3500, 50],
			[8000, 50],
			[8500, 50],
		],
		played: [1, 1, 1, 1, 0],
	},
	{
		name: "a try that fails within 5 s makes the next one wait twice as long",
		levels: 2,
		level: 1,
		ticks: [
			[0, 50],
			[5000, 50],
			[6000, 400],
			[7000, 400],
			[7500, 50],
			[12500, 50],
			[17000, 50],
			[17500, 50],
		],
		played: [1, 0, 0, 1, 1, 1, 1, 0],
	},
	{
		name: "a version that holds for the longer wait sets the wait back",
		levels: 2,
		level: 1,
		ticks: [
			[0, 50],
			[5000, 50],
			[7000, 400],
			[7500, 50],
			[17500, 50],
			[28000, 50],
			[28500, 400],
			[29000, 50],
			[34000, 50],
		],
		played: [1, 0, 1, 1, 0, 0, 1, 1, 0],
	},
])("$name", ({ levels, level, ticks, played }) => {
	const quality = createQualitySwitch({ levels, level });
	expect(
		ticks.map(([now, late]) => quality.tick(now, late ?? undefined)),
	).toEqual(played);
});

test.each<{ levels: number; level?: number }>([
	{ levels: 0 },
	{ levels: 1.5 },
	{ levels: 2, level: 2 },
	{ levels: 2, level: -1 },
])(
	"a number of versions or a version that is none is refused: $levels, $level",
	(options) => {
		expect(() => createQualitySwitch(options)).toThrow(RangeError);
	},
);
