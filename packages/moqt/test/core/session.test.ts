import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import type { SessionLimits } from "../../src/core/limits.ts";
import type { SubgroupOptions } from "../../src/core/outgoing-subgroups.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fromALibrary, hex, MOQTAIL_RELAY_SETUP, utf8 } from "../helpers.ts";

const NAMESPACE = [utf8("xmoq"), utf8("phase0")];
const TRACK_NAME = utf8("object");
const TRACK = { trackNamespace: NAMESPACE, trackName: TRACK_NAME };

/** Small enough to reach in a test. A held Object without payload counts 64 bytes. */
const SMALL_LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16,
	maxObjectPropertiesLength: 16,
	maxPeerRequests: 64,
	maxPeerFetches: 16,
	maxHeldSubgroupBytes: 64,
};

function started(limits?: SessionLimits): Session {
	const session = createSession({ limits });
	session.start();
	return session;
}

/**
 * The outputs without the ones that say that a stream of the peer has said
 * what it is (`peer-stream-identified`), so that a test lists what a stream
 * leads to; `session-first-bytes.test.ts` has when that output comes.
 */
function said(outputs: SessionOutput[]): SessionOutput[] {
	return outputs.filter((output) => output.type !== "peer-stream-identified");
}

/** A session with SUBSCRIBE on stream 1 (Request ID 0) and PUBLISH on stream 2 (Request ID 2, Track Alias 1). */
function withRequests(): Session {
	const session = started();
	session.subscribe(TRACK);
	session.publish({ ...TRACK, trackAlias: 1n });
	return session;
}

/** A session whose SUBSCRIBE on stream 1 (Request ID 0) is Established with Track Alias 9. */
function subscribed(): Session {
	const session = started();
	session.subscribe(TRACK);
	session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
	return session;
}

const SUBGROUP: SubgroupOptions = {
	groupId: 0n,
	subgroupId: { mode: "Zero" },
	properties: false,
	endOfGroup: false,
};

const OBJECT_ON_ALIAS_9: SessionOutput = {
	type: "event",
	event: {
		type: "object",
		requestId: 0n,
		trackAlias: 9n,
		groupId: 0n,
		subgroupId: 0n,
		publisherPriority: 0,
		object: {
			objectId: 0n,
			properties: [],
			status: "Normal",
			payload: utf8("a"),
		},
	},
};

const CLOSED_AFTER_PUBLISH_DONE: SessionOutput[] = [
	{
		type: "event",
		event: { type: "request-closed", requestId: 0n, reason: "publish-done" },
	},
	{ type: "finish", stream: 1 },
];

