import { describe, expect, test } from "bun:test";
import {
	createSubgroupStreamDecoder,
	encodeSubgroupHeader,
	encodeSubgroupObject,
	type SubgroupHeader,
	type SubgroupObject,
	type SubgroupStreamItem,
} from "../../src/core/subgroup.ts";
import { hex, toHex, utf8 } from "../helpers.ts";

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

	// A broken rule of the draft ends the session.
	test.each([
		{ name: "SUBGROUP_ID_MODE 0b11 (Type 0x16)", bytes: "16 00 00" },
		{ name: "a Type without bit 4 (0x04)", bytes: "04 00 00" },
		{ name: "Object Status 1", bytes: "14 02 00 00 00 00 00 01" },
		{
			name: "properties on an End of Group Object",
			bytes: "15 02 00 00 00 00 02 04 01 00 03",
		},
		{
			name: "an Object ID above 2^64 - 1",
			bytes: "34 00 00 00 ff ff ff ff ff ff ff ff ff 01 61 00 01 61",
		},
		{
			name: "Object Properties that end inside a pair",
			bytes: "15 02 00 00 00 00 01 05",
		},
	])("reports a PROTOCOL_VIOLATION for $name", ({ bytes }) => {
		const { failure } = createSubgroupStreamDecoder(LIMITS).push(hex(bytes));
		expect(failure).toMatchObject({ scope: "session", error: { code: 3 } });
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

	// A malformed Track (§2.4.2) ends its subscription, not the session.
	// Object Properties in these rows: 0x4000 is a Mandatory Track Property,
	// 0x3C is Prior Group ID Gap, 0x3E is Prior Object ID Gap.
	test.each([
		{
			name: "a Mandatory Track Property on an Object (§2.5.1)",
			bytes: "15 02 00 00 00 00 04 c0 40 00 01 01 61",
			reason: "Mandatory Track Property 0x4000 on an Object",
		},
		{
			// Fields are judged in the order they arrive: the properties come
			// before the status that makes them a PROTOCOL_VIOLATION.
			name: "a Mandatory Track Property on an End of Group Object",
			bytes: "15 02 00 00 00 00 04 c0 40 00 01 00 03",
			reason: "Mandatory Track Property 0x4000 on an Object",
		},
		{
			name: "the first byte of an Object after End of Group, before its Object ID Delta is whole",
			bytes: "14 02 00 00 00 00 00 03 80",
			reason: "an Object follows the end of its Group or Track",
		},
		{
			name: "an Object after End of Group (§2.4.2)",
			bytes: "14 02 00 00 00 00 00 03 00 01 61",
			reason: "an Object follows the end of its Group or Track",
		},
		{
			name: "two Prior Object ID Gap properties (§12.9)",
			bytes: "15 02 05 00 00 03 04 3e 00 00 00 01 61",
			reason: "more than one Prior Object ID Gap on an Object",
		},
		{
			name: "Prior Object ID Gap 4 on Object 3 (§12.9)",
			bytes: "15 02 05 00 00 03 02 3e 04 01 61",
			reason: "Prior Object ID Gap 4 is larger than Object ID 3",
		},
		{
			name: "Prior Group ID Gap 6 in Group 5 (§12.8)",
			bytes: "15 02 05 00 00 03 02 3c 06 01 61",
			reason: "Prior Group ID Gap 6 is larger than Group ID 5",
		},
	])("reports a malformed Track for $name", ({ bytes, reason }) => {
		const { failure } = createSubgroupStreamDecoder(LIMITS).push(hex(bytes));
		expect(failure).toEqual({ scope: "track", reason });
	});

	test("accepts gap properties that fit their Group and Object (§12.8, §12.9)", () => {
		// Group 5, Object 3, Prior Group ID Gap 5 and Prior Object ID Gap 3.
		const result = createSubgroupStreamDecoder(LIMITS).push(
			hex("15 02 05 00 00 03 04 3c 05 02 03 01 61"),
		);
		expect(result.failure).toBeUndefined();
		expect(result.items).toHaveLength(2);
	});

	// §11.4: a FIN in the middle of a serialized Object is a PROTOCOL_VIOLATION.
	test.each([
		{ name: "after the header", bytes: "14 02 00 00 00", violation: false },
		{
			name: "after a whole Object",
			bytes: "14 02 00 00 00 00 01 61",
			violation: false,
		},
		{ name: "inside the header", bytes: "14 02", violation: true },
		{
			name: "inside the Object fields",
			bytes: "14 02 00 00 00 00",
			violation: true,
		},
		{
			name: "inside the Object Payload",
			bytes: "14 02 00 00 00 00 02 61",
			violation: true,
		},
	])("FIN $name", ({ bytes, violation }) => {
		const decoder = createSubgroupStreamDecoder(LIMITS);
		decoder.push(hex(bytes));
		expect(decoder.finish() !== undefined).toBe(violation);
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
	])("encodeSubgroupObject refuses $name", ({ objectId, properties }) => {
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
