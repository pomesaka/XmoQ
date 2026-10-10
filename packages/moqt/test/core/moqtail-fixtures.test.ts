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
import { hex, toHex, utf8 } from "../helpers.ts";

// Control messages produced by moqtail implementations, copied from the hex the
// moqtail relay 0.15.0 (moqtail/0.15.0+dd8623d) logs for every control message.
// `source` names the implementation that encoded the bytes.
//
// The log shows the bytes of the wire, not a re-encoding: the relay prints what
// it read from the stream and what it is about to write
// (libs/moqtail-rs/src/transport/control_stream_handler.rs at tag relay@0.15.0,
// lines 84-90 and 175-179, read with `gh api` on 2026-10-10).
//
// FIXME: move these byte fixtures to a place the Rust crates share, so both
// implementations are pinned to the same bytes.
const FIXTURES: {
	name: string;
	source: string;
	bytes: string;
	message: ControlMessage;
}[] = [
	{
		name: "SETUP without Setup Options",
		source: "moqtail-ts 0.13.0",
		bytes: "af 00 00 00",
		message: { type: "SETUP", setupOptions: [] },
	},
	{
		name: "SETUP with MOQT_IMPLEMENTATION",
		source: "moqtail relay 0.15.0",
		bytes:
			"af 00 00 18 07 16 6d 6f 71 74 61 69 6c 2f 30 2e 31 35 2e 30 2b 64 64 38 36 32 33 64",
		message: {
			type: "SETUP",
			setupOptions: [{ type: 7n, value: utf8("moqtail/0.15.0+dd8623d") }],
		},
	},
	{
		name: "PUBLISH_NAMESPACE",
		source: "moqtail-ts 0.13.0",
		bytes: "06 00 0f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 00",
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
		bytes: "07 00 01 00",
		message: { type: "REQUEST_OK", parameters: {}, trackProperties: [] },
	},
	{
		name: "PUBLISH with a 9-byte Track Alias and FORWARD",
		source: "moqtail-ts 0.13.0",
		bytes:
			"1d 00 21 02 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 ff 0a d2 0b 51 39 c4 21 d5 01 10 01",
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
		bytes: "07 00 05 02 10 01 12 01",
		message: {
			type: "REQUEST_OK",
			parameters: { forward: true, groupOrder: "Ascending" },
			trackProperties: [],
		},
	},
	{
		name: "SUBSCRIBE with four parameters",
		source: "moqtail crate 0.15.0",
		bytes:
			"03 00 1f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 04 10 01 10 80 01 01 02 01 01",
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
		bytes:
			"03 00 1d 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 03 10 01 10 80 01 01 02",
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
		bytes: "04 00 02 00 00",
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
		bytes:
			"1d 00 19 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 01 01 10 01",
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
		bytes:
			"05 00 1a 10 00 17 55 6e 6b 6e 6f 77 6e 20 74 72 61 63 6b 20 6e 61 6d 65 73 70 61 63 65",
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
		bytes:
			"0b 00 19 02 01 16 50 75 62 6c 69 73 68 65 72 20 64 69 73 63 6f 6e 6e 65 63 74 65 64",
		message: {
			type: "PUBLISH_DONE",
			statusCode: 2n,
			streamCount: 1n,
			errorReason: "Publisher disconnected",
		},
	},
];

// Whole subgroup streams as the own browser subscriber received them from the
// relay. `source` names the publisher whose SUBGROUP_HEADER Type the relay
// forwarded; the relay replaced the Track Alias with its own.
const SUBGROUP_FIXTURES: {
	name: string;
	source: string;
	bytes: string;
	header: SubgroupHeader;
	object: SubgroupObject;
}[] = [
	{
		name: "Type 0x5d: explicit Subgroup ID, priority, empty Object Properties",
		source: "moqtail crate 0.15.0 through the relay",
		bytes:
			"5d 00 00 00 00 00 00 36 78 6d 6f 71 2d 70 68 61 73 65 30 62 20 32 30 32 36 2d 31 30 2d 31 30 54 30 31 3a 32 38 3a 31 31 2e 32 39 38 5a 20 63 63 65 63 36 63 32 35 65 63 37 63 34 34 36 65",
		header: {
			trackAlias: 0n,
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: 0,
			properties: true,
			endOfGroup: true,
			firstObject: true,
		},
		object: {
			objectId: 0n,
			properties: [],
			status: "Normal",
			payload: utf8("xmoq-phase0b 2026-10-10T01:28:11.298Z ccec6c25ec7c446e"),
		},
	},
	{
		name: "Type 0x58: Subgroup ID 0 without the field, priority, no Object Properties",
		source: "moqtail-ts 0.13.0 through the relay",
		bytes:
			"58 00 00 00 00 36 78 6d 6f 71 2d 70 68 61 73 65 30 62 20 32 30 32 36 2d 31 30 2d 31 30 54 30 31 3a 32 38 3a 32 35 2e 30 36 34 5a 20 37 65 39 35 35 62 31 39 36 32 30 36 64 35 35 31",
		header: {
			trackAlias: 0n,
			groupId: 0n,
			subgroupId: { mode: "Zero" },
			publisherPriority: 0,
			properties: false,
			endOfGroup: true,
			firstObject: true,
		},
		object: {
			objectId: 0n,
			properties: [],
			status: "Normal",
			payload: utf8("xmoq-phase0b 2026-10-10T01:28:25.064Z 7e955b196206d551"),
		},
	},
];

describe("subgroup streams captured from moqtail", () => {
	test.each(SUBGROUP_FIXTURES)(
		"decodes $name ($source)",
		({ bytes, header, object }) => {
			const decoder = createSubgroupStreamDecoder({
				maxObjectPayloadLength: 1024,
				maxObjectPropertiesLength: 1024,
			});
			expect(decoder.push(hex(bytes))).toEqual({
				items: [
					{ type: "header", header },
					{ type: "object", object },
				],
			});
			expect(decoder.finish()).toBeUndefined();
		},
	);

	test.each(SUBGROUP_FIXTURES)(
		"encodes $name to the bytes received",
		({ bytes, header, object }) => {
			const stream = [
				encodeSubgroupHeader(header),
				encodeSubgroupObject(header, undefined, object),
			];
			expect(stream.map(toHex).join(" ")).toBe(bytes);
		},
	);
});

describe("control messages captured from moqtail", () => {
	test.each(FIXTURES)("decodes $name ($source)", ({ bytes, message }) => {
		const input = hex(bytes);
		expect(decodeControlMessage(input)).toEqual({
			status: "ok",
			value: message,
			bytesRead: input.length,
		});
	});

	test.each(FIXTURES)(
		"encodes $name to the bytes $source sent",
		({ bytes, message }) => {
			expect(toHex(encodeControlMessage(message))).toBe(bytes);
		},
	);
});
