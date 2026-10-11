import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { toAnnexB } from "../src/core/video-payload.ts";

const FIXTURES = join(import.meta.dir, "..", "..", "..", "fixtures", "loc-04");

/** Bytes written as pairs of hex digits, with spaces between or without. */
function hex(text: string): Uint8Array {
	const digits = text.replaceAll(/\s+/g, "");
	if (!/^(?:[0-9a-f]{2})*$/.test(digits)) throw new Error(`not hex: ${text}`);
	return Uint8Array.from(digits.match(/../g) ?? [], (pair) =>
		Number.parseInt(pair, 16),
	);
}
const toHex = (bytes: Uint8Array): string =>
	Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(" ");
const fixture = (name: string): Uint8Array =>
	hex(readFileSync(join(FIXTURES, `${name}.hex`), "utf8"));

describe("fixtures/loc-04: the video payloads of moqlivemock's publisher have a length before every NAL unit", () => {
	// Where each NAL unit begins after its 4 bytes, and its first byte: for
	// H.264, 0x67 is a sequence parameter set, 0x68 a picture parameter
	// set, 0x65 a slice of an IDR picture, 0x41 a slice of another one.
	test.each([
		{
			file: "video-key-length-prefixed-from-moqlivemock",
			length: 21533,
			replaced: 3,
			units: [
				{ at: 0, first: 0x67 },
				{ at: 29, first: 0x68 },
				{ at: 37, first: 0x65 },
			],
		},
		{
			file: "video-delta-length-prefixed-from-moqlivemock",
			length: 931,
			replaced: 1,
			units: [{ at: 0, first: 0x41 }],
		},
	])(
		"$file: every length becomes a start code, and nothing else changes",
		({ file, length, replaced, units }) => {
			const payload = fixture(file);
			expect(payload.length).toBe(length);
			const converted = toAnnexB(payload);
			if (converted.type !== "ok") throw new Error(converted.reason);
			expect(converted.replaced).toBe(replaced);
			expect(converted.payload.length).toBe(length);
			const expected = new Uint8Array(payload);
			for (const { at, first } of units) {
				expected.set([0, 0, 0, 1], at);
				expect(payload[at + 4]).toBe(first);
			}
			expect(converted.payload).toEqual(expected);
			// The input is not written to.
			expect(payload).toEqual(fixture(file));
		},
	);

	test("the key frame's first length is 25, the bytes of its sequence parameter set", () => {
		expect(
			toHex(
				fixture("video-key-length-prefixed-from-moqlivemock").subarray(0, 6),
			),
		).toBe("00 00 00 19 67 4d");
	});
});

