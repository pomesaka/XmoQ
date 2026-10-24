import { describe, expect, test } from "bun:test";
import {
	type ControlMessage,
	decodeControlMessage,
	decodeRequestStart,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import type { MessageParameters } from "../../src/core/parameters.ts";
import { hex, toHex, utf8 } from "../helpers.ts";

// Bytes in this file are derived by hand from the message formats of draft 18.
// Bytes captured from another implementation are in moqtail-fixtures.test.ts.

const HAND_ENCODED: { name: string; bytes: string; message: ControlMessage }[] =
	[
		{
			name: "REQUEST_ERROR DOES_NOT_EXIST (§10.6.2)",
			bytes: "05 00 05 10 00 02 6e 6f",
			message: {
				type: "REQUEST_ERROR",
				errorCode: 16n,
				retryInterval: 0n,
				errorReason: "no",
			},
		},
		{
			name: "REQUEST_ERROR REDIRECT with a Redirect structure (§10.6.1)",
			bytes: "05 00 0e 34 01 00 05 68 3a 2f 2f 61 01 01 62 01 63",
			message: {
				type: "REQUEST_ERROR",
				errorCode: 52n,
				retryInterval: 1n,
				errorReason: "",
				redirect: {
					connectUri: "h://a",
					trackNamespace: [utf8("b")],
					trackName: utf8("c"),
				},
			},
		},
		{
			name: "PUBLISH_DONE TRACK_ENDED (§10.11)",
			bytes: "0b 00 06 02 01 03 65 6e 64",
			message: {
				type: "PUBLISH_DONE",
				statusCode: 2n,
				streamCount: 1n,
				errorReason: "end",
			},
		},
		{
			name: "SUBSCRIBE_OK with EXPIRES, LARGEST_OBJECT and a Track Property (§10.8)",
			bytes: "04 00 0a 07 02 08 0a 01 03 05 02 80 80",
			message: {
				type: "SUBSCRIBE_OK",
				trackAlias: 7n,
				parameters: { expires: 10n, largestObject: { group: 3n, object: 5n } },
				trackProperties: [{ type: 2n, value: 128n }],
			},
		},
		{
			name: "SUBSCRIBE with timeouts, an AbsoluteRange filter and NEW_GROUP_REQUEST (§10.2)",
			bytes:
				"03 00 13 04 00 01 74 05 02 05 04 06 1b 04 04 02 03 01 01 02 10 00",
			message: {
				type: "SUBSCRIBE",
				requestId: 4n,
				trackNamespace: [],
				trackName: utf8("t"),
				parameters: {
					objectDeliveryTimeout: 5n,
					subgroupDeliveryTimeout: 6n,
					subscriptionFilter: {
						filterType: "AbsoluteRange",
						startLocation: { group: 2n, object: 3n },
						endGroupDelta: 1n,
					},
					groupOrder: "Descending",
					newGroupRequest: 0n,
				},
			},
		},
		{
			name: "SUBSCRIBE with two AUTHORIZATION_TOKENs and RENDEZVOUS_TIMEOUT (§10.2.2, §10.2.6)",
			bytes: "03 00 0e 00 00 01 74 03 03 02 aa bb 00 01 cc 01 05",
			message: {
				type: "SUBSCRIBE",
				requestId: 0n,
				trackNamespace: [],
				trackName: utf8("t"),
				parameters: {
					authorizationToken: [hex("aa bb"), hex("cc")],
					rendezvousTimeout: 5n,
				},
			},
		},
		{
			name: "REQUEST_UPDATE with FORWARD (§10.9)",
			bytes: "02 00 04 01 01 10 01",
			message: {
				type: "REQUEST_UPDATE",
				requestId: 1n,
				parameters: { forward: true },
			},
		},
		// The FETCH messages below are the same bytes as
		// `the_fetches_sent_to_take_the_fixtures_say_what_they_were_meant_to`
		// in crates/moqt/src/message.rs: written by hand there, and sent to
		// the moqtail relay to take its answers.
		{
			name: "FETCH, Relative Joining, Joining Start 1 (§10.12.2)",
			bytes: "16 00 07 02 02 00 01 01 20 c8",
			message: {
				type: "FETCH",
				requestId: 2n,
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 0n,
					joiningStart: 1n,
				},
				parameters: { subscriberPriority: 200 },
			},
		},
		{
			name: "FETCH, Absolute Joining, Joining Start 2 (§10.12.2)",
			bytes: "16 00 07 02 03 00 02 01 20 c8",
			message: {
				type: "FETCH",
				requestId: 2n,
				range: {
					fetchType: "AbsoluteJoining",
					joiningRequestId: 0n,
					joiningStart: 2n,
				},
				parameters: { subscriberPriority: 200 },
			},
		},
		{
			name: "FETCH, Relative Joining, without parameters",
			bytes: "16 00 05 00 02 08 00 00",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 8n,
					joiningStart: 0n,
				},
				parameters: {},
			},
		},
		{
			name: "FETCH, Standalone, with GROUP_ORDER Descending (§10.2.8)",
			bytes:
				"16 00 1d 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 00 00 02 00 01 22 02",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "Standalone",
					trackNamespace: [utf8("xmoq"), utf8("phase0")],
					trackName: utf8("object"),
					startLocation: { group: 0n, object: 0n },
					endLocation: { group: 2n, object: 0n },
				},
				parameters: { groupOrder: "Descending" },
			},
		},
		{
			name: "FETCH, Standalone, Groups 0 to 3",
			bytes:
				"16 00 1b 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 00 00 04 00 00",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "Standalone",
					trackNamespace: [utf8("xmoq"), utf8("phase0")],
					trackName: utf8("object"),
					startLocation: { group: 0n, object: 0n },
					endLocation: { group: 4n, object: 0n },
				},
				parameters: {},
			},
		},
		{
			name: "FETCH, Standalone, Group 1 alone",
			bytes:
				"16 00 1b 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 01 00 01 00 00",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "Standalone",
					trackNamespace: [utf8("xmoq"), utf8("phase0")],
					trackName: utf8("object"),
					startLocation: { group: 1n, object: 0n },
					endLocation: { group: 1n, object: 0n },
				},
				parameters: {},
			},
		},
		{
			name: "FETCH, Standalone, Group 100 (0x64 is one byte by §1.4.1)",
			bytes:
				"16 00 1b 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 64 00 65 00 00",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "Standalone",
					trackNamespace: [utf8("xmoq"), utf8("phase0")],
					trackName: utf8("object"),
					startLocation: { group: 100n, object: 0n },
					endLocation: { group: 101n, object: 0n },
				},
				parameters: {},
			},
		},
		// The next two are the bytes the Rust tests have for the same.
		{
			name: "FETCH with FILL_TIMEOUT and GROUP_ORDER Descending, as in the Rust tests",
			bytes: "16 00 09 04 02 00 00 02 0a 05 18 02",
			message: {
				type: "FETCH",
				requestId: 4n,
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 0n,
					joiningStart: 0n,
				},
				parameters: { fillTimeout: 5n, groupOrder: "Descending" },
			},
		},
		{
			name: "SUBSCRIBE_OK with DYNAMIC_GROUPS 1, the largest value (§12.6)",
			bytes: "04 00 04 00 00 30 01",
			message: {
				type: "SUBSCRIBE_OK",
				trackAlias: 0n,
				parameters: {},
				trackProperties: [{ type: 0x30n, value: 1n }],
			},
		},
		{
			name: "FETCH_OK with End Of Track and a Track Property, as in the Rust tests",
			bytes: "18 00 06 01 02 03 00 0e 05",
			message: {
				type: "FETCH_OK",
				endOfTrack: true,
				endLocation: { group: 2n, object: 3n },
				parameters: {},
				trackProperties: [{ type: 0x0en, value: 5n }],
			},
		},
		{
			name: "FETCH with FILL_TIMEOUT and an AUTHORIZATION_TOKEN (§10.2.5, §10.2.2)",
			bytes: "16 00 0a 00 02 00 00 02 03 01 aa 07 05",
			message: {
				type: "FETCH",
				requestId: 0n,
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 0n,
					joiningStart: 0n,
				},
				parameters: { authorizationToken: [hex("aa")], fillTimeout: 5n },
			},
		},
		{
			name: "FETCH_OK with End Of Track and a Track Property (§10.13)",
			bytes: "18 00 06 01 03 00 00 02 05",
			message: {
				type: "FETCH_OK",
				endOfTrack: true,
				endLocation: { group: 3n, object: 0n },
				parameters: {},
				trackProperties: [{ type: 2n, value: 5n }],
			},
		},
	];

