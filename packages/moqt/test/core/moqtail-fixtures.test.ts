import { describe, expect, test } from "bun:test";
import {
	type ControlMessage,
	decodeControlMessage,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import {
	createSubgroupStreamDecoder,
	encodeSubgroupHeader,
	encodeSubgroupObject,
	type SubgroupHeader,
	type SubgroupObject,
} from "../../src/core/subgroup.ts";
import { fixture, fixtureNames, hex, toHex, utf8 } from "../helpers.ts";

// Bytes moqtail put on the wire, read from fixtures/moqt-18/, which the Rust
// implementation reads too. The README there records how each file was
// captured and the value it decodes to. `source` names the implementation
// that encoded the bytes.
const PAYLOAD = hex("4d 3b c3 24 4c 84 57 2d");

const FIXTURES: {
	name: string;
	source: string;
	file: string;
	message: ControlMessage;
}[] = [
	{
		name: "SETUP without Setup Options",
		source: "moqtail-ts 0.13.0",
		file: "setup-moqtail-ts",
		message: { type: "SETUP", setupOptions: [] },
	},
	{
		name: "SETUP with MOQT_IMPLEMENTATION",
		source: "moqtail relay 0.15.0",
		file: "setup-moqtail-relay",
		message: {
			type: "SETUP",
			setupOptions: [{ type: 7n, value: utf8("moqtail/0.15.0+dd8623d") }],
		},
	},
	{
		name: "PUBLISH_NAMESPACE",
		source: "moqtail-ts 0.13.0",
		file: "publish-namespace-moqtail-ts",
		message: {
			type: "PUBLISH_NAMESPACE",
			requestId: 0n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			parameters: {},
		},
	},
	{
		name: "REQUEST_OK without parameters (PUBLISH_NAMESPACE_OK)",
		source: "moqtail relay 0.15.0",
		file: "request-ok-publish-namespace-moqtail-relay",
		message: { type: "REQUEST_OK", parameters: {}, trackProperties: [] },
	},
	{
		name: "PUBLISH with a 9-byte Track Alias and FORWARD",
		source: "moqtail-ts 0.13.0",
		file: "publish-moqtail-ts",
		message: {
			type: "PUBLISH",
			requestId: 2n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			trackName: utf8("object"),
			trackAlias: 779698128977928661n,
			parameters: { forward: true },
			trackProperties: [],
		},
	},
	{
		name: "REQUEST_OK with FORWARD and GROUP_ORDER (PUBLISH_OK)",
		source: "moqtail relay 0.15.0",
		file: "request-ok-publish-moqtail-relay",
		message: {
			type: "REQUEST_OK",
			parameters: { forward: true, groupOrder: "Ascending" },
			trackProperties: [],
		},
	},
	{
		name: "SUBSCRIBE with four parameters",
		source: "moqtail crate 0.15.0",
		file: "subscribe-moqtail-rs",
		message: {
			type: "SUBSCRIBE",
			requestId: 0n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			trackName: utf8("object"),
			parameters: {
				forward: true,
				subscriberPriority: 128,
				subscriptionFilter: { filterType: "LargestObject" },
				groupOrder: "Ascending",
			},
		},
	},
	{
		name: "SUBSCRIBE with three parameters, without GROUP_ORDER",
		source: "moqtail-ts 0.13.0",
		file: "subscribe-moqtail-ts",
		message: {
			type: "SUBSCRIBE",
			requestId: 0n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			trackName: utf8("object"),
			parameters: {
				forward: true,
				subscriberPriority: 128,
				subscriptionFilter: { filterType: "LargestObject" },
			},
		},
	},
	{
		name: "SUBSCRIBE_OK without parameters",
		source: "moqtail relay 0.15.0",
		file: "subscribe-ok-moqtail-relay",
		message: {
			type: "SUBSCRIBE_OK",
			trackAlias: 0n,
			parameters: {},
			trackProperties: [],
		},
	},
	{
		name: "PUBLISH with a 1-byte Track Alias",
		source: "moqtail crate 0.15.0",
		file: "publish-moqtail-rs",
		message: {
			type: "PUBLISH",
			requestId: 0n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			trackName: utf8("object"),
			trackAlias: 1n,
			parameters: { forward: true },
			trackProperties: [],
		},
	},
	{
		name: "REQUEST_ERROR DOES_NOT_EXIST for a SUBSCRIBE to a track nobody published",
		source: "moqtail relay 0.15.0",
		file: "request-error-does-not-exist-moqtail-relay",
		message: {
			type: "REQUEST_ERROR",
			errorCode: 16n,
			retryInterval: 0n,
			errorReason: "Unknown track namespace",
		},
	},
	{
		name: "PUBLISH_DONE TRACK_ENDED after the publisher disconnected",
		source: "moqtail relay 0.15.0",
		file: "publish-done-moqtail-relay",
		message: {
			type: "PUBLISH_DONE",
			statusCode: 2n,
			streamCount: 1n,
			errorReason: "Publisher disconnected",
		},
	},
	{
		name: "SUBSCRIBE_OK with LARGEST_OBJECT",
		source: "moqtail relay 0.15.0",
		file: "subscribe-ok-largest-object-moqtail-relay",
		message: {
			type: "SUBSCRIBE_OK",
			trackAlias: 0n,
			parameters: { largestObject: { group: 3n, object: 2n } },
			trackProperties: [],
		},
	},
];

// Whole subgroup streams as a subscriber received them from the relay.
// `source` names the publisher whose SUBGROUP_HEADER Type the relay forwarded;
// the relay replaced the Track Alias with its own.
const GAP_OF_ONE = [{ type: 0x3en, value: 1n }];
const SUBGROUP_FIXTURES: {
	name: string;
	source: string;
	file: string;
	header: SubgroupHeader;
	objects: SubgroupObject[];
}[] = [
	{
		name: "Type 0x5d: explicit Subgroup ID, priority, empty Object Properties",
		source: "moqtail crate 0.15.0 through the relay",
		file: "subgroup-stream-from-moqtail-rs",
		header: {
			trackAlias: 0n,
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: 0,
			properties: true,
			endOfGroup: true,
			firstObject: true,
		},
		objects: [
			{
				objectId: 0n,
				properties: [],
				status: "Normal",
				payload: utf8("xmoq-phase0 2026-10-10T01:19:03.764Z bb30f5dd09c8981f"),
			},
		],
	},
	{
		name: "Type 0x58: Subgroup ID 0 without the field, priority, no Object Properties",
		source: "moqtail-ts 0.13.0 through the relay",
		file: "subgroup-stream-from-moqtail-ts",
		header: {
			trackAlias: 0n,
			groupId: 0n,
			subgroupId: { mode: "Zero" },
			publisherPriority: 0,
			properties: false,
			endOfGroup: true,
			firstObject: true,
		},
		objects: [
			{
				objectId: 0n,
				properties: [],
				status: "Normal",
				payload: utf8("xmoq-phase0 2026-10-10T01:18:43.029Z f69e3bea2a578c6b"),
			},
		],
	},
	{
		name: "three Objects with Object ID Deltas and Prior Object ID Gap",
		source: "moqtail test client 0.15.0 through the relay",
		file: "subgroup-stream-object-id-gaps-from-moqtail-rs",
		header: {
			trackAlias: 0n,
			groupId: 4n,
			subgroupId: { mode: "Explicit", value: 1n },
			publisherPriority: 128,
			properties: true,
			endOfGroup: true,
			firstObject: true,
		},
		objects: [
			{ objectId: 0n, properties: [], status: "Normal", payload: PAYLOAD },
			{
				objectId: 2n,
				properties: GAP_OF_ONE,
				status: "Normal",
				payload: PAYLOAD,
			},
			{
				objectId: 4n,
				properties: GAP_OF_ONE,
				status: "Normal",
				payload: PAYLOAD,
			},
		],
	},
	{
		name: "a stream joined in the middle of a Group: first Object ID 4",
		source: "moqtail test client 0.15.0 through the relay",
		file: "subgroup-stream-joined-mid-group-from-moqtail-rs",
		header: {
			trackAlias: 0n,
			groupId: 3n,
			subgroupId: { mode: "Explicit", value: 1n },
			publisherPriority: 128,
			properties: true,
			endOfGroup: true,
			firstObject: true,
		},
		objects: [
			{
				objectId: 4n,
				properties: GAP_OF_ONE,
				status: "Normal",
				payload: PAYLOAD,
			},
		],
	},
];

// WHY: the tests below read the files listed in the two tables. A file added
// to fixtures/moqt-18/ and to neither table would be read by nothing, and
// nobody would notice.
test("every file of fixtures/moqt-18/ is in one of the tables, and every table entry is a file", () => {
	const listed = [...FIXTURES, ...SUBGROUP_FIXTURES].map(({ file }) => file);
	expect(listed.sort()).toEqual(fixtureNames());
});

describe("subgroup streams captured from moqtail", () => {
	test.each(SUBGROUP_FIXTURES)(
		"decodes $name ($source)",
		({ file, header, objects }) => {
			const decoder = createSubgroupStreamDecoder({
				maxObjectPayloadLength: 1024,
				maxObjectPropertiesLength: 1024,
			});
			expect(decoder.push(hex(fixture(file)))).toEqual({
				items: [
					{ type: "header", header } as const,
					...objects.map((object) => ({ type: "object", object }) as const),
				],
			});
			expect(decoder.finish()).toBeUndefined();
		},
	);

	test.each(SUBGROUP_FIXTURES)(
		"encodes $name to the bytes received",
		({ file, header, objects }) => {
			const stream = [encodeSubgroupHeader(header)];
			let previousObjectId: bigint | undefined;
			for (const object of objects) {
				stream.push(encodeSubgroupObject(header, previousObjectId, object));
				previousObjectId = object.objectId;
			}
			expect(stream.map(toHex).join(" ")).toBe(fixture(file));
		},
	);
});

describe("control messages captured from moqtail", () => {
	test.each(FIXTURES)("decodes $name ($source)", ({ file, message }) => {
		const input = hex(fixture(file));
		expect(decodeControlMessage(input)).toEqual({
			status: "ok",
			value: message,
			bytesRead: input.length,
		});
	});

	test.each(FIXTURES)(
		"encodes $name to the bytes $source sent",
		({ file, message }) => {
			expect(toHex(encodeControlMessage(message))).toBe(fixture(file));
		},
	);
});