describe("toAnnexB", () => {
	test.each([
		{
			name: "one NAL unit with a length",
			payload: "00 00 00 02 aa bb",
			result: "00 00 00 01 aa bb",
			replaced: 1,
		},
		{
			name: "two NAL units with lengths",
			payload: "00 00 00 03 67 01 02 00 00 00 02 aa bb",
			result: "00 00 00 01 67 01 02 00 00 00 01 aa bb",
			replaced: 2,
		},
		{
			name: "a payload that has start codes of 4 bytes is as it was",
			payload: "00 00 00 01 67 aa 00 00 00 01 68 bb",
			result: "00 00 00 01 67 aa 00 00 00 01 68 bb",
			replaced: 0,
		},
		{
			name: "a length of 1 is a start code: what follows it is left alone",
			payload: "00 00 00 02 aa bb 00 00 00 01 ff ff ff ff ff",
			result: "00 00 00 01 aa bb 00 00 00 01 ff ff ff ff ff",
			replaced: 1,
		},
		{
			name: "a length after a length of 1 stays a length",
			payload: "00 00 00 02 aa bb 00 00 00 01 cc 00 00 00 02 dd ee",
			result: "00 00 00 01 aa bb 00 00 00 01 cc 00 00 00 02 dd ee",
			replaced: 1,
		},
		{
			name: "a start code alone",
			payload: "00 00 00 01",
			result: "00 00 00 01",
			replaced: 0,
		},
		{
			name: "bytes inside a NAL unit that look like a start code are copied",
			payload: "00 00 00 06 65 00 00 00 01 aa",
			result: "00 00 00 01 65 00 00 00 01 aa",
			replaced: 1,
		},
		{
			name: "a NAL unit to the last byte",
			payload: "00 00 00 03 01 02 03",
			result: "00 00 00 01 01 02 03",
			replaced: 1,
		},
		{ name: "no bytes", payload: "", result: "", replaced: 0 },
	])("$name", ({ payload, result, replaced }) => {
		expect(toAnnexB(hex(payload))).toEqual({
			type: "ok",
			payload: hex(result),
			replaced,
		});
	});

	test.each([
		{
			name: "a length of 0",
			payload: "00 00 00 00 aa",
			reason: "a NAL unit of no bytes at 0",
		},
		{
			name: "a length one byte beyond the payload",
			payload: "00 00 00 03 aa bb",
			reason: "a NAL unit of 3 bytes at 0, where 2 are left",
		},
		{
			name: "a length beyond the payload",
			payload: "00 00 00 05 aa bb",
			reason: "a NAL unit of 5 bytes at 0, where 2 are left",
		},
		{
			name: "a second length beyond the payload",
			payload: "00 00 00 02 aa bb 00 00 00 09 cc",
			reason: "a NAL unit of 9 bytes at 6, where 1 are left",
		},
		{
			name: "fewer than four bytes where a length is to begin",
			payload: "00 00 00 02 aa bb 00 00",
			reason: "2 bytes at 6, where a length of 4 bytes is to begin",
		},
		{
			name: "fewer than four bytes in all",
			payload: "00 00 01",
			reason: "3 bytes at 0, where a length of 4 bytes is to begin",
		},
		{
			// §2.1.4 allows it for a track that never has length prefixes;
			// here its first four bytes are a length of 359 bytes.
			name: "a start code of 3 bytes, which reads as a length",
			payload: "00 00 01 67 aa bb",
			reason: "a NAL unit of 359 bytes at 0, where 2 are left",
		},
	])("$name is no payload of either form", ({ payload, reason }) => {
		expect(toAnnexB(hex(payload))).toEqual({ type: "invalid", reason });
	});

	test("a start code of 3 bytes whose first four bytes are the number of bytes after them is `ok`, and its NAL unit has lost its first byte", () => {
		// 00 00 01, and a NAL unit of 266 bytes that begins with 09: read as
		// the length 0x109, which is the 265 bytes that follow.
		const payload = new Uint8Array(269).fill(0xaa);
		payload.set([0x00, 0x00, 0x01, 0x09]);
		const read = toAnnexB(payload);
		if (read.type !== "ok") throw new Error(read.reason);
		expect(read.replaced).toBe(1);
		expect(Array.from(read.payload.subarray(0, 6))).toEqual([
			0x00, 0x00, 0x00, 0x01, 0xaa, 0xaa,
		]);
	});

	test("the result is a new array over a buffer of its own, and the payload that was given is as it was", () => {
		const payload = new Uint8Array([0x00, 0x00, 0x00, 0x01, 0x67]);
		const lengths = new Uint8Array([0x00, 0x00, 0x00, 0x02, 0xaa, 0xbb]);
		for (const given of [payload, lengths]) {
			const before = Array.from(given);
			const read = toAnnexB(given);
			if (read.type !== "ok") throw new Error(read.reason);
			expect(read.payload).not.toBe(given);
			expect(read.payload.buffer).not.toBe(given.buffer);
			expect(Array.from(given)).toEqual(before);
		}
	});

	test("a payload that is a view into a larger buffer is read from where it begins", () => {
		const whole = hex("ff ff 00 00 00 02 aa bb ff");
		expect(toAnnexB(whole.subarray(2, 8))).toEqual({
			type: "ok",
			payload: hex("00 00 00 01 aa bb"),
			replaced: 1,
		});
	});
});