describe("control messages encoded by hand from the draft", () => {
	test.each(HAND_ENCODED)("decodes $name", ({ bytes, message }) => {
		const input = hex(bytes);
		expect(decodeControlMessage(input)).toEqual({
			status: "ok",
			value: message,
			bytesRead: input.length,
		});
	});

	test.each(HAND_ENCODED)("encodes $name", ({ bytes, message }) => {
		expect(toHex(encodeControlMessage(message))).toBe(bytes);
	});
});

describe("decodeControlMessage", () => {
	test.each([
		{ name: "empty input", bytes: "" },
		{ name: "Message Type only", bytes: "04" },
		{ name: "half of Message Length", bytes: "04 00" },
		{ name: "payload one byte short", bytes: "04 00 02 00" },
	])("waits for more bytes: $name", ({ bytes }) => {
		expect(decodeControlMessage(hex(bytes))).toEqual({ status: "incomplete" });
	});

	test("reads only the first message and reports its length", () => {
		expect(decodeControlMessage(hex("07 00 01 00 04 00 02 00 00"))).toEqual({
			status: "ok",
			value: { type: "REQUEST_OK", parameters: {}, trackProperties: [] },
			bytesRead: 4,
		});
	});

	// `code` is the Session termination error code of §3.5.
	test.each([
		{ name: "unknown Message Type 0x3f", bytes: "3f 00 00", code: 3 },
		{ name: "reserved Message Type 0x01", bytes: "01 00 00", code: 3 },
		{ name: "reserved Message Type 0x20", bytes: "20 00 00", code: 3 },
		{ name: "reserved Message Type 0x21", bytes: "21 00 00", code: 3 },
		{ name: "reserved Message Type 0x40", bytes: "40 00 00", code: 3 },
		{ name: "reserved Message Type 0x41", bytes: "41 00 00", code: 3 },
		{
			name: "TRACK_NAMESPACE_PREFIX in the REQUEST_UPDATE of a subscription (§10.2.1)",
			bytes: "02 00 03 01 01 34",
			code: 3,
		},
		{
			name: "EXPIRES in REQUEST_UPDATE (§10.2.1)",
			bytes: "02 00 04 01 01 08 00",
			code: 3,
		},
		{
			name: "0x1E PUBLISH_OK, in Table 5 without a format",
			bytes: "1e 00 01 00",
			code: 1,
		},
		{ name: "GOAWAY without its Timeout", bytes: "10 00 01 00", code: 3 },
		{
			name: "REQUEST_OK whose Track Properties end inside a pair",
			bytes: "07 00 02 00 01",
			code: 3,
		},
		{
			name: "PUBLISH_DONE with a byte of its Message Length left over",
			bytes: "0b 00 04 02 01 00 00",
			code: 3,
		},
		{
			name: "Message Length shorter than the payload",
			bytes: "04 00 01 00",
			code: 3,
		},
		{
			name: "unknown Message Parameter 0x01",
			bytes: "07 00 03 01 01 00",
			code: 3,
		},
		{ name: "duplicate FORWARD", bytes: "07 00 05 02 10 01 00 01", code: 3 },
		{ name: "FORWARD value 2", bytes: "07 00 03 01 10 02", code: 3 },
		{ name: "GROUP_ORDER value 3", bytes: "07 00 03 01 22 03", code: 3 },
		{
			name: "AUTHORIZATION_TOKEN in REQUEST_OK (§10.2.1)",
			bytes: "07 00 04 01 03 01 00",
			code: 3,
		},
		{
			name: "RENDEZVOUS_TIMEOUT in PUBLISH (§10.2.1)",
			bytes: "1d 00 07 00 00 00 00 01 04 05",
			code: 3,
		},
		{
			name: "FILL_TIMEOUT in SUBSCRIBE (§10.2.1)",
			bytes: "03 00 06 00 00 00 01 0a 00",
			code: 3,
		},
		// The FETCH and FETCH_OK cases are the same bytes as in
		// `rejects_malformed_input` of crates/moqt/src/message.rs.
		{
			name: "FETCH with Fetch Type 4 (§10.12)",
			bytes: "16 00 03 00 04 00",
			code: 3,
		},
		{
			name: "FETCH with Fetch Type 0 (§10.12)",
			bytes: "16 00 03 00 00 00",
			code: 3,
		},
		{
			name: "FORWARD, a SUBSCRIBE parameter, in FETCH (§10.2.1)",
			bytes: "16 00 07 02 02 00 00 01 10 01",
			code: 3,
		},
		{
			name: "GROUP_ORDER value 3 in FETCH (§10.2.8)",
			bytes: "16 00 07 02 02 00 00 01 22 03",
			code: 3,
		},
		{
			name: "a Standalone FETCH that stops after its Start Location",
			bytes: "16 00 07 00 01 00 01 61 00 01",
			code: 3,
		},
		{
			name: "unknown Message Parameter 0x01 in FETCH (§10.2)",
			bytes: "16 00 07 00 02 00 00 01 01 00",
			code: 3,
		},
		{
			name: "FETCH_OK with End Of Track 2 (§10.13)",
			bytes: "18 00 04 02 01 02 00",
			code: 3,
		},
		{
			name: "DYNAMIC_GROUPS 2 in SUBSCRIBE_OK (§12.6)",
			bytes: "04 00 04 00 00 30 02",
			code: 3,
		},
		{
			name: "DYNAMIC_GROUPS 2 in a PUBLISH of a/t (§12.6)",
			bytes: "1d 00 0a 00 01 01 61 01 74 00 00 30 02",
			code: 3,
		},
		{
			name: "DYNAMIC_GROUPS 2 in REQUEST_OK (§12.6)",
			bytes: "07 00 03 00 30 02",
			code: 3,
		},
		{
			name: "DYNAMIC_GROUPS 2 in FETCH_OK (§12.6)",
			bytes: "18 00 06 00 01 02 00 30 02",
			code: 3,
		},
		{
			name: "DEFAULT PUBLISHER PRIORITY 256 in SUBSCRIBE_OK (§12.4)",
			bytes: "04 00 05 00 00 0e 81 00",
			code: 3,
		},
		{
			name: "DEFAULT PUBLISHER PRIORITY 256 in a PUBLISH of a/t (§12.4)",
			bytes: "1d 00 0b 00 01 01 61 01 74 00 00 0e 81 00",
			code: 3,
		},
		{
			name: "DEFAULT PUBLISHER PRIORITY 256 in REQUEST_OK (§12.4)",
			bytes: "07 00 04 00 0e 81 00",
			code: 3,
		},
		{
			name: "DEFAULT PUBLISHER PRIORITY 256 in FETCH_OK (§12.4)",
			bytes: "18 00 07 00 01 02 00 0e 81 00",
			code: 3,
		},
		{
			name: "EXPIRES in FETCH_OK: no parameter lists FETCH_OK (§10.2.1)",
			bytes: "18 00 06 00 01 02 01 08 05",
			code: 3,
		},
		{
			name: "unknown Filter Type 5",
			bytes: "03 00 07 00 00 00 01 21 01 05",
			code: 3,
		},
		{
			name: "SUBSCRIPTION_FILTER longer than LargestObject needs",
			bytes: "03 00 08 00 00 00 01 21 02 02 00",
			code: 3,
		},
		{
			name: "EXPIRES in SUBSCRIBE (§10.2.1)",
			bytes: "03 00 06 00 00 00 01 08 00",
			code: 3,
		},
		{
			name: "FORWARD in SUBSCRIBE_OK (§10.2.1)",
			bytes: "04 00 04 00 01 10 01",
			code: 3,
		},
		{
			name: "FORWARD in PUBLISH_NAMESPACE (§10.2.1)",
			bytes: "06 00 05 00 00 01 10 01",
			code: 3,
		},
		{
			name: "Track Namespace Field of length 0",
			bytes: "06 00 04 00 01 00 00",
			code: 3,
		},
		{ name: "33 Track Namespace Fields", bytes: "06 00 03 00 21 00", code: 3 },
		{
			name: "Reason Phrase Length 1025",
			bytes: "05 00 04 00 00 84 01",
			code: 3,
		},
		{
			name: "Key-Value-Pair Length 65536",
			bytes: "af 00 00 05 01 c1 00 00 00",
			code: 3,
		},
		{ name: "Setup Option cut short", bytes: "af 00 00 02 07 05", code: 3 },
		// TRACK_STATUS 0xD, Request ID 2, namespace (a), name b: only a
		// SUBSCRIBE may carry these (§10.2.1, §10.2.2, §10.14).
		{
			name: "RENDEZVOUS_TIMEOUT in TRACK_STATUS (§10.2.1)",
			bytes: "0d 00 09 02 01 01 61 01 62 01 04 01",
			code: 3,
		},
		{
			name: "SUBSCRIBER_PRIORITY in TRACK_STATUS (§10.14)",
			bytes: "0d 00 09 02 01 01 61 01 62 01 20 80",
			code: 3,
		},
	])("rejects $name", ({ bytes, code }) => {
		const result = decodeControlMessage(hex(bytes));
		expect(result.status).toBe("error");
		if (result.status === "error") expect(result.error.code).toBe(code);
	});
});

