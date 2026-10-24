import { expect, test } from "bun:test";
import { readLocTiming } from "@xmoq/loc";
import { locProperties } from "../src/moqt/loc-properties.ts";

// The Timestamp of LOC-04 is 0x10 (`packages/loc`); Immutable Properties is
// 0xB (transport §12.7). The inside of an Immutable Properties is a
// sequence of its own, written by hand here: `10 05` is Type 0x10 and the
// value 5; `0b 00` is an Immutable Properties of no bytes.
const IMMUTABLE = 0xbn;
const inside = (...bytes: number[]) => ({
	type: IMMUTABLE,
	value: new Uint8Array(bytes),
});

test.each([
	["the Timestamp outside", [{ type: 0x10n, value: 7n }], "ok 7"],
	["the Timestamp inside Immutable Properties", [inside(0x10, 0x05)], "ok 5"],
	[
		"a Timestamp outside and inside: two of them",
		[{ type: 0x10n, value: 7n }, inside(0x10, 0x05)],
		"invalid the Timestamp comes twice",
	],
	["no Timestamp anywhere", [inside()], "ok none"],
	[
		"Immutable Properties inside Immutable Properties: read as it came",
		[{ type: 0x10n, value: 7n }, inside(0x0b, 0x00)],
		"ok 7",
	],
])("%s", (_case, properties, expected) => {
	const read = readLocTiming(locProperties(properties));
	const said =
		read.type === "invalid"
			? `invalid ${read.reason}`
			: `ok ${read.timing === undefined ? "none" : String(read.timing.timestamp)}`;
	expect(said).toBe(expected);
});
