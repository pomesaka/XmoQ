import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import type { SessionLimits } from "../../src/core/limits.ts";
import type { SubgroupOptions } from "../../src/core/outgoing-subgroups.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex, MOQTAIL_RELAY_SETUP, utf8 } from "../helpers.ts";

const NAMESPACE = [utf8("xmoq"), utf8("phase0")];
const TRACK_NAME = utf8("object");
const TRACK = { trackNamespace: NAMESPACE, trackName: TRACK_NAME };

/** Small enough to reach in a test. A held Object without payload counts 64 bytes. */
const SMALL_LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16,
	maxObjectPropertiesLength: 16,
	maxHeldSubgroupBytes: 64,
};

function started(limits?: SessionLimits): Session {
	const session = createSession({ limits });
	session.start();
	return session;
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
			session.receivePeerUniStreamData(0, hex(MOQTAIL_RELAY_SETUP), false),
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
			session.receivePeerUniStreamData(1, hex("14 00 03 00 80 00 01 61"), true),
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
			session.receivePeerUniStreamData(0, hex("14 09 00 00 00 00 01 61"), true),
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
			session.receivePeerUniStreamData(
				0,
				hex("14 09 00 00 00 00 00 00"),
				false,
			),
		).toEqual([]);
		expect(session.receivePeerUniStreamData(0, hex("00 00 00"), false)).toEqual(
			[
				{ type: "stop-sending", peerStream: 0, code: 0 },
				{
					type: "event",
					event: {
						type: "subgroup-abandoned",
						trackAlias: 9n,
						reason: "too many Objects before SUBSCRIBE_OK",
					},
				},
			],
		);
	});

	test("stops a subgroup stream whose Track Alias no subscription can establish", () => {
		const session = started();
		expect(
			session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false),
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
		session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false);
		// A second SUBSCRIBE is Pending, so an unknown alias would be held.
		session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });

		expect(session.cancelRequest(0n)).toEqual([
			{ type: "stop-sending", peerStream: 0, code: 1 },
			{ type: "reset", stream: 1, code: 1 },
		]);
		expect(session.receivePeerUniStreamData(0, hex("00 01 61"), false)).toEqual(
			[],
		);
		expect(
			session.receivePeerUniStreamData(1, hex("14 09 01 00 00"), false),
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
		session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), true);
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

		// REQUEST_UPDATE with Request ID 1 and no parameters.
		const outputs = session.receiveRequestStreamData(
			2,
			hex("02 00 02 01 00"),
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
			session.receivePeerUniStreamData(0, hex("14 09 00 00 00 00 01 61"), true),
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
			session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false),
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
			session.receivePeerUniStreamData(
				0,
				hex("15 09 00 00 00 00 04 c0 40 00 01 01 61"),
				false,
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
				session
					.receivePeerUniStreamData(0, hex("14 09 00 00 00 00 01 61"), true)
					.slice(0, 2),
			).toEqual([OBJECT_ON_ALIAS_9, ...CLOSED_AFTER_PUBLISH_DONE.slice(0, 1)]);
		},
	);

	test("STOP_SENDING on this endpoint's control stream closes the session (§3.3)", () => {
		expect(started().localStreamStopped(0, 0)).toMatchObject([
			{ type: "close-session", code: 3 },
		]);
	});

	test("answers a request the peer opens with REQUEST_ERROR NOT_SUPPORTED", () => {
		const session = started();
		const [respond, event] = session.receivePeerBidiStreamData(
			0,
			hex("03 00 1f 01"),
			false,
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
			event: { type: "peer-request-rejected", messageType: 3n, requestId: 1n },
		});
	});

	// Each entry of `requests` is the start of a request on its own stream.
	// `code` is the Session termination error code of §3.5.
	test.each([
		{
			name: "an even Request ID from the server (§10.1)",
			requests: ["03 00 1f 02"],
			code: 4,
		},
		{
			name: "a duplicate Request ID (§10.1)",
			requests: ["03 00 1f 01", "16 00 02 01 00"],
			code: 4,
		},
	])("closes the session on $name", ({ requests, code }) => {
		const session = started(SMALL_LIMITS);
		const outputs = requests.map((bytes, peerStream) =>
			session.receivePeerBidiStreamData(peerStream, hex(bytes), false),
		);
		expect(outputs.at(-1)?.at(-1)).toMatchObject({
			type: "close-session",
			code,
		});
	});

	test("a gap in the peer's Request IDs never closes the session (§10.1)", () => {
		const session = started();
		// Request ID 1 never arrives; 3, 5, ... do, 1,100 of them. Each is the
		// start of a FETCH whose Request ID is written as a 9-byte varint.
		for (let index = 0; index < 1100; index++) {
			const requestId = (3 + index * 2).toString(16).padStart(16, "0");
			const bytes = `16 00 09 ff ${requestId.match(/../g)?.join(" ")}`;
			session.receivePeerBidiStreamData(index, hex(bytes), true);
		}
		expect(session.closed).toBe(false);
		// The late request with Request ID 1 is still a new request.
		expect(
			session
				.receivePeerBidiStreamData(2000, hex("16 00 02 01 00"), true)
				.map((output) => output.type),
		).toEqual(["respond", "event"]);
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
			session.receivePeerBidiStreamData(0, hex("16 00 02 01 00"), false).at(-1),
		).toMatchObject({ type: "close-session", code: 4 });
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
			session.receivePeerUniStreamData(0, hex("14 09 00 00 00 00 01 61"), true),
		).toEqual([OBJECT_ON_ALIAS_9, ...CLOSED_AFTER_PUBLISH_DONE]);
	});

	test("counts a subgroup stream that was reset before SUBSCRIBE_OK (§10.11)", () => {
		const session = started();
		session.subscribe(TRACK);
		session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), false);
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
		session.receivePeerUniStreamData(0, hex("14 09 00 00 00"), true);
		for (let index = 0; index < 64; index++) {
			const alias = (0x40 + index).toString(16);
			session.receivePeerUniStreamData(
				1 + index,
				hex(`14 80 ${alias} 00 00 00`),
				true,
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

	test("drops a unidirectional stream that ends inside its type", () => {
		const session = started();
		expect(session.receivePeerUniStreamData(0, hex("80"), true)).toEqual([]);
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
			name: "FETCH_HEADER, not implemented",
			on: "peer-uni",
			bytes: "05 00",
			fin: false,
			code: 1,
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
		{
			name: "GOAWAY, not implemented",
			on: "peer-uni",
			bytes: "af 00 00 00 10 00 02 00 00",
			fin: false,
			code: 1,
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
	] satisfies {
		name: string;
		on: "peer-uni" | "subscribe" | "publish" | "peer-bidi";
		bytes: string;
		fin: boolean;
		code: number;
	}[])("closes the session on $name", ({ on, bytes, fin, code }) => {
		const session = withRequests();
		const feed: Record<typeof on, () => SessionOutput[]> = {
			"peer-uni": () => session.receivePeerUniStreamData(0, hex(bytes), fin),
			subscribe: () => session.receiveRequestStreamData(1, hex(bytes), fin),
			publish: () => session.receiveRequestStreamData(2, hex(bytes), fin),
			"peer-bidi": () => session.receivePeerBidiStreamData(0, hex(bytes), fin),
		};
		expect(feed[on]().at(-1)).toMatchObject({ type: "close-session", code });
		expect(session.closed).toBe(true);
		expect(session.receivePeerUniStreamData(5, hex("3f"), false)).toEqual([]);
	});
});
