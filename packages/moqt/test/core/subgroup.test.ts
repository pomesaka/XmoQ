import { describe, expect, test } from "bun:test";
import type { KeyValuePair } from "../../src/core/key-value-pair.ts";
import {
	createSubgroupStreamDecoder,
	encodeSubgroupHeader,
	encodeSubgroupObject,
	type ObjectStreamFailure,
	type SubgroupHeader,
	type SubgroupObject,
	type SubgroupStreamItem,
} from "../../src/core/subgroup.ts";
import { fixtureTable, hex, toHex, utf8 } from "../helpers.ts";

const LIMITS = { maxObjectPayloadLength: 1024, maxObjectPropertiesLength: 64 };

// Bytes derived by hand from §11.4.2. No other implementation produced them;
// the interop run is what checks subgroup streams against moqtail.
const STREAMS: {
	name: string;
	bytes: string;
	header: SubgroupHeader;
	objects: SubgroupObject[];
}[] = [
	{
		name: "first example of §11.6: Type 0x14, two Objects",
		bytes: "14 02 00 00 00 00 04 61 62 63 64 00 04 65 66 67 68",
		header: {
			trackAlias: 2n,
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: 0,
			properties: false,
			endOfGroup: false,
			firstObject: false,
		},
		objects: [
			{ objectId: 0n, properties: [], status: "Normal", payload: utf8("abcd") },
			{ objectId: 1n, properties: [], status: "Normal", payload: utf8("efgh") },
		],
	},
	{
		name: "Type 0x35 with Object Properties, after the second example of §11.6",
		bytes:
			"35 02 00 00 00 16 04 f0 82 57 dc d3 49 0e 74 72 61 63 65 49 44 3a 31 32 33 34 35 36 04 61 62 63 64 00 00 04 65 66 67 68",
		header: {
			trackAlias: 2n,
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: undefined,
			properties: true,
			endOfGroup: false,
			firstObject: false,
		},
		objects: [
			{
				objectId: 0n,
				properties: [
					{ type: 4n, value: 2186796243n },
					{ type: 77n, value: utf8("traceID:123456") },
				],
				status: "Normal",
				payload: utf8("abcd"),
			},
			{ objectId: 1n, properties: [], status: "Normal", payload: utf8("efgh") },
		],
	},
	{
		name: "Type 0x78: Subgroup ID 0, END_OF_GROUP, FIRST_OBJECT, an Object ID gap and End of Group",
		bytes: "78 05 09 03 01 78 02 00 03",
		header: {
			trackAlias: 5n,
			groupId: 9n,
			subgroupId: { mode: "Zero" },
			publisherPriority: undefined,
			properties: false,
			endOfGroup: true,
			firstObject: true,
		},
		objects: [
			{ objectId: 3n, properties: [], status: "Normal", payload: utf8("x") },
			{
				objectId: 6n,
				properties: [],
				status: "EndOfGroup",
				payload: new Uint8Array(0),
			},
		],
	},
	{
		name: "Type 0x12: Subgroup ID from the first Object, a zero-length Normal Object",
		bytes: "12 01 00 80 07 00 00",
		header: {
			trackAlias: 1n,
			groupId: 0n,
			subgroupId: { mode: "FirstObjectId" },
			publisherPriority: 128,
			properties: false,
			endOfGroup: false,
			firstObject: false,
		},
		objects: [
			{
				objectId: 7n,
				properties: [],
				status: "Normal",
				payload: new Uint8Array(0),
			},
		],
	},
];

function expectedItems(
	header: SubgroupHeader,
	objects: SubgroupObject[],
): SubgroupStreamItem[] {
	return [
		{ type: "header", header },
		...objects.map(
			(object): SubgroupStreamItem => ({ type: "object", object }),
		),
	];
}

// fixtures/moqt-18-verdicts/subgroup-stream.tsv, which the Rust
// implementation reads as well: a subgroup stream, whole and a byte at a
// time, ended with a FIN, and whether it is read, closes the session, makes
// the track malformed, or ends that stream alone (a limit of this end). §11.4:
// a FIN inside a serialized Object, or inside the header, is a
// PROTOCOL_VIOLATION. §2.4.2 and §11.4.3: an Object after End of Group or End
// of Track makes the track malformed as soon as its first byte is there.
// §2.5.1, §12.8 and §12.9: a Mandatory Track Property on an Object, and a
// Prior Group ID Gap or Prior Object ID Gap that is larger than its ID or
// comes twice, make it malformed, decided when the properties are read, before
// the status that would close the session (an Object is judged field by field
// as it arrives). More than MAX_KEY_VALUE_PAIRS properties are this end's
// limit.
describe("fixtures/moqt-18-verdicts/subgroup-stream.tsv", () => {
	const rows = fixtureTable("moqt-18-verdicts", "subgroup-stream.tsv");
	test("the table has its 23 rows of 3 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(23).fill(3));
	});
	test.each(rows)("%s: %s is %s", (_name, bytes, verdict) => {
		const input = hex((bytes ?? "").match(/../g)?.join(" ") ?? "");
		for (const chunk of [Math.max(input.length, 1), 1]) {
			const decoder = createSubgroupStreamDecoder(LIMITS);
			let failure: ObjectStreamFailure | undefined;
			for (
				let at = 0;
				at < input.length && failure === undefined;
				at += chunk
			) {
				failure = decoder.push(input.subarray(at, at + chunk)).failure;
			}
			const violation = failure === undefined ? decoder.finish() : undefined;
			const got =
				failure === undefined
					? violation === undefined
						? "ok"
						: `closed:0x${violation.code.toString(16)}`
					: failure.scope === "session"
						? `closed:0x${failure.error.code.toString(16)}`
						: failure.scope === "track"
							? "malformed"
							: "stopped";
			expect(got).toBe(verdict ?? "");
		}
	});
});