// §10.14: the format of SUBSCRIBE; of the parameters of §10.2,
// AUTHORIZATION_TOKEN alone may appear in it (§10.2.2). Written by hand from
// the draft, as the Rust test `a_track_status_takes_an_authorization_token_and_nothing_else`;
// the bytes of moqtail are in moqtail-fixtures.test.ts.
describe("TRACK_STATUS", () => {
	test.each([
		{
			name: "no parameter",
			bytes: "0d 00 07 02 01 01 61 01 62 00",
			parameters: {},
		},
		{
			// AUTHORIZATION_TOKEN (0x3): USE_VALUE (0x3), Token Type 0, "t".
			name: "an AUTHORIZATION_TOKEN",
			bytes: "0d 00 0c 02 01 01 61 01 62 01 03 03 03 00 74",
			parameters: { authorizationToken: [hex("03 00 74")] },
		},
	] satisfies { name: string; bytes: string; parameters: MessageParameters }[])(
		"with $name is decoded, and encoded again to the same bytes",
		({ bytes, parameters }) => {
			const message: ControlMessage = {
				type: "TRACK_STATUS",
				requestId: 2n,
				trackNamespace: [utf8("a")],
				trackName: utf8("b"),
				parameters,
			};
			expect(decodeControlMessage(hex(bytes))).toEqual({
				status: "ok",
				value: message,
				bytesRead: hex(bytes).length,
			});
			expect(toHex(encodeControlMessage(message))).toBe(bytes);
		},
	);
});

