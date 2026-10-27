import { describe, expect, test } from "bun:test";
import {
	createByteReader,
	createByteWriter,
	ShortBufferError,
} from "../../src/core/bytes.ts";
import { hex, toHex, varintFixtures } from "../helpers.ts";

describe("varint (§1.4.1)", () => {
	// fixtures/moqt-18-by-hand/varints.tsv, which the Rust implementation
	// reads as well: Table 2 of §1.4.1 and the edges of each length of
	// Table 1. §1.4.1: "any encoding length that can represent the value is
	// valid", and the encoder writes the fewest.
	test("the table has its 26 rows", () => {
		expect(varintFixtures()).toHaveLength(26);
	});

	test.each(varintFixtures())(
		"$bytes is $value, the fewest bytes for it: $fewest",
		({ bytes, value, fewest }) => {
			const reader = createByteReader(hex(bytes));
			expect(reader.varint()).toBe(value);
			expect(reader.remaining).toBe(0);
			expect(toHex(createByteWriter().varint(value).finish()) === bytes).toBe(
				fewest,
			);
		},
	);

	test.each([
		{ name: "empty input", bytes: "" },
		{ name: "2-byte form cut short", bytes: "80" },
		{ name: "9-byte form cut short", bytes: "ff 00 00 00 00 00 00 00" },
	])("reports a short buffer: $name", ({ bytes }) => {
		expect(() => createByteReader(hex(bytes)).varint()).toThrow(
			ShortBufferError,
		);
	});

	test.each([{ value: -1n }, { value: 18446744073709551616n }])(
		"refuses to encode $value",
		({ value }) => {
			expect(() => createByteWriter().varint(value)).toThrow(RangeError);
		},
	);
});