describe("Session", () => {
	test.each([
		{
			name: "no Setup Options",
			implementation: undefined,
			bytes: "af 00 00 00",
		},
		{
			name: "MOQT_IMPLEMENTATION",
			implementation: "xmoq/0",
			bytes: "af 00 00 08 07 06 78 6d 6f 71 2f 30",
		},
	])(
		"start() opens the control stream with SETUP: $name",
		({ implementation, bytes }) => {
			expect(createSession({ implementation }).start()).toEqual([
				{ type: "open-uni-stream", stream: 0, data: hex(bytes) },
			]);
		},
	);

	// §10.4: "Upon receiving a GOAWAY on the control stream, an endpoint
	// SHOULD NOT initiate new requests to the peer", and "The GOAWAY message
	// does not impact subscription state".
	test("a GOAWAY is reported, opens no request after it, and leaves what is open", () => {
		const session = subscribed();
		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("af 00 00 00 10 00 03 00 00 02"),
					false,
				),
			),
		).toEqual([
			{ type: "event", event: { type: "setup", setupOptions: [] } },
			{
				type: "event",
				event: { type: "go-away", newSessionUri: "", timeout: 0n },
			},
		]);
		expect(() => session.subscribe({ ...TRACK, trackName: utf8("b") })).toThrow(
			"the peer sent GOAWAY",
		);
		expect(() =>
			session.publishNamespace({ trackNamespace: NAMESPACE }),
		).toThrow("the peer sent GOAWAY");
		expect(
			said(
				session.receivePeerUniStreamData(
					4,
					hex("14 09 00 00 00 00 01 61"),
					true,
				),
			),
		).toEqual([OBJECT_ON_ALIAS_9]);
	});

	// §10.4: a GOAWAY on a request stream asks for that request to be sent
	// again; the session tells it and leaves the request, and the session, as
	// they were.
	test("a GOAWAY on the stream of a SUBSCRIBE is told and changes nothing", () => {
		const session = subscribed();
		expect(
			session.receiveRequestStreamData(
				1,
				hex(fromALibrary("goaway-request-stream-moqtail-ts")),
				false,
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "request-go-away",
					requestId: 0n,
					newSessionUri: "https://relay.example:4443/",
					timeout: 2000n,
				},
			},
		]);
		expect(session.subscribe({ ...TRACK, trackName: utf8("b") }).value).toBe(
			2n,
		);
		expect(
			said(
				session.receivePeerUniStreamData(
					4,
					hex("14 09 00 00 00 00 01 61"),
					true,
				),
			),
		).toEqual([OBJECT_ON_ALIAS_9]);
	});

	test("subscribes with the bytes the moqtail crate sends and reads the relay's answers", () => {
		const session = started();

		expect(
			session.subscribe({
				...TRACK,
				parameters: {
					forward: true,
					subscriberPriority: 128,
					subscriptionFilter: { filterType: "LargestObject" },
					groupOrder: "Ascending",
				},
			}),
		).toEqual({
			value: 0n,
			outputs: [
				{
					type: "open-bidi-stream",
					stream: 1,
					data: hex(
						"03 00 1f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 04 10 01 10 80 01 01 02 01 01",
					),
				},
			],
		});

		expect(
			said(
				session.receivePeerUniStreamData(0, hex(MOQTAIL_RELAY_SETUP), false),
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "setup",
					setupOptions: [{ type: 7n, value: utf8("moqtail/0.15.0+dd8623d") }],
				},
			},
		]);
		expect(
			session.receiveRequestStreamData(1, hex("04 00 02 00 00"), false),
		).toEqual([
			{
				type: "event",
				event: {
					type: "subscribe-ok",
					requestId: 0n,
					trackAlias: 0n,
					parameters: {},
					trackProperties: [],
				},
			},
		]);
		expect(
			said(
				session.receivePeerUniStreamData(
					1,
					hex("14 00 03 00 80 00 01 61"),
					true,
				),
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "object",
					requestId: 0n,
					trackAlias: 0n,
					groupId: 3n,
					subgroupId: 0n,
					publisherPriority: 128,
					object: {
						objectId: 0n,
						properties: [],
						status: "Normal",
						payload: utf8("a"),
					},
				},
			},
		]);
	});

	test("publishes with the bytes moqtail-ts sends and writes one Object", () => {
		const session = started();

		expect(session.publishNamespace({ trackNamespace: NAMESPACE })).toEqual({
			value: 0n,
			outputs: [
				{
					type: "open-bidi-stream",
					stream: 1,
					data: hex("06 00 0f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 00"),
				},
			],
		});
		expect(
			session.receiveRequestStreamData(1, hex("07 00 01 00"), false),
		).toEqual([
			{ type: "event", event: { type: "publish-namespace-ok", requestId: 0n } },
		]);

		expect(
			session.publish({
				...TRACK,
				trackAlias: 779698128977928661n,
				parameters: { forward: true },
			}),
		).toEqual({
			value: 2n,
			outputs: [
				{
					type: "open-bidi-stream",
					stream: 2,
					data: hex(
						"1d 00 21 02 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 ff 0a d2 0b 51 39 c4 21 d5 01 10 01",
					),
				},
			],
		});
		expect(
			session.receiveRequestStreamData(
				2,
				hex("07 00 05 02 10 01 12 01"),
				false,
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "publish-ok",
					requestId: 2n,
					parameters: { forward: true, groupOrder: "Ascending" },
				},
			},
		]);

		expect(
			session.openSubgroup(2n, {
				groupId: 0n,
				subgroupId: { mode: "Explicit", value: 0n },
				publisherPriority: 0,
				properties: false,
				endOfGroup: true,
			}),
		).toEqual({
			value: 3,
			outputs: [
				{
					type: "open-uni-stream",
					stream: 3,
					data: hex("5c ff 0a d2 0b 51 39 c4 21 d5 00 00 00"),
				},
			],
		});
		expect(
			session.writeObject(3, { objectId: 0n, payload: utf8("a") }),
		).toEqual([{ type: "send", stream: 3, data: hex("00 01 61") }]);
		expect(session.closeSubgroup(3)).toEqual([{ type: "finish", stream: 3 }]);
		expect(session.publishDone(2n, 2n, "")).toEqual([
			{ type: "send", stream: 2, data: hex("0b 00 03 02 01 00") },
			{ type: "finish", stream: 2 },
		]);
	});

	test("holds Objects that arrive before SUBSCRIBE_OK and delivers them after it", () => {
		const session = started();
		session.subscribe(TRACK);

		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("14 09 00 00 00 00 01 61"),
					true,
				),
			),
		).toEqual([]);
		expect(
			session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false),
		).toEqual([
			{
				type: "event",
				event: {
					type: "subscribe-ok",
					requestId: 0n,
					trackAlias: 9n,
					parameters: {},
					trackProperties: [],
				},
			},
			OBJECT_ON_ALIAS_9,
		]);
	});

	test("bounds held Objects by count too: the second Object without payload exceeds 64 bytes", () => {
		const session = started(SMALL_LIMITS);
		session.subscribe(TRACK);

		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("14 09 00 00 00 00 00 00"),
					false,
				),
			),
		).toEqual([]);
		expect(
			said(session.receivePeerUniStreamData(0, hex("00 00 00"), false)),
		).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 0 },
			{
				type: "event",
				event: {
					type: "subgroup-abandoned",
					trackAlias: 9n,
					reason: "too many Objects before SUBSCRIBE_OK",
				},
			},
		]);
	});

	test("stops a subgroup stream whose Track Alias no subscription can establish", () => {
		const session = started();
		expect(
			said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false)),
		).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 1 },
			{
				type: "event",
				event: {
					type: "subgroup-abandoned",
					trackAlias: 9n,
					reason: "unknown Track Alias",
				},
			},
		]);
	});

	test("a cancelled SUBSCRIBE stops its open subgroup streams, resets its request stream and drops later Objects (§10.11, §11.1)", () => {
		const session = subscribed();
		said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false));
		// A second SUBSCRIBE is Pending, so an unknown alias would be held.
		session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });

		expect(session.cancelRequest(0n)).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 1 },
			{ type: "reset", stream: 1, code: 1 },
		]);
		expect(
			said(session.receivePeerUniStreamData(0, hex("00 01 61"), false)),
		).toEqual([]);
		expect(
			said(session.receivePeerUniStreamData(1, hex("14 09 01 00 00"), false)),
		).toEqual([
			{ type: "stop-sending", peerStream: 1, code: 1 },
			{
				type: "event",
				event: {
					type: "subgroup-abandoned",
					trackAlias: 9n,
					reason: "the subscription ended",
				},
			},
		]);
	});

	test("counts a subgroup stream that ended without an Object before SUBSCRIBE_OK (§10.11)", () => {
		const session = started();
		session.subscribe(TRACK);
		said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), true));
		session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
		// PUBLISH_DONE with Stream Count 1: the one stream has already ended.
		expect(
			session
				.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false)
				.slice(1),
		).toEqual(CLOSED_AFTER_PUBLISH_DONE);
	});

	test("a Track Alias that received PUBLISH_DONE can go to a new subscription (§11.1, §5.1.1)", () => {
		const session = subscribed();
		session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
		session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });

		expect(
			session.receiveRequestStreamData(2, hex("04 00 02 09 00"), false),
		).toEqual([
			...CLOSED_AFTER_PUBLISH_DONE,
			{
				type: "event",
				event: {
					type: "subscribe-ok",
					requestId: 2n,
					trackAlias: 9n,
					parameters: {},
					trackProperties: [],
				},
			},
		]);
	});

	// §5.1: PUBLISH_DONE may be sent while the PUBLISH is Pending, and the peer
	// can have answered before it saw that.
	test.each([
		{ name: "PUBLISH_OK", bytes: "07 00 01 00" },
		{ name: "REQUEST_ERROR", bytes: "05 00 05 10 00 02 6e 6f" },
	])(
		"ignores a $name that crosses this endpoint's PUBLISH_DONE",
		({ bytes }) => {
			const session = withRequests();
			expect(session.publishDone(2n, 2n, "")).toEqual([
				{ type: "send", stream: 2, data: hex("0b 00 03 02 00 00") },
				{ type: "finish", stream: 2 },
			]);
			expect(session.receiveRequestStreamData(2, hex(bytes), false)).toEqual(
				[],
			);
			expect(session.closed).toBe(false);
		},
	);

	test("refuses a REQUEST_UPDATE for its PUBLISH and ends the subscription with UPDATE_FAILED (§10.9.1)", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.openSubgroup(2n, SUBGROUP);

		// REQUEST_UPDATE with Request ID 1 and FORWARD = 0.
		const outputs = session.receiveRequestStreamData(
			2,
			hex("02 00 04 01 01 10 00"),
			false,
		);
		expect(outputs.map((output) => output.type)).toEqual([
			"send",
			"reset",
			"send",
			"event",
			"finish",
		]);
		const [refusal, reset, done, closed] = outputs;
		if (refusal?.type !== "send" || done?.type !== "send") {
			throw new Error("the outputs are not the two messages expected");
		}
		expect(decodeControlMessage(refusal.data)).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode: 3n },
		});
		expect(reset).toEqual({ type: "reset", stream: 3, code: 1 });
		expect(decodeControlMessage(done.data)).toMatchObject({
			status: "ok",
			value: { type: "PUBLISH_DONE", statusCode: 8n, streamCount: 1n },
		});
		expect(closed).toEqual({
			type: "event",
			event: { type: "request-closed", requestId: 2n, reason: "update-failed" },
		});
		expect(session.closed).toBe(false);
	});

	// §10.2.13. The bytes are written by hand: Request ID 1, one parameter,
	// NEW_GROUP_REQUEST (0x32) with the value in the last byte.
	test.each([
		{ name: "0, no Group known", bytes: "02 00 04 01 01 32 00", group: 0n },
		{
			name: "the largest Group plus 1",
			bytes: "02 00 04 01 01 32 07",
			group: 7n,
		},
	])(
		"accepts a REQUEST_UPDATE with NEW_GROUP_REQUEST for its PUBLISH and reports it: $name",
		({ bytes, group }) => {
			const session = withRequests();
			session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
			const outputs = session.receiveRequestStreamData(2, hex(bytes), false);
			expect(outputs).toEqual([
				{ type: "send", stream: 2, data: hex("07 00 01 00") },
				{
					type: "event",
					event: { type: "new-group-request", requestId: 2n, group },
				},
			]);
			expect(session.closed).toBe(false);
		},
	);

	test("REQUEST_ERROR ends the request and finishes its stream", () => {
		const session = started();
		session.subscribe(TRACK);
		expect(
			session.receiveRequestStreamData(1, hex("05 00 05 10 00 02 6e 6f"), true),
		).toEqual([
			{
				type: "event",
				event: {
					type: "request-error",
					requestId: 0n,
					errorCode: 16n,
					retryInterval: 0n,
					errorReason: "no",
					redirect: undefined,
				},
			},
			{ type: "finish", stream: 1 },
		]);
	});

	test("after PUBLISH_DONE and FIN, a late stream still delivers; the subscription ends at Stream Count (§10.11)", () => {
		const session = subscribed();
		expect(
			session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), true),
		).toEqual([
			{
				type: "event",
				event: {
					type: "publish-done",
					requestId: 0n,
					statusCode: 2n,
					streamCount: 1n,
					errorReason: "",
				},
			},
		]);
		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("14 09 00 00 00 00 01 61"),
					true,
				),
			),
		).toEqual([OBJECT_ON_ALIAS_9, ...CLOSED_AFTER_PUBLISH_DONE]);
	});

	test("PUBLISH_DONE with Stream Count 0 ends the subscription at once", () => {
		const session = subscribed();
		expect(
			session
				.receiveRequestStreamData(1, hex("0b 00 03 02 00 00"), false)
				.slice(1),
		).toEqual([...CLOSED_AFTER_PUBLISH_DONE]);
	});

	test("expireSubscription ends the wait after PUBLISH_DONE; later Objects are dropped", () => {
		const session = subscribed();
		session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);

		expect(session.expireSubscription(0n)).toEqual([
			...CLOSED_AFTER_PUBLISH_DONE,
		]);
		expect(
			said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false)),
		).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 1 },
			{
				type: "event",
				event: {
					type: "subgroup-abandoned",
					trackAlias: 9n,
					reason: "the subscription ended",
				},
			},
		]);
	});

	test("expireSubscription does nothing before PUBLISH_DONE", () => {
		expect(subscribed().expireSubscription(0n)).toEqual([]);
	});

	test("cancels a subscription whose SUBSCRIBE_OK has an unknown Mandatory Track Property (§2.5.1)", () => {
		const session = started();
		session.subscribe(TRACK);
		// Track Property 0x4000 (even, so a varint value) follows the parameters.
		expect(
			session.receiveRequestStreamData(
				1,
				hex("04 00 06 09 00 c0 40 00 01"),
				false,
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "request-closed",
					requestId: 0n,
					reason: "unsupported-track-property",
				},
			},
			{ type: "reset", stream: 1, code: 1 },
		]);
	});

	test("cancels a subscription whose Object carries a Mandatory Track Property, with MALFORMED_TRACK (§2.4.2, §3.3.3)", () => {
		const session = subscribed();
		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("15 09 00 00 00 00 04 c0 40 00 01 01 61"),
					false,
				),
			),
		).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 18 },
			{
				type: "event",
				event: {
					type: "subgroup-abandoned",
					trackAlias: 9n,
					reason: "Mandatory Track Property 0x4000 on an Object",
				},
			},
			{
				type: "event",
				event: {
					type: "request-closed",
					requestId: 0n,
					reason: "malformed-track",
				},
			},
			{ type: "reset", stream: 1, code: 18 },
		]);
	});

	test("PUBLISH_OK with FORWARD 0 resets open subgroup streams and stops Objects (§5.1, §11.4.3)", () => {
		const session = withRequests();
		session.openSubgroup(2n, SUBGROUP);

		expect(
			session.receiveRequestStreamData(2, hex("07 00 03 01 10 00"), false),
		).toEqual([
			{ type: "reset", stream: 3, code: 1 },
			{
				type: "event",
				event: {
					type: "publish-ok",
					requestId: 2n,
					parameters: { forward: false },
				},
			},
		]);
		expect(() =>
			session.writeObject(3, { objectId: 0n, payload: utf8("a") }),
		).toThrow("not an open subgroup stream");
		expect(() => session.openSubgroup(2n, SUBGROUP)).toThrow("Forward State 0");
	});

	test("a peer reset of a PUBLISH request stream resets its open subgroup streams (§5.1.1)", () => {
		const session = withRequests();
		session.openSubgroup(2n, SUBGROUP);
		expect(session.requestStreamReset(2)).toEqual([
			{ type: "reset", stream: 3, code: 1 },
			{
				type: "event",
				event: { type: "request-closed", requestId: 2n, reason: "peer-reset" },
			},
			{ type: "reset", stream: 2, code: 1 },
		]);
	});

	test("STOP_SENDING on the request stream of an Established PUBLISH ends the subscription (§5.1)", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.openSubgroup(2n, SUBGROUP);
		expect(session.localStreamStopped(2, 7)).toEqual([
			{ type: "reset", stream: 3, code: 1 },
			{
				type: "event",
				event: {
					type: "request-closed",
					requestId: 2n,
					reason: "peer-reset",
					resetCode: 7,
				},
			},
			{ type: "reset", stream: 2, code: 1 },
		]);
	});

	// A stop whose code the transport did not pass on (`localStreamAborted`)
	// is read as a stop, but for the control stream, stream 0: there the
	// reading is not certain enough to make a violation of the peer.
	// `withRequests()` has a PUBLISH on stream 2, here Established, and its
	// subgroup stream is 3.
	test.each<{ name: string; stream: number; expected: SessionOutput[] }>([
		{ name: "the control stream: nothing", stream: 0, expected: [] },
		{
			name: "a subgroup stream: it is reset, and the PUBLISH goes on",
			stream: 3,
			expected: [{ type: "reset", stream: 3, code: 1 }],
		},
		{
			name: "the request stream of the PUBLISH: the subscription ends, with no code",
			stream: 2,
			expected: [
				{ type: "reset", stream: 3, code: 1 },
				{
					type: "event",
					event: {
						type: "request-closed",
						requestId: 2n,
						reason: "peer-reset",
						resetCode: undefined,
					},
				},
				{ type: "reset", stream: 2, code: 1 },
			],
		},
	])("a stop without a code on $name", ({ stream, expected }) => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.openSubgroup(2n, SUBGROUP);
		expect(session.localStreamAborted(stream)).toEqual(expected);
		expect(session.closed).toBe(false);
	});

	test("a subgroup stream that was stopped without a code no longer keeps PUBLISH_DONE back, and is counted (§10.11)", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.openSubgroup(2n, SUBGROUP);
		session.localStreamAborted(3);
		// PUBLISH_DONE, Status Code 2, Stream Count 1, no reason.
		expect(session.publishDone(2n, 0x2n, "").at(0)).toEqual({
			type: "send",
			stream: 2,
			data: hex("0b 00 03 02 01 00"),
		});
	});

	test("a FIN of the subscriber on the request stream of an Established PUBLISH does not end the subscription (§5.1)", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.openSubgroup(2n, SUBGROUP);
		// The FIN alone: nothing ends, and Objects can still be written.
		expect(session.receiveRequestStreamData(2, hex(""), true)).toEqual([]);
		expect(
			session.writeObject(3, { objectId: 0n, payload: utf8("a") }),
		).toEqual([{ type: "send", stream: 3, data: hex("00 01 61") }]);
		// STOP_SENDING after it still ends the subscription.
		expect(session.localStreamStopped(2, 7)).toEqual([
			{ type: "reset", stream: 3, code: 1 },
			{
				type: "event",
				event: {
					type: "request-closed",
					requestId: 2n,
					reason: "peer-reset",
					resetCode: 7,
				},
			},
			{ type: "reset", stream: 2, code: 1 },
		]);
	});

	// STOP_SENDING closes only the direction this endpoint sends in.
	test.each([
		{ name: "a Pending SUBSCRIBE", stream: 1 },
		{ name: "a Pending PUBLISH", stream: 2 },
	])(
		"STOP_SENDING on the request stream of $name changes nothing",
		({ stream }) => {
			expect(withRequests().localStreamStopped(stream, 7)).toEqual([]);
		},
	);

	test("a SUBSCRIBE whose request stream was stopped still reads its REQUEST_ERROR", () => {
		const session = withRequests();
		session.localStreamStopped(1, 7);
		expect(
			session
				.receiveRequestStreamData(1, hex("05 00 05 10 00 02 6e 6f"), true)
				.map((output) =>
					output.type === "event" ? output.event.type : output.type,
				),
		).toEqual(["request-error"]);
	});

	test("PUBLISH_DONE is not written to a request stream the peer stopped", () => {
		const session = withRequests();
		session.localStreamStopped(2, 7);
		expect(session.publishDone(2n, 2n, "")).toEqual([]);
	});

	// §10.11: Objects can arrive after PUBLISH_DONE, whatever becomes of the
	// request stream.
	test.each([
		{
			name: "STOP_SENDING",
			act: (session) => session.localStreamStopped(1, 7),
		},
		{
			name: "a reset",
			act: (session) => session.requestStreamReset(1, 7),
		},
	] satisfies {
		name: string;
		act: (session: Session) => SessionOutput[];
	}[])(
		"a subscription that received PUBLISH_DONE keeps draining after $name on its request stream",
		({ act }) => {
			const session = subscribed();
			session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
			expect(act(session)).toEqual([]);
			expect(
				said(
					session.receivePeerUniStreamData(
						0,
						hex("14 09 00 00 00 00 01 61"),
						true,
					),
				).slice(0, 2),
			).toEqual([OBJECT_ON_ALIAS_9, ...CLOSED_AFTER_PUBLISH_DONE.slice(0, 1)]);
		},
	);

	test("STOP_SENDING on this endpoint's control stream closes the session (§3.3)", () => {
		expect(started().localStreamStopped(0, 0)).toMatchObject([
			{ type: "close-session", code: 3 },
		]);
	});

	// A TRACK_STATUS, which has the format of a SUBSCRIBE and another type
	// (§10.14). A SUBSCRIBE is handed to the application
	// (session-peer-subscribe.test.ts).
	test("answers a request the peer opens, and that is not served, with REQUEST_ERROR NOT_SUPPORTED", () => {
		const session = started();
		const [respond, event] = said(
			session.receivePeerBidiStreamData(0, hex("0d 00 1f 01"), false),
		);
		if (respond?.type !== "respond") {
			throw new Error("the first output is not a response");
		}
		expect(decodeControlMessage(respond.data)).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode: 3n, retryInterval: 0n },
		});
		expect(event).toEqual({
			type: "event",
			event: {
				type: "peer-request-rejected",
				messageType: 13n,
				requestId: 1n,
				errorCode: 3n,
			},
		});
	});

	// What the session refuses of a FETCH by itself. A FETCH is decoded whole
	// first, so that one the draft has close the session does (§10.12,
	// §10.2.1), and the refusals are the ones the session of the Rust
	// implementation makes for the same bytes: INVALID_RANGE (0x11) and
	// INVALID_JOINING_REQUEST_ID (0x32). A Standalone FETCH with a range that
	// is in order is handed to the application
	// (session-peer-fetch.test.ts, and the table of fixtures/peer-fetch/).
	// The Standalone ones are for the track ["a"] "b", with Request ID 1.
	test.each([
		{
			name: "a Standalone FETCH from {1, 5} to {1, 4}: INVALID_RANGE",
			bytes: "16 00 0c 01 01 01 01 61 01 62 01 05 01 04 00",
			errorCode: 0x11n,
		},
		{
			name: "one from {2, 0} to {1, 0}: INVALID_RANGE",
			bytes: "16 00 0c 01 01 01 01 61 01 62 02 00 01 00 00",
			errorCode: 0x11n,
		},
		{
			name: "a Relative Joining FETCH for Request ID 0: INVALID_JOINING_REQUEST_ID",
			bytes: "16 00 05 01 02 00 00 00",
			errorCode: 0x32n,
		},
		{
			name: "an Absolute Joining FETCH for Request ID 3: INVALID_JOINING_REQUEST_ID",
			bytes: "16 00 05 01 03 03 00 00",
			errorCode: 0x32n,
		},
	])("refuses $name", ({ bytes, errorCode }) => {
		// The session has a SUBSCRIBE of its own with Request ID 0, which is
		// not one of the peer's.
		const session = withRequests();
		const whole = hex(bytes);
		// Its start alone asks for nothing yet.
		expect(
			said(session.receivePeerBidiStreamData(0, whole.subarray(0, 5), false)),
		).toEqual([]);
		const [respond, event] = said(
			session.receivePeerBidiStreamData(0, whole.subarray(5), false),
		);
		if (respond?.type !== "respond") {
			throw new Error("the first output is not a response");
		}
		expect(decodeControlMessage(respond.data)).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode, retryInterval: 0n },
		});
		expect(event).toEqual({
			type: "event",
			event: {
				type: "peer-request-rejected",
				messageType: 22n,
				requestId: 1n,
				errorCode,
			},
		});
		expect(session.closed).toBe(false);
		// The Request ID is used up.
		expect(
			said(
				session.receivePeerBidiStreamData(1, hex("0d 00 02 01 00"), false),
			).at(-1),
		).toMatchObject({ type: "close-session", code: 4 });
	});

	// `code` is the Session termination error code of §3.5.
	test.each([
		{ name: "Fetch Type 4 (§10.12)", bytes: "16 00 03 01 04 00", code: 3 },
		{
			name: "FORWARD, which is no parameter of a FETCH (§10.2.1)",
			bytes: "16 00 07 01 02 00 00 01 10 01",
			code: 3,
		},
		{
			name: "an unknown Message Parameter (§10.2)",
			bytes: "16 00 07 01 02 00 00 01 01 00",
			code: 3,
		},
		{
			// Both wrong: the FETCH is decoded before its Request ID is looked
			// at, as in the Rust implementation, so the code is that of the
			// message.
			name: "Fetch Type 4 and an even Request ID",
			bytes: "16 00 03 02 04 00",
			code: 3,
		},
	])(
		"closes the session on a FETCH of the peer with $name",
		({ bytes, code }) => {
			expect(
				said(started().receivePeerBidiStreamData(0, hex(bytes), false)).at(-1),
			).toMatchObject({ type: "close-session", code });
		},
	);

	test.each([
		{
			// FETCH_OK: End Of Track 0, End Location {1, 2}, no parameters.
			name: "in answer to a SUBSCRIBE",
			feed: (session: Session) =>
				session.receiveRequestStreamData(1, hex("18 00 04 00 01 02 00"), false),
		},
		{
			name: "at the start of a stream the peer opened (§3.3)",
			feed: (session: Session) =>
				said(
					session.receivePeerBidiStreamData(
						0,
						hex("18 00 04 00 01 02 00"),
						false,
					),
				),
		},
	])(
		"closes the session with PROTOCOL_VIOLATION on a FETCH_OK that answers no FETCH, $name",
		({ feed }) => {
			expect(feed(withRequests()).at(-1)).toMatchObject({
				type: "close-session",
				code: 3,
			});
		},
	);

	// §10: a Message Length that its payload does not match is a
	// PROTOCOL_VIOLATION, and a stream that ends inside its first message is
	// treated as that, as in the Rust implementation.
	test.each([
		{ name: "a FETCH", bytes: "16 00 05 01 02 00" },
		{ name: "a TRACK_STATUS, after its Request ID", bytes: "0d 00 09 01 00" },
		{ name: "a SUBSCRIBE, inside its Request ID", bytes: "03 00 1f 80" },
		{ name: "a request, after its Message Type", bytes: "03" },
	])(
		"closes the session when the stream of $name ends inside the message",
		({ bytes }) => {
			expect(
				said(started().receivePeerBidiStreamData(0, hex(bytes), true)).at(-1),
			).toMatchObject({ type: "close-session", code: 3 });
		},
	);

	test("closes the session when a request that was refused on its start ends before it is whole", () => {
		const session = started();
		// A TRACK_STATUS of 31 bytes, of which one has come: it is refused.
		expect(
			said(session.receivePeerBidiStreamData(0, hex("0d 00 1f 01"), false)).map(
				(output) => output.type,
			),
		).toEqual(["respond", "event"]);
		// Ten more bytes, and the end.
		expect(
			said(session.receivePeerBidiStreamData(0, new Uint8Array(10), true)).at(
				-1,
			),
		).toMatchObject({ type: "close-session", code: 3 });
	});

	test("a stream of the peer that ends without a byte asked for nothing", () => {
		const session = started();
		expect(
			said(session.receivePeerBidiStreamData(0, new Uint8Array(0), true)),
		).toEqual([]);
		expect(session.closed).toBe(false);
	});

	// Each entry of `requests` is the start of a request on its own stream.
	// `code` is the Session termination error code of §3.5.
	test.each([
		{
			name: "an even Request ID from the server (§10.1)",
			requests: ["0d 00 1f 02"],
			code: 4,
		},
		{
			name: "a duplicate Request ID (§10.1)",
			requests: ["0d 00 1f 01", "0d 00 02 01 00"],
			code: 4,
		},
	])("closes the session on $name", ({ requests, code }) => {
		const session = started(SMALL_LIMITS);
		const outputs = requests.map((bytes, peerStream) =>
			said(session.receivePeerBidiStreamData(peerStream, hex(bytes), false)),
		);
		expect(outputs.at(-1)?.at(-1)).toMatchObject({
			type: "close-session",
			code,
		});
	});

	test("a gap in the peer's Request IDs never closes the session (§10.1)", () => {
		const session = started();
		// Request ID 1 never arrives; 3, 5, ... do, 1,100 of them. Each is the
		// start of a TRACK_STATUS whose Request ID is written as a 9-byte
		// varint. WHY not a FETCH, the other request that is not served: a
		// FETCH is decoded whole before it is refused, and these are not whole.
		for (let index = 0; index < 1100; index++) {
			const requestId = (3 + index * 2).toString(16).padStart(16, "0");
			const bytes = `0d 00 09 ff ${requestId.match(/../g)?.join(" ")}`;
			said(session.receivePeerBidiStreamData(index, hex(bytes), true));
		}
		expect(session.closed).toBe(false);
		// The late request with Request ID 1 is still a new request.
		expect(
			said(
				session.receivePeerBidiStreamData(2000, hex("0d 00 02 01 00"), true),
			).map((output) => output.type),
			// The refusal, that it was refused, and that the peer has ended
			// its direction.
		).toEqual(["respond", "event", "event"]);
	});

	test("records the Request ID of a REQUEST_UPDATE that crosses this endpoint's PUBLISH_DONE (§10.1)", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.publishDone(2n, 2n, "");
		// The update, with Request ID 1, is ignored.
		expect(
			session.receiveRequestStreamData(2, hex("02 00 02 01 00"), false),
		).toEqual([]);
		// Its Request ID was used all the same: a request that uses it again is a duplicate.
		expect(
			said(
				session.receivePeerBidiStreamData(0, hex("0d 00 02 01 00"), false),
			).at(-1),
		).toMatchObject({ type: "close-session", code: 4 });
	});

	// The notes, "A publisher that ended its PUBLISH with PUBLISH_DONE": how
	// the subscriber ends its direction afterwards is reported.
	test.each([
		{
			name: "a FIN",
			end: (session: Session) =>
				session.receiveRequestStreamData(2, new Uint8Array(0), true),
			event: { type: "peer-direction-ended", requestId: 2n, how: "fin" },
		},
		{
			name: "a reset",
			end: (session: Session) => session.requestStreamReset(2, 1),
			event: {
				type: "peer-direction-ended",
				requestId: 2n,
				how: "reset",
				resetCode: 1,
			},
		},
	])(
		"after PUBLISH_DONE, reports $name of the subscriber's direction once",
		({ end, event }) => {
			const session = withRequests();
			session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
			session.publishDone(2n, 2n, "");
			expect(end(session)).toEqual([{ type: "event", event }]);
			// The stream is forgotten: neither a second end nor the expiry does anything.
			expect(session.requestStreamReset(2, 1)).toEqual([]);
			expect(session.expirePeerEnd(2n)).toEqual([]);
		},
	);

	test("PUBLISH_DONE reports the FIN of a subscriber that had finished its direction", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), true);
		expect(session.publishDone(2n, 2n, "")).toEqual([
			{ type: "send", stream: 2, data: hex("0b 00 03 02 00 00") },
			{ type: "finish", stream: 2 },
			{
				type: "event",
				event: { type: "peer-direction-ended", requestId: 2n, how: "fin" },
			},
		]);
	});

	test("the expired wait after PUBLISH_DONE stops the subscriber's direction and reads no more of it", () => {
		const session = withRequests();
		session.receiveRequestStreamData(2, hex("07 00 01 00"), false);
		session.publishDone(2n, 2n, "");
		expect(session.expirePeerEnd(2n)).toEqual([
			{ type: "stop-peer-direction", stream: 2, code: 1 },
		]);
		expect(
			session.receiveRequestStreamData(2, new Uint8Array(0), true),
		).toEqual([]);
		// A request that did not send PUBLISH_DONE has no such wait.
		expect(session.expirePeerEnd(0n)).toEqual([]);
	});

	test("a SUBSCRIBE_OK that is refused does not take the Track Alias of a draining subscription (§2.5.1)", () => {
		const session = subscribed();
		session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
		session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });

		// SUBSCRIBE_OK for Track Alias 9 with the Mandatory Track Property 0x4000.
		expect(
			session.receiveRequestStreamData(
				2,
				hex("04 00 06 09 00 c0 40 00 01"),
				false,
			),
		).toEqual([
			{
				type: "event",
				event: {
					type: "request-closed",
					requestId: 2n,
					reason: "unsupported-track-property",
				},
			},
			{ type: "reset", stream: 2, code: 1 },
		]);
		// The draining subscription still gets its late stream, and then ends.
		expect(
			said(
				session.receivePeerUniStreamData(
					0,
					hex("14 09 00 00 00 00 01 61"),
					true,
				),
			),
		).toEqual([OBJECT_ON_ALIAS_9, ...CLOSED_AFTER_PUBLISH_DONE]);
	});

	test("counts a subgroup stream that was reset before SUBSCRIBE_OK (§10.11)", () => {
		const session = started();
		session.subscribe(TRACK);
		said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false));
		session.peerUniStreamReset(0);
		session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
		expect(
			session
				.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false)
				.slice(1),
		).toEqual(CLOSED_AFTER_PUBLISH_DONE);
	});

	test("counts early-ended streams for 64 unknown Track Aliases and forgets the oldest", () => {
		const session = started();
		session.subscribe(TRACK);
		// One stream without an Object for Track Alias 9, then one each for 64
		// other aliases (0x40 to 0x7f, written as 2-byte varints).
		said(session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), true));
		for (let index = 0; index < 64; index++) {
			const alias = (0x40 + index).toString(16);
			said(
				session.receivePeerUniStreamData(
					1 + index,
					hex(`14 80 ${alias} 00 00 00`),
					true,
				),
			);
		}
		session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
		// PUBLISH_DONE with Stream Count 1: the count for alias 9 was dropped, so
		// the subscription goes on waiting.
		expect(
			session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false),
		).toHaveLength(1);
	});

	test("refuses a second PUBLISH with a Track Alias in use (§11.1)", () => {
		const session = withRequests();
		expect(() =>
			session.publish({
				trackNamespace: NAMESPACE,
				trackName: utf8("other"),
				trackAlias: 1n,
			}),
		).toThrow(RangeError);
	});

	test("DUPLICATE_TRACK_ALIAS wins over an unknown Mandatory Track Property in the same SUBSCRIBE_OK", () => {
		const session = subscribed();
		session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });
		expect(
			session.receiveRequestStreamData(
				2,
				hex("04 00 06 09 00 c0 40 00 01"),
				false,
			),
		).toEqual([
			{
				type: "close-session",
				code: 5,
				reason: "Track Alias 9 is in use by another subscription",
			},
		]);
	});

	test("a message after REQUEST_ERROR closes the session even in a later chunk (§5.1)", () => {
		const session = started();
		session.subscribe(TRACK);
		session.receiveRequestStreamData(1, hex("05 00 05 10 00 02 6e 6f"), false);
		expect(
			session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false).at(-1),
		).toMatchObject({ type: "close-session", code: 3 });
	});

	test("a message after PUBLISH_DONE closes the session (§10.11)", () => {
		const session = subscribed();
		session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
		expect(
			session
				.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false)
				.at(-1),
		).toMatchObject({ type: "close-session", code: 3 });
	});

	test("a Redirect with a Track Name for PUBLISH_NAMESPACE closes the session (§10.6.1)", () => {
		const session = started();
		session.publishNamespace({ trackNamespace: NAMESPACE });
		expect(
			session
				.receiveRequestStreamData(
					1,
					hex("05 00 07 34 01 00 00 00 01 63"),
					false,
				)
				.at(-1),
		).toMatchObject({ type: "close-session", code: 3 });
	});

	// The draft has no rule for it. It is read like a stream that ends
	// inside an Object (§11.4), as the Rust implementation reads it.
	// `80` starts a type of two bytes (§1.4.1).
	test.each([
		{ name: "in one read", chunks: ["80"] },
		{ name: "after a read that brought the first byte", chunks: ["80", ""] },
	])(
		"closes the session when a unidirectional stream ends inside its type, $name",
		({ chunks }) => {
			const session = started();
			const outputs = chunks.map((chunk, index) =>
				said(
					session.receivePeerUniStreamData(
						0,
						hex(chunk),
						index === chunks.length - 1,
					),
				),
			);
			expect(outputs.at(-1)).toMatchObject([
				{ type: "close-session", code: 3 },
			]);
		},
	);

	test("a unidirectional stream that ends without a byte, or is reset inside its type, carried nothing (§11.4.1)", () => {
		const session = started();
		expect(said(session.receivePeerUniStreamData(0, hex(""), true))).toEqual(
			[],
		);
		expect(said(session.receivePeerUniStreamData(1, hex("80"), false))).toEqual(
			[],
		);
		expect(session.peerUniStreamReset(1)).toEqual([]);
		expect(session.closed).toBe(false);
	});

	// `withRequests()` state: stream 1 is a Pending SUBSCRIBE, stream 2 a Pending
	// PUBLISH. `code` is the Session termination error code of §3.5.
	test.each([
		{
			name: "an unknown stream type",
			on: "peer-uni",
			bytes: "06",
			fin: false,
			code: 3,
		},
		{
			name: "a SUBGROUP_HEADER Type with the reserved SUBGROUP_ID_MODE",
			on: "peer-uni",
			bytes: "3f",
			fin: false,
			code: 3,
		},
		{
			name: "a fetch stream for a request that is no FETCH",
			on: "peer-uni",
			bytes: "05 00",
			fin: false,
			code: 3,
		},
		{
			name: "a second SETUP",
			on: "peer-uni",
			bytes: "af 00 00 00 af 00 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "a PATH option from the server",
			on: "peer-uni",
			bytes: "af 00 00 03 01 01 2f",
			fin: false,
			code: 8,
		},
		{
			name: "an AUTHORITY option from the server",
			on: "peer-uni",
			bytes: "af 00 00 03 05 01 61",
			fin: false,
			code: 25,
		},
		{
			name: "a FIN on the control stream",
			on: "peer-uni",
			bytes: "af 00 00 00",
			fin: true,
			code: 3,
		},
		// §10.4, a GOAWAY on the control stream.
		{
			name: "a GOAWAY without a Request ID",
			on: "peer-uni",
			bytes: "af 00 00 00 10 00 02 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "a GOAWAY with the odd Request ID of a server",
			on: "peer-uni",
			bytes: "af 00 00 00 10 00 03 00 00 01",
			fin: false,
			code: 4,
		},
		{
			name: "a second GOAWAY",
			on: "peer-uni",
			bytes: "af 00 00 00 10 00 03 00 00 02 10 00 03 00 00 02",
			fin: false,
			code: 3,
		},
		// §10.4, a GOAWAY on a request stream.
		{
			name: "a GOAWAY on a request stream with a Request ID",
			on: "subscribe",
			bytes: "10 00 03 00 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "a second GOAWAY on one request stream",
			on: "subscribe",
			bytes: "10 00 02 00 00 10 00 02 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "a FIN inside an Object",
			on: "peer-uni",
			bytes: "14 00 00 00 00 00 02 61",
			fin: true,
			code: 3,
		},
		{
			name: "REQUEST_OK for SUBSCRIBE",
			on: "subscribe",
			bytes: "07 00 01 00",
			fin: false,
			code: 3,
		},
		{
			name: "two SUBSCRIBE_OK",
			on: "subscribe",
			bytes: "04 00 02 00 00 04 00 02 01 00",
			fin: false,
			code: 3,
		},
		{
			name: "a FIN inside a response",
			on: "subscribe",
			bytes: "04 00",
			fin: true,
			code: 3,
		},
		{
			name: "SUBSCRIBE_OK for PUBLISH",
			on: "publish",
			bytes: "04 00 02 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "Track Properties in PUBLISH_OK",
			on: "publish",
			bytes: "07 00 03 00 02 01",
			fin: false,
			code: 3,
		},
		{
			name: "LARGEST_OBJECT in PUBLISH_OK",
			on: "publish",
			bytes: "07 00 04 01 09 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "REQUEST_UPDATE on a SUBSCRIBE request stream (§10.9)",
			on: "subscribe",
			bytes: "02 00 02 01 00",
			fin: false,
			code: 3,
		},
		{
			name: "REQUEST_UPDATE for a PUBLISH that is still Pending (§10.9)",
			on: "publish",
			bytes: "02 00 02 01 00",
			fin: false,
			code: 3,
		},
		{
			name: "a peer stream that begins with SUBSCRIBE_OK",
			on: "peer-bidi",
			bytes: "04 00 02 00 00",
			fin: false,
			code: 3,
		},
		// A SUBSCRIBE of the peer, refused for want of `serve`, and then
		// GOAWAYs on its stream, held to §10.4 as the Rust session holds them.
		{
			name: "a second GOAWAY on the stream of a refused request of the peer",
			on: "peer-bidi",
			bytes:
				"03 00 0a 01 01 04 78 6d 6f 71 01 61 00 10 00 02 00 00 10 00 02 00 00",
			fin: false,
			code: 3,
		},
		{
			name: "a GOAWAY with a Request ID on the stream of a refused request of the peer",
			on: "peer-bidi",
			bytes: "03 00 0a 01 01 04 78 6d 6f 71 01 61 00 10 00 03 00 00 00",
			fin: false,
			code: 3,
		},
	] satisfies {
		name: string;
		on: "peer-uni" | "subscribe" | "publish" | "peer-bidi";
		bytes: string;
		fin: boolean;
		code: number;
	}[])("closes the session on $name", ({ on, bytes, fin, code }) => {
		const session = withRequests();
		const feed: Record<typeof on, () => SessionOutput[]> = {
			"peer-uni": () =>
				said(session.receivePeerUniStreamData(0, hex(bytes), fin)),
			subscribe: () => session.receiveRequestStreamData(1, hex(bytes), fin),
			publish: () => session.receiveRequestStreamData(2, hex(bytes), fin),
			"peer-bidi": () =>
				said(session.receivePeerBidiStreamData(0, hex(bytes), fin)),
		};
		expect(feed[on]().at(-1)).toMatchObject({ type: "close-session", code });
		expect(session.closed).toBe(true);
		expect(said(session.receivePeerUniStreamData(5, hex("3f"), false))).toEqual(
			[],
		);
	});
});