describe("local limits of decodeControlMessage", () => {
	// `pairs` Setup Options of Type 2, 4, 6, ... with value 0: "02 00" each.
	test.each([
		{ pairs: 64, status: "ok" },
		{ pairs: 65, status: "error" },
	])("a SETUP with $pairs Setup Options: $status", ({ pairs, status }) => {
		const payload = "02 00 ".repeat(pairs);
		const length = (pairs * 2).toString(16).padStart(4, "0");
		const result = decodeControlMessage(
			hex(`af 00 ${length.slice(0, 2)} ${length.slice(2)} ${payload}`),
		);
		expect(result.status).toBe(status);
		// The peer broke no rule: INTERNAL_ERROR, not PROTOCOL_VIOLATION.
		if (result.status === "error") expect(result.error.code).toBe(1);
	});
});

describe("the limit on Message Parameters", () => {
	// A SUBSCRIBE with `tokens` AUTHORIZATION_TOKEN parameters, each "aa":
	// "03 01 aa" for the first, "00 01 aa" for the rest.
	test.each([
		{ tokens: 64, status: "ok" },
		{ tokens: 65, status: "error" },
	])(
		"a SUBSCRIBE with $tokens AUTHORIZATION_TOKENs: $status",
		({ tokens, status }) => {
			const payload = `00 00 00 ${tokens.toString(16)} 03 01 aa ${"00 01 aa ".repeat(tokens - 1)}`;
			const length = (4 + tokens * 3).toString(16).padStart(4, "0");
			const result = decodeControlMessage(
				hex(`03 ${length.slice(0, 2)} ${length.slice(2)} ${payload}`),
			);
			expect(result.status).toBe(status);
			if (result.status === "error") expect(result.error.code).toBe(1);
		},
	);
});

