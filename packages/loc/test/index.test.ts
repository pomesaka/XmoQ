import { describe, expect, test } from "bun:test";
import {
	beginsWithStartCode,
	LOC_REVISION,
	locProperties,
	microsecondsOf,
	readLocTiming,
	toAnnexB,
} from "../src/index.ts";

// What holds of this package whichever revision of LOC it is: nothing here
// names a number of a property. What is of one revision is tested in the
// directory with its name (`test/draft-04/`).

describe("@xmoq/loc, as the revision it was built as", () => {
	test("says which document it implements", () => {
		// The one test here that a change of the revision changes.
		expect(LOC_REVISION).toBe("draft-ietf-moq-loc-04");
	});

	test.each<{ name: string; timestamp: bigint; timescale?: bigint }>([
		{ name: "a time alone", timestamp: 1791690527000000n },
		{ name: "a time of 0", timestamp: 0n },
		{ name: "a time with its unit", timestamp: 3000n, timescale: 90000n },
	])(
		"the properties it writes for $name, it reads back as that time",
		({ timestamp, timescale }) => {
			const timing =
				timescale === undefined ? { timestamp } : { timestamp, timescale };
			expect(readLocTiming(locProperties(timing))).toEqual({
				type: "ok",
				timing,
			});
		},
	);

	test("an Object with no property of LOC has no time", () => {
		expect(readLocTiming([])).toEqual({ type: "ok", timing: undefined });
	});

	test("a time with its unit is that many microseconds", () => {
		expect(microsecondsOf({ timestamp: 3000n, timescale: 90000n })).toBe(
			33333n,
		);
	});

	test("what `toAnnexB` gives begins with a start code, and is left as it is a second time", () => {
		const lengths = new Uint8Array([0x00, 0x00, 0x00, 0x02, 0xaa, 0xbb]);
		const first = toAnnexB(lengths);
		if (first.type !== "ok") throw new Error(first.reason);
		expect(beginsWithStartCode(lengths)).toBe(false);
		expect(beginsWithStartCode(first.payload)).toBe(true);
		const second = toAnnexB(first.payload);
		if (second.type !== "ok") throw new Error(second.reason);
		expect(Array.from(second.payload)).toEqual(Array.from(first.payload));
		expect(second.replaced).toBe(0);
	});
});
