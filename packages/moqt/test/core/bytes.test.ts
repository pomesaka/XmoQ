import { describe, expect, test } from "bun:test";
import {
	createByteReader,
	createByteWriter,
	ShortBufferError,
} from "../../src/core/bytes.ts";
import { hex, toHex } from "../helpers.ts";

describe("varint (§1.4.1)", () => {
	// Table 2 of draft-ietf-moq-transport-18.
	test.each([
		{ bytes: "25", value: 37n },
		{ bytes: "80 25", value: 37n },
		{ bytes: "bb bd", value: 15293n },
		{ bytes: "ed 7f 3e 7d", value: 226442877n },
		{ bytes: "fa a1 a0 e4 03 d8", value: 2893212287960n },
		{ bytes: "fc 89 98 ab c6 6b c0", value: 151288809941952n },
		{ bytes: "fe fa 31 8f a8 e3 ca 11", value: 70423237261249041n },
		{ bytes: "ff ff ff ff ff ff ff ff ff", value: 18446744073709551615n },
	])("decodes $bytes", ({ bytes, value }) => {
		const reader = createByteReader(hex(bytes));
		expect(reader.varint()).toBe(value);
		expect(reader.remaining).toBe(0);
	});

	// The upper bound of each length in Table 1, and the value after it.
	test.each([
		{ value: 0n, bytes: "00" },
		{ value: 37n, bytes: "25" },
		{ value: 127n, bytes: "7f" },
		{ value: 128n, bytes: "80 80" },
		{ value: 15293n, bytes: "bb bd" },
		{ value: 16383n, bytes: "bf ff" },
		{ value: 16384n, bytes: "c0 40 00" },
		{ value: 2097151n, bytes: "df ff ff" },
		{ value: 2097152n, bytes: "e0 20 00 00" },
		{ value: 226442877n, bytes: "ed 7f 3e 7d" },
		{ value: 268435455n, bytes: "ef ff ff ff" },
		{ value: 268435456n, bytes: "f0 10 00 00 00" },
		{ value: 34359738367n, bytes: "f7 ff ff ff ff" },
		{ value: 34359738368n, bytes: "f8 08 00 00 00 00" },
		{ value: 2893212287960n, bytes: "fa a1 a0 e4 03 d8" },
		{ value: 4398046511103n, bytes: "fb ff ff ff ff ff" },
		{ value: 4398046511104n, bytes: "fc 04 00 00 00 00 00" },
		{ value: 151288809941952n, bytes: "fc 89 98 ab c6 6b c0" },
		{ value: 562949953421311n, bytes: "fd ff ff ff ff ff ff" },
		{ value: 562949953421312n, bytes: "fe 02 00 00 00 00 00 00" },
		{ value: 70423237261249041n, bytes: "fe fa 31 8f a8 e3 ca 11" },
		{ value: 72057594037927935n, bytes: "fe ff ff ff ff ff ff ff" },
		{ value: 72057594037927936n, bytes: "ff 01 00 00 00 00 00 00 00" },
		{ value: 18446744073709551615n, bytes: "ff ff ff ff ff ff ff ff ff" },
	])(
		"encodes $value with the fewest bytes and decodes it back",
		({ value, bytes }) => {
			expect(toHex(createByteWriter().varint(value).finish())).toBe(bytes);
			expect(createByteReader(hex(bytes)).varint()).toBe(value);
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