describe("a Number of Parameters the message is too short for", () => {
	// A SUBSCRIBE that ends after its Number of Parameters. `code` is the
	// Session termination error code.
	test.each([
		{ name: "64, within the limit", count: "40", code: 3 },
		{ name: "65, over the limit as well", count: "41", code: 1 },
	])("$name: closes with code $code", ({ count, code }) => {
		const result = decodeControlMessage(hex(`03 00 04 00 00 00 ${count}`));
		expect(result).toMatchObject({ status: "error", error: { code } });
	});
});

describe("decodeRequestStart", () => {
	test.each([
		{
			name: "SUBSCRIBE with one byte of its payload",
			bytes: "03 00 1f 01",
			messageType: 3n,
			requestId: 1n,
		},
		{
			name: "FETCH, whose start is all that is read here",
			bytes: "16 00 02 05 00",
			messageType: 22n,
			requestId: 5n,
		},
	])(
		"reads Message Type and Request ID: $name",
		({ bytes, messageType, requestId }) => {
			expect(decodeRequestStart(hex(bytes))).toMatchObject({
				status: "ok",
				value: { messageType, requestId },
			});
		},
	);

	test.each([
		{ name: "Message Type only", bytes: "03" },
		{ name: "no payload byte yet", bytes: "03 00 1f" },
		{ name: "half of a 2-byte Request ID", bytes: "03 00 1f 80" },
	])("waits for more bytes: $name", ({ bytes }) => {
		expect(decodeRequestStart(hex(bytes))).toEqual({ status: "incomplete" });
	});

	// §3.3: a request stream begins with one of seven message types.
	test.each([
		{ name: "SUBSCRIBE_OK", bytes: "04 00 02 00 00" },
		{ name: "an unknown Message Type", bytes: "3f 00 00" },
		{ name: "a SUBSCRIBE with an empty payload", bytes: "03 00 00" },
		{
			name: "a Message Length that ends inside the Request ID",
			bytes: "03 00 01 80",
		},
	])("reports a PROTOCOL_VIOLATION for $name", ({ bytes }) => {
		const result = decodeRequestStart(hex(bytes));
		expect(result.status).toBe("error");
		if (result.status === "error") expect(result.error.code).toBe(3);
	});
});

describe("encodeControlMessage", () => {
	test.each([
		{
			name: "a parameter outside its scope",
			message: {
				type: "SUBSCRIBE_OK",
				trackAlias: 0n,
				parameters: { forward: true },
				trackProperties: [],
			},
		},
		{
			name: "an empty Track Namespace Field",
			message: {
				type: "PUBLISH_NAMESPACE",
				requestId: 0n,
				trackNamespace: [new Uint8Array(0)],
				parameters: {},
			},
		},
		{
			name: "a Redirect without the REDIRECT code",
			message: {
				type: "REQUEST_ERROR",
				errorCode: 0n,
				retryInterval: 0n,
				errorReason: "",
				redirect: {
					connectUri: "",
					trackNamespace: [],
					trackName: new Uint8Array(0),
				},
			},
		},
	] satisfies { name: string; message: ControlMessage }[])(
		"refuses $name",
		({ message }) => {
			expect(() => encodeControlMessage(message)).toThrow();
		},
	);
});
