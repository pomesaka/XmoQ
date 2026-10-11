import { describe, expect, test } from "bun:test";
import type { LocProperty } from "../../src/core/revision.ts";
import {
	locProperties,
	microsecondsOf,
	readLocTiming,
} from "../../src/draft-04/properties.ts";

describe("locProperties: the Object Properties of a frame", () => {
	test("a timestamp alone is the Timestamp, 0x10", () => {
		expect(locProperties({ timestamp: 1791690527000000n })).toEqual([
			{ type: 0x10n, value: 1791690527000000n },
		]);
	});

	test("with a timescale, the Timescale, 0x08, comes first: the order of the types", () => {
		expect(locProperties({ timestamp: 3000n, timescale: 90000n })).toEqual([
			{ type: 0x08n, value: 90000n },
			{ type: 0x10n, value: 3000n },
		]);
	});

	test.each([
		{ name: "a negative timestamp", timing: { timestamp: -1n } },
		{
			name: "a timestamp of 2^64",
			timing: { timestamp: 18446744073709551616n },
		},
		{ name: "a timescale of 0", timing: { timestamp: 1n, timescale: 0n } },
		{ name: "a negative timescale", timing: { timestamp: 1n, timescale: -1n } },
		{
			name: "a timescale of 2^64",
			timing: { timestamp: 1n, timescale: 18446744073709551616n },
		},
	])("$name is a RangeError", ({ timing }) => {
		expect(() => locProperties(timing)).toThrow(RangeError);
	});

	test("a timestamp of 0 is taken", () => {
		expect(locProperties({ timestamp: 0n })).toEqual([
			{ type: 0x10n, value: 0n },
		]);
	});

	test("the largest timestamp is taken", () => {
		expect(locProperties({ timestamp: 18446744073709551615n })).toEqual([
			{ type: 0x10n, value: 18446744073709551615n },
		]);
	});
});

describe("readLocTiming: what the Object Properties say of the time", () => {
	test.each<{ name: string; properties: LocProperty[]; read: unknown }>([
		{
			// As on the first Object of fixtures/loc-04: the one property
			// moqlivemock's publisher sends.
			name: "a Timestamp alone",
			properties: [{ type: 0x10n, value: 1791690527000000n }],
			read: { type: "ok", timing: { timestamp: 1791690527000000n } },
		},
		{
			name: "a Timescale and a Timestamp",
			properties: [
				{ type: 0x08n, value: 90000n },
				{ type: 0x10n, value: 3000n },
			],
			read: { type: "ok", timing: { timestamp: 3000n, timescale: 90000n } },
		},
		{
			name: "the two in the other order",
			properties: [
				{ type: 0x10n, value: 3000n },
				{ type: 0x08n, value: 90000n },
			],
			read: { type: "ok", timing: { timestamp: 3000n, timescale: 90000n } },
		},
		{
			name: "no property",
			properties: [],
			read: { type: "ok", timing: undefined },
		},
		{
			name: "properties that are not read: Video Frame Marking, Video Config, one LOC does not define",
			properties: [
				{ type: 0x09n, value: new Uint8Array([0x80]) },
				{ type: 0x0dn, value: new Uint8Array([1, 2, 3]) },
				{ type: 0x10n, value: 7n },
				{ type: 0x3cn, value: 1n },
			],
			read: { type: "ok", timing: { timestamp: 7n } },
		},
		{
			name: "a Timestamp twice",
			properties: [
				{ type: 0x10n, value: 1n },
				{ type: 0x10n, value: 2n },
			],
			read: { type: "invalid", reason: "the Timestamp comes twice" },
		},
		{
			name: "a Timescale twice",
			properties: [
				{ type: 0x08n, value: 1n },
				{ type: 0x08n, value: 1n },
				{ type: 0x10n, value: 2n },
			],
			read: { type: "invalid", reason: "the Timescale comes twice" },
		},
		{
			name: "a Timescale of 0",
			properties: [
				{ type: 0x08n, value: 0n },
				{ type: 0x10n, value: 2n },
			],
			read: { type: "invalid", reason: "the Timescale is 0" },
		},
		{
			name: "a Timescale without a Timestamp",
			properties: [{ type: 0x08n, value: 90000n }],
			read: { type: "invalid", reason: "a Timescale without a Timestamp" },
		},
		{
			name: "a Timestamp that is bytes",
			properties: [{ type: 0x10n, value: new Uint8Array([1]) }],
			read: { type: "invalid", reason: "the Timestamp is no integer" },
		},
	])("$name", ({ properties, read }) => {
		expect<unknown>(readLocTiming(properties)).toEqual(read);
	});

	test("what is written is read back", () => {
		const timing = { timestamp: 3000n, timescale: 90000n };
		expect(readLocTiming(locProperties(timing))).toEqual({
			type: "ok",
			timing,
		});
	});
});

describe("microsecondsOf: the time in the unit of WebCodecs", () => {
	test.each([
		{
			name: "without a timescale, the timestamp itself",
			timing: { timestamp: 1791690527040000n },
			microseconds: 1791690527040000n,
		},
		{
			name: "90 kHz",
			timing: { timestamp: 3000n, timescale: 90000n },
			microseconds: 33333n,
		},
		{
			name: "milliseconds",
			timing: { timestamp: 40n, timescale: 1000n },
			microseconds: 40000n,
		},
		{
			name: "a timescale finer than a microsecond, rounded down",
			timing: { timestamp: 2999n, timescale: 1000000000n },
			microseconds: 2n,
		},
	])("$name", ({ timing, microseconds }) => {
		expect(microsecondsOf(timing)).toBe(microseconds);
	});
});