describe("subgroup streams (§11.4.2)", () => {
	test.each(STREAMS)(
		"decodes in one chunk: $name",
		({ bytes, header, objects }) => {
			const decoder = createSubgroupStreamDecoder(LIMITS);
			expect(decoder.push(hex(bytes))).toEqual({
				items: expectedItems(header, objects),
			});
			expect(decoder.finish()).toBeUndefined();
		},
	);

	test.each(STREAMS)(
		"decodes byte by byte: $name",
		({ bytes, header, objects }) => {
			const decoder = createSubgroupStreamDecoder(LIMITS);
			const items: SubgroupStreamItem[] = [];
			for (const byte of hex(bytes)) {
				const result = decoder.push(Uint8Array.of(byte));
				expect(result.failure).toBeUndefined();
				items.push(...result.items);
			}
			expect(items).toEqual(expectedItems(header, objects));
			expect(decoder.finish()).toBeUndefined();
		},
	);

	test.each(STREAMS)("encodes: $name", ({ bytes, header, objects }) => {
		const parts = [encodeSubgroupHeader(header)];
		let previous: bigint | undefined;
		for (const object of objects) {
			parts.push(encodeSubgroupObject(header, previous, object));
			previous = object.objectId;
		}
		expect(parts.map(toHex).join(" ")).toBe(bytes);
	});

	test.each([
		{ name: "Object Payload Length 1025", bytes: "14 02 00 00 00 00 84 01" },
		{
			name: "Object Properties Length 65",
			bytes: "15 02 00 00 00 00 80 41",
		},
	])(
		"ends the stream, not the session, at a local limit: $name",
		({ bytes }) => {
			const decoder = createSubgroupStreamDecoder(LIMITS);
			expect(decoder.push(hex(bytes)).failure).toMatchObject({
				scope: "stream",
			});
			expect(decoder.push(hex("00"))).toEqual({ items: [] });
		},
	);

	test("ends the stream at more than 64 Object Properties", () => {
		const decoder = createSubgroupStreamDecoder({
			maxObjectPayloadLength: 16,
			maxObjectPropertiesLength: 1024,
		});
		// 65 properties of Type 2, 4, 6, ... with value 0, in 130 (0x8082) bytes.
		const result = decoder.push(
			hex(`15 02 00 00 00 00 80 82 ${"02 00 ".repeat(65)}`),
		);
		expect(result.failure).toEqual({
			scope: "stream",
			reason: "more than 64 Key-Value-Pairs in one sequence",
		});
	});

	test.each([
		{
			name: "an Object ID that does not ascend",
			previous: 1n,
			object: {
				objectId: 1n,
				properties: [],
				status: "Normal",
				payload: utf8("a"),
			},
		},
		{
			name: "a payload on an End of Group Object",
			previous: undefined,
			object: {
				objectId: 0n,
				properties: [],
				status: "EndOfGroup",
				payload: utf8("a"),
			},
		},
		{
			name: "properties without the PROPERTIES bit",
			previous: undefined,
			object: {
				objectId: 0n,
				properties: [{ type: 2n, value: 1n }],
				status: "Normal",
				payload: utf8("a"),
			},
		},
	] satisfies {
		name: string;
		previous: bigint | undefined;
		object: SubgroupObject;
	}[])("refuses to encode $name", ({ previous, object }) => {
		expect(() =>
			encodeSubgroupObject(
				{ properties: false, groupId: 0n },
				previous,
				object,
			),
		).toThrow(RangeError);
	});
});

// What the decoder takes for a malformed Track, the encoder does not write
// (§2.5.1, §12.8, §12.9). The Rust encoder refuses the same.
describe("Object Properties that break the track", () => {
	test.each([
		{
			name: "a Mandatory Track Property",
			objectId: 0n,
			properties: [{ type: 0x4000n, value: 0n }],
		},
		{
			name: "a Prior Object ID Gap larger than the Object ID",
			objectId: 0n,
			properties: [{ type: 0x3en, value: 1n }],
		},
		{
			name: "two Prior Object ID Gaps",
			objectId: 5n,
			properties: [
				{ type: 0x3en, value: 1n },
				{ type: 0x3en, value: 1n },
			],
		},
		{
			name: "a Prior Group ID Gap larger than the Group ID",
			objectId: 0n,
			properties: [{ type: 0x3cn, value: 1n }],
		},
		{
			name: "a Prior Object ID Gap inside Immutable Properties larger than the Object ID (§12.7)",
			objectId: 0n,
			properties: [{ type: 0x0bn, value: Uint8Array.from([0x3e, 0x01]) }],
		},
		{
			name: "Immutable Properties inside Immutable Properties (§12.7)",
			objectId: 0n,
			properties: [{ type: 0x0bn, value: Uint8Array.from([0x0b, 0x00]) }],
		},
		{
			name: "a Type over 2^64 - 1 inside Immutable Properties (§1.4.3)",
			objectId: 0n,
			properties: [
				{
					type: 0x0bn,
					value: hex("ff ff ff ff ff ff ff ff fe 00 02 00"),
				},
			],
		},
	] satisfies {
		name: string;
		objectId: bigint;
		properties: KeyValuePair[];
	}[])("encodeSubgroupObject refuses $name", ({ objectId, properties }) => {
		expect(() =>
			encodeSubgroupObject({ properties: true, groupId: 0n }, undefined, {
				objectId,
				properties: [...properties],
				status: "Normal",
				payload: new Uint8Array(0),
			}),
		).toThrow(RangeError);
	});
});
