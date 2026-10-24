import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import {
	connectOver,
	type MoqtConnection,
	type PublishedTrack,
	type SessionConnectOptions,
	type Subscription,
} from "../../src/webtransport/connection.ts";
import {
	type PublishEnd,
	PublishEndedError,
	type RequestAbort,
	RequestAbortedError,
	RequestFailedError,
	type SessionClose,
	SessionClosedError,
	SubgroupClosedError,
	type SubscriptionEnd,
	SubscriptionEndedError,
} from "../../src/webtransport/endings.ts";
import { hex, MOQTAIL_RELAY_SETUP, toHex, utf8 } from "../helpers.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

// The peer's side is played with literal bytes; the ones marked "moqtail" are
// in test/core/moqtail-fixtures.test.ts.
const SUBSCRIBE_OK_ALIAS_9 = "04 00 02 09 00";
const TRACK = {
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
};
const END_OF_STREAM = { done: true, value: undefined } as const;
const SUBGROUP = {
	groupId: 0n,
	subgroupId: { mode: "Zero" },
	properties: false,
	endOfGroup: false,
} as const;

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

async function sentBytes(
	reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<Uint8Array> {
	const { value } = await reader.read();
	if (value === undefined) throw new Error("the stream ended");
	return value;
}

async function sentHex(
	reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<string> {
	return toHex(await sentBytes(reader));
}

/** What a promise rejects with. Fails the test when it resolves. */
async function rejection(promise: Promise<unknown>): Promise<unknown> {
	const outcome = await promise.then(
		() => ({ rejected: false }) as const,
		(error: unknown) => ({ rejected: true, error }) as const,
	);
	if (!outcome.rejected) throw new Error("the promise resolved");
	return outcome.error;
}

/** Connects and completes the SETUP exchange. */
async function connected(
	options: SessionConnectOptions = {},
): Promise<{ peer: FakePeer; connection: MoqtConnection }> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport, options);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	return { peer, connection: await connecting };
}

/** Subscribes and lets the peer accept with Track Alias 9. */
async function subscribed(options: SessionConnectOptions = {}): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	request: PeerBidiStream;
	subscription: Subscription;
}> {
	const { peer, connection } = await connected(options);
	await peer.clientUni();
	const subscribing = connection.subscribe(TRACK);
	const request = await peer.clientBidi();
	await request.sent.read();
	await request.reply.write(hex(SUBSCRIBE_OK_ALIAS_9));
	return { peer, connection, request, subscription: await subscribing };
}

/** Publishes with Track Alias 1 and lets the peer accept. */
async function published(options: SessionConnectOptions = {}): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	request: PeerBidiStream;
	track: PublishedTrack;
}> {
	const { peer, connection } = await connected(options);
	await peer.clientUni();
	const publishing = connection.publish({ ...TRACK, trackAlias: 1n });
	const request = await peer.clientBidi();
	await request.sent.read();
	await request.reply.write(hex("07 00 01 00"));
	return { peer, connection, request, track: await publishing };
}

/** A subgroup stream for Track Alias 9 with Explicit Subgroup ID `group`, in Group `group`. */
function subgroupHeader(group: number): Uint8Array {
	return Uint8Array.of(0x14, 0x09, group, group, 0x00);
}

/** One Object with a one-byte payload, in the delta encoding of a sequential stream. */
function objectWith(payloadByte: number): Uint8Array {
	return Uint8Array.of(0x00, 0x01, payloadByte);
}

describe("the connection's AUTHORIZATION TOKEN (§10.2.2)", () => {
	const TOKEN = Uint8Array.of(0x03, 0x00, 0x61, 0x62);
	const OWN = Uint8Array.of(0x03, 0x00, 0x7a);
	/** The tokens of the first message a request stream carries, in hex. */
	const tokensOf = (bytes: Uint8Array): string[] => {
		const decoded = decodeControlMessage(bytes);
		if (decoded.status !== "ok") return [`not a message: ${decoded.status}`];
		const message = decoded.value;
		return "parameters" in message
			? (message.parameters.authorizationToken ?? []).map((token) =>
					toHex(token),
				)
			: [`a ${message.type} has no parameters`];
	};

	test.each<{
		name: string;
		options: SessionConnectOptions;
		tokens: string[];
		withItsOwn: string[];
	}>([
		{
			name: "a connection with a token puts it into every request, before the request's own",
			options: { authorizationToken: TOKEN },
			tokens: ["03 00 61 62"],
			withItsOwn: ["03 00 61 62", "03 00 7a"],
		},
		{
			name: "a connection without one adds nothing",
			options: {},
			tokens: [],
			withItsOwn: ["03 00 7a"],
		},
	])("$name", async ({ options, tokens, withItsOwn }) => {
		const { peer, connection } = await connected(options);
		await peer.clientUni();
		const sent: Record<string, string[]> = {};
		// The requests are not answered: what was sent is all that is looked at.
		connection.publishNamespace(TRACK.trackNamespace).catch(() => {});
		sent.PUBLISH_NAMESPACE = tokensOf(
			await sentBytes((await peer.clientBidi()).sent),
		);
		connection.publish({ ...TRACK, trackAlias: 1n }).catch(() => {});
		sent.PUBLISH = tokensOf(await sentBytes((await peer.clientBidi()).sent));
		connection.subscribe(TRACK).catch(() => {});
		sent.SUBSCRIBE = tokensOf(await sentBytes((await peer.clientBidi()).sent));
		await connection.fetch({
			range: {
				fetchType: "Standalone",
				...TRACK,
				startLocation: { group: 0n, object: 0n },
				endLocation: { group: 1n, object: 0n },
			},
		});
		sent.FETCH = tokensOf(await sentBytes((await peer.clientBidi()).sent));
		connection
			.subscribe({ ...TRACK, parameters: { authorizationToken: [OWN] } })
			.catch(() => {});
		sent["SUBSCRIBE with its own"] = tokensOf(
			await sentBytes((await peer.clientBidi()).sent),
		);
		expect(sent).toEqual({
			PUBLISH_NAMESPACE: tokens,
			PUBLISH: tokens,
			SUBSCRIBE: tokens,
			FETCH: tokens,
			"SUBSCRIBE with its own": withItsOwn,
		});
		connection.close();
	});
});

describe("the connection's AUTHORIZATION TOKEN in SETUP (§10.3.1.4)", () => {
	test.each<{ name: string; options: SessionConnectOptions; setup: string }>([
		{
			name: "a connection with a token sends it as Setup Option 0x03, before MOQT_IMPLEMENTATION",
			options: {
				implementation: "xmoq/0",
				authorizationToken: Uint8Array.of(0x03, 0x00, 0x61, 0x62),
			},
			setup: "af 00 00 0e 03 04 03 00 61 62 04 06 78 6d 6f 71 2f 30",
		},
		{
			name: "a connection without one sends SETUP as before",
			options: { implementation: "xmoq/0" },
			setup: "af 00 00 08 07 06 78 6d 6f 71 2f 30",
		},
	])("$name", async ({ options, setup }) => {
		const peer = createFakePeer();
		const connecting = connectOver(peer.transport, options);
		expect(await sentHex(await peer.clientUni())).toBe(setup);
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		(await connecting).close();
	});
});

describe("connectOver", () => {
	test("sends SETUP on a new control stream and resolves with the peer's Setup Options", async () => {
		const peer = createFakePeer();
		const connecting = connectOver(peer.transport, {
			implementation: "xmoq/0",
		});
		expect(await sentHex(await peer.clientUni())).toBe(
			"af 00 00 08 07 06 78 6d 6f 71 2f 30",
		);
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		expect(connection.peerSetupOptions).toEqual([
			{ type: 7n, value: utf8("moqtail/0.15.0+dd8623d") },
		]);
	});

	test("closes the transport with CONTROL_MESSAGE_TIMEOUT when no SETUP arrives", async () => {
		const peer = createFakePeer();
		const error = await rejection(
			connectOver(peer.transport, { setupTimeoutMs: 5 }),
		);
		expect(error).toBeInstanceOf(SessionClosedError);
		expect(error).toMatchObject({
			close: { type: "error", error: { code: 17 } },
		});
		expect(peer.closeInfo()).toEqual({
			closeCode: 17,
			reason: "the peer sent no SETUP in time",
		});
	});

	test("subscribes, and delivers an Object from a subgroup stream", async () => {
		const { peer, connection } = await connected();
		await peer.clientUni();

		const subscribing = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		expect(await sentHex(request.sent)).toBe(
			"03 00 16 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 00",
		);
		await request.reply.write(hex(SUBSCRIBE_OK_ALIAS_9));
		const subscription = await subscribing;
		expect(subscription.trackAlias).toBe(9n);

		await peer.openUni().write(hex("14 09 00 00 00 00 01 61"));
		const { value } = await subscription.objects.getReader().read();
		expect(value).toEqual({
			groupId: 0n,
			subgroupId: 0n,
			publisherPriority: 0,
			object: {
				objectId: 0n,
				properties: [],
				status: "Normal",
				payload: utf8("a"),
			},
		});
	});

	test("keeps reading every subgroup stream of a subscription after its queue was full", async () => {
		const { peer, subscription } = await subscribed({
			subscriptionHighWaterMark: 1,
		});
		// Three streams deliver at once into a queue that holds one Object, so
		// all three pumps wait for the reader at the same time.
		const streams = [peer.openUni(), peer.openUni(), peer.openUni()];
		await Promise.all(
			streams.map(async (stream, group) => {
				await stream.write(subgroupHeader(group));
				await stream.write(objectWith(0x61));
			}),
		);
		await settle();

		const reader = subscription.objects.getReader();
		const first = [
			(await reader.read()).value?.groupId,
			(await reader.read()).value?.groupId,
			(await reader.read()).value?.groupId,
		];
		expect(first.sort()).toEqual([0n, 1n, 2n]);

		// Each stream is still being read: a second Object arrives from all three.
		await Promise.all(streams.map((stream) => stream.write(objectWith(0x62))));
		const second = [
			(await reader.read()).value?.groupId,
			(await reader.read()).value?.groupId,
			(await reader.read()).value?.groupId,
		];
		expect(second.sort()).toEqual([0n, 1n, 2n]);
	});

	test("a subscription nobody reads does not hold up other requests", async () => {
		const { peer, connection } = await subscribed({
			subscriptionHighWaterMark: 1,
		});
		// Two Objects fill the queue of one; the stream's pump now waits.
		const stream = peer.openUni();
		await stream.write(subgroupHeader(0));
		await stream.write(objectWith(0x61));
		await stream.write(objectWith(0x62));
		await settle();

		const announcing = connection.publishNamespace(TRACK.trackNamespace);
		const request = await peer.clientBidi();
		await request.sent.read();
		await request.reply.write(hex("07 00 01 00"));
		await announcing;
	});

	test("rejects subscribe() with the REQUEST_ERROR of the peer", async () => {
		const { peer, connection } = await connected();
		const subscribing = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		// moqtail: REQUEST_ERROR DOES_NOT_EXIST "Unknown track namespace".
		await request.reply.write(
			hex(
				"05 00 1a 10 00 17 55 6e 6b 6e 6f 77 6e 20 74 72 61 63 6b 20 6e 61 6d 65 73 70 61 63 65",
			),
		);
		const error = await rejection(subscribing);
		expect(error).toBeInstanceOf(RequestFailedError);
		expect(error).toMatchObject({
			errorCode: 16n,
			retryInterval: 0n,
			errorReason: "Unknown track namespace",
		});
	});

	// A request that ends without a response rejects with what ended it.
	test.each([
		{
			name: "a FIN",
			act: (request) => request.reply.close(),
			abort: { type: "peer-closed" },
		},
		{
			name: "a reset with code 5",
			act: (request) => request.reply.abort(fakeStreamError(5, "reset")),
			abort: { type: "peer-reset", code: 5 },
		},
		{
			name: "a SUBSCRIBE_OK with an unknown Mandatory Track Property (§2.5.1)",
			act: (request) => request.reply.write(hex("04 00 06 09 00 c0 40 00 01")),
			abort: { type: "unsupported-track-property" },
		},
		{
			name: "a SUBSCRIBE_OK with Immutable Properties inside Immutable Properties (§12.7)",
			act: (request) => request.reply.write(hex("04 00 06 09 00 0b 02 0b 00")),
			abort: { type: "malformed-track" },
		},
	] satisfies {
		name: string;
		act: (request: PeerBidiStream) => Promise<void>;
		abort: RequestAbort;
	}[])(
		"rejects subscribe() with a RequestAbortedError after $name",
		async ({ act, abort }) => {
			const { peer, connection } = await connected();
			const subscribing = connection.subscribe(TRACK);
			await act(await peer.clientBidi());
			const error = await rejection(subscribing);
			expect(error).toBeInstanceOf(RequestAbortedError);
			expect(error).toMatchObject({ abort });
		},
	);

	test("publishes a namespace and a track, writes an Object and ends with PUBLISH_DONE", async () => {
		const { peer, connection } = await connected();
		await peer.clientUni();

		const announcing = connection.publishNamespace(TRACK.trackNamespace);
		const namespaceRequest = await peer.clientBidi();
		expect(await sentHex(namespaceRequest.sent)).toBe(
			"06 00 0f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 00",
		);
		await namespaceRequest.reply.write(hex("07 00 01 00"));
		await announcing;

		const publishing = connection.publish({ ...TRACK, trackAlias: 1n });
		const publishRequest = await peer.clientBidi();
		expect(await sentHex(publishRequest.sent)).toBe(
			"1d 00 17 02 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 01 00",
		);
		await publishRequest.reply.write(hex("07 00 05 02 10 01 12 01"));
		const track = await publishing;
		expect(track.parameters).toEqual({
			forward: true,
			groupOrder: "Ascending",
		});

		const subgroup = await track.openSubgroup({
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: 0,
			properties: false,
			endOfGroup: true,
		});
		await subgroup.write({ objectId: 0n, payload: utf8("a") });
		await subgroup.close();
		const stream = await peer.clientUni();
		expect(await sentHex(stream)).toBe("5c 01 00 00 00");
		expect(await sentHex(stream)).toBe("00 01 61");
		expect(await stream.read()).toEqual(END_OF_STREAM);

		const done = track.done(2n, "");
		expect(await sentHex(publishRequest.sent)).toBe("0b 00 03 02 01 00");
		expect(await publishRequest.sent.read()).toEqual(END_OF_STREAM);
		expect(await track.ended).toEqual({ type: "done" });
		await publishRequest.reply.close();
		expect(await done).toEqual({ type: "fin" });
	});

	// After PUBLISH_DONE, `done()` waits for the subscriber to end its
	// direction of the request stream and says what it saw (the row "A
	// publisher that ended its PUBLISH with PUBLISH_DONE" of the notes).
	test.each([
		{
			name: "a FIN",
			act: (request: PeerBidiStream) => request.reply.close(),
			end: { type: "fin" },
		},
		{
			name: "a reset",
			act: (request: PeerBidiStream) =>
				request.reply.abort(fakeStreamError(1, "reset")),
			end: { type: "reset", code: 1 },
		},
		{
			// What the subscriber still says on the stream changes nothing
			// (§5.1): a REQUEST_UPDATE that crossed PUBLISH_DONE, then the FIN.
			name: "a message and then a FIN",
			act: async (request: PeerBidiStream) => {
				await request.reply.write(hex("02 00 04 01 01 10 00"));
				await request.reply.close();
			},
			end: { type: "fin" },
		},
	])(
		"done() resolves with how the subscriber ended its direction: $name",
		async ({ act, end }) => {
			const { request, track } = await published();
			let settled = false;
			const done = track.done(2n, "").then((value) => {
				settled = true;
				return value;
			});
			await request.sent.read();
			await settle();
			// PUBLISH_DONE is out, and the wait goes on.
			expect(await track.ended).toEqual({ type: "done" });
			expect(settled).toBe(false);
			await act(request);
			expect(await done).toEqual(end);
		},
	);

	test("done() does not wait for a subscriber that had finished its direction", async () => {
		// WHY a wait no test could sit through: only the FIN seen before
		// PUBLISH_DONE can settle this.
		const { request, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		await request.reply.close();
		await settle();
		expect(await track.done(2n, "")).toEqual({ type: "fin" });
	});

	// WHY 0 and not a short wait: the test waits for no clock. The path is
	// the one every wait ends on.
	test("done() gives up when its wait is over and stops the subscriber's direction", async () => {
		const { request, track } = await published({ peerStreamEndTimeoutMs: 0 });
		expect(await track.done(2n, "")).toEqual({ type: "nothing-in-time" });
		// STOP_SENDING with CANCELLED reached the subscriber.
		expect(await rejection(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
	});

	// WHY STOP_SENDING does not end the wait: this endpoint has finished its
	// direction, so there is nothing left to stop, and the Rust client goes on
	// waiting as well.
	test("done() goes on waiting when the subscriber sends STOP_SENDING", async () => {
		const { request, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		let settled = false;
		const done = track.done(2n, "").then((value) => {
			settled = true;
			return value;
		});
		await request.sent.read();
		await request.sent.cancel(fakeStreamError(7, "stop"));
		await settle();
		expect(settled).toBe(false);
		await request.reply.close();
		expect(await done).toEqual({ type: "fin" });
	});

	// The session sends PUBLISH_DONE UPDATE_FAILED itself (§10.9.1). The
	// subscriber's direction is not left open after it either, as in the
	// Rust client.
	test("after a refused REQUEST_UPDATE, the subscriber's direction is stopped when the wait is over", async () => {
		const { request, track } = await published({ peerStreamEndTimeoutMs: 0 });
		await request.reply.write(hex("02 00 04 01 01 10 00"));
		expect(await track.ended).toEqual({ type: "update-failed" });
		expect(await rejection(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
	});

	// The session refuses a second PUBLISH_DONE. It must not take the wait of
	// the first call away, whether it comes at once or later.
	test("a second done() rejects with PublishEndedError and leaves the first one waiting", async () => {
		const { request, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		const first = track.done(2n, "");
		const second = rejection(track.done(2n, ""));
		expect(await second).toBeInstanceOf(PublishEndedError);
		expect(await second).toMatchObject({ end: { type: "done" } });
		await request.reply.close();
		expect(await first).toEqual({ type: "fin" });
		expect(await rejection(track.done(2n, ""))).toBeInstanceOf(
			PublishEndedError,
		);
	});

	test("done() resolves when the transport fails while it waits", async () => {
		const { peer, request, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		const done = track.done(2n, "");
		await request.sent.read();
		peer.fail(new Error("connection lost"));
		expect(await done).toMatchObject({
			type: "session-closed",
			close: { type: "error" },
		});
	});

	test("done() rejects when PUBLISH_DONE cannot be written, and the session's end finds no wait left", async () => {
		const { peer, connection, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		// The streams fail now; the session learns of it a turn later.
		peer.fail(new Error("connection lost"));
		expect(await rejection(track.done(2n, ""))).toBeInstanceOf(Error);
		expect(await connection.closed).toMatchObject({ type: "error" });
	});

	test("done() resolves when the session ends while it waits", async () => {
		const { connection, request, track } = await published({
			peerStreamEndTimeoutMs: 3_600_000,
		});
		const done = track.done(2n, "");
		await request.sent.read();
		connection.close(0, "bye");
		expect(await done).toEqual({
			type: "session-closed",
			close: { type: "local", code: 0, reason: "bye" },
		});
	});

	// A timer fires at once for NaN, a negative delay and a delay above
	// 2^31 - 1, so a wait that was asked for would not happen.
	test.each([
		{
			options: { peerStreamEndTimeoutMs: 2147483648 },
			message:
				"peerStreamEndTimeoutMs is a whole number from 0 to 2147483647, not 2147483648",
		},
		{
			options: { peerStreamEndTimeoutMs: -1 },
			message:
				"peerStreamEndTimeoutMs is a whole number from 0 to 2147483647, not -1",
		},
		{
			options: { peerStreamEndTimeoutMs: 2.5 },
			message:
				"peerStreamEndTimeoutMs is a whole number from 0 to 2147483647, not 2.5",
		},
		{
			options: { peerStreamEndTimeoutMs: Number.NaN },
			message:
				"peerStreamEndTimeoutMs is a whole number from 0 to 2147483647, not NaN",
		},
		{
			options: { peerStreamEndTimeoutMs: Number.POSITIVE_INFINITY },
			message:
				"peerStreamEndTimeoutMs is a whole number from 0 to 2147483647, not Infinity",
		},
		{
			options: { firstBytesTimeoutMs: 2147483648 },
			message:
				"firstBytesTimeoutMs is a whole number from 0 to 2147483647, not 2147483648",
		},
		{
			options: { firstBytesTimeoutMs: -1 },
			message:
				"firstBytesTimeoutMs is a whole number from 0 to 2147483647, not -1",
		},
		{
			options: { firstBytesTimeoutMs: 2.5 },
			message:
				"firstBytesTimeoutMs is a whole number from 0 to 2147483647, not 2.5",
		},
		{
			options: { firstBytesTimeoutMs: Number.NaN },
			message:
				"firstBytesTimeoutMs is a whole number from 0 to 2147483647, not NaN",
		},
		{
			options: { publishDoneTimeoutMs: 2147483648 },
			message:
				"publishDoneTimeoutMs is a whole number from 0 to 2147483647, not 2147483648",
		},
		{
			options: { publishDoneTimeoutMs: -1 },
			message:
				"publishDoneTimeoutMs is a whole number from 0 to 2147483647, not -1",
		},
		{
			options: { publishDoneTimeoutMs: Number.NaN },
			message:
				"publishDoneTimeoutMs is a whole number from 0 to 2147483647, not NaN",
		},
		{
			options: { setupTimeoutMs: 2147483648 },
			message:
				"setupTimeoutMs is a whole number from 0 to 2147483647, not 2147483648",
		},
		{
			options: { setupTimeoutMs: -1 },
			message: "setupTimeoutMs is a whole number from 0 to 2147483647, not -1",
		},
		{
			options: { setupTimeoutMs: Number.NaN },
			message: "setupTimeoutMs is a whole number from 0 to 2147483647, not NaN",
		},
	])(
		"connectOver rejects with a RangeError: $message",
		async ({ options, message }) => {
			const error = await rejection(
				connectOver(createFakePeer().transport, options),
			);
			expect(error).toBeInstanceOf(RangeError);
			expect(error).toMatchObject({ message });
		},
	);

	test("connectOver takes the longest delay a timer has for its waits", async () => {
		const { connection } = await connected({
			peerStreamEndTimeoutMs: 2147483647,
			publishDoneTimeoutMs: 2147483647,
			setupTimeoutMs: 2147483647,
		});
		connection.close();
		expect(await connection.closed).toMatchObject({ type: "local" });
	});

	// How the publisher ends a subscription, what `ended` says and whether
	// `objects` closes or fails (the table in the doc of `Subscription.objects`).
	test.each([
		{
			name: "PUBLISH_DONE whose Stream Count is already reached",
			// TRACK_ENDED, Stream Count 0, no reason.
			act: (request) => request.reply.write(hex("0b 00 03 02 00 00")),
			end: { type: "publish-done", statusCode: 2n, errorReason: "" },
			objects: "closes",
		},
		{
			name: "moqtail's PUBLISH_DONE, once the wait for its one stream runs out",
			// moqtail: TRACK_ENDED, Stream Count 1, "Publisher disconnected".
			act: (request) =>
				request.reply.write(
					hex(
						"0b 00 19 02 01 16 50 75 62 6c 69 73 68 65 72 20 64 69 73 63 6f 6e 6e 65 63 74 65 64",
					),
				),
			end: {
				type: "publish-done",
				statusCode: 2n,
				errorReason: "Publisher disconnected",
			},
			objects: "closes",
		},
		{
			name: "a FIN without PUBLISH_DONE",
			act: (request) => request.reply.close(),
			end: { type: "peer-closed" },
			objects: "fails",
		},
		{
			name: "a reset with code 5",
			act: (request) => request.reply.abort(fakeStreamError(5, "reset")),
			end: { type: "peer-reset", code: 5 },
			objects: "fails",
		},
	] satisfies {
		name: string;
		act: (request: PeerBidiStream) => Promise<void>;
		end: SubscriptionEnd;
		objects: "closes" | "fails";
	}[])(
		"tells a subscriber how its subscription ended: $name",
		async ({ act, end, objects }) => {
			const { request, subscription } = await subscribed({
				publishDoneTimeoutMs: 5,
			});
			await act(request);
			expect(await subscription.ended).toEqual(end);

			const reading = subscription.objects.getReader().read();
			if (objects === "closes") {
				expect(await reading).toEqual(END_OF_STREAM);
			} else {
				const error = await rejection(reading);
				expect(error).toBeInstanceOf(SubscriptionEndedError);
				expect(error).toMatchObject({ end });
			}
		},
	);

	// Two Objects are queued and unread when the subscription ends. `left` is
	// how many of them the reader still gets before `objects` closes or fails.
	test.each([
		{
			name: "PUBLISH_DONE",
			act: ({ request }) => request.reply.write(hex("0b 00 03 02 01 00")),
			left: 2,
			outcome: "closes",
		},
		{
			name: "a reset of the request stream",
			act: ({ request }) => request.reply.abort(fakeStreamError(5, "reset")),
			left: 2,
			outcome: "fails",
		},
		{
			name: "a FIN without PUBLISH_DONE",
			act: ({ request }) => request.reply.close(),
			left: 2,
			outcome: "fails",
		},
		{
			name: "the peer closing the session",
			act: async ({ peer }) => peer.close({ closeCode: 0, reason: "bye" }),
			left: 2,
			outcome: "fails",
		},
		{
			name: "cancel()",
			act: ({ subscription }) => subscription.cancel(),
			left: 0,
			outcome: "closes",
		},
		{
			name: "this endpoint closing the session",
			act: async ({ connection }) => connection.close(),
			left: 0,
			outcome: "closes",
		},
	] satisfies {
		name: string;
		act: (context: Awaited<ReturnType<typeof subscribed>>) => Promise<void>;
		left: number;
		outcome: "closes" | "fails";
	}[])(
		"Objects unread at the end of a subscription, ended by $name: $left delivered, then objects $outcome",
		async ({ act, left, outcome }) => {
			// The subgroup stream stays open, so PUBLISH_DONE ends the subscription
			// when the wait for it runs out.
			const context = await subscribed({ publishDoneTimeoutMs: 5 });
			const stream = context.peer.openUni();
			await stream.write(subgroupHeader(0));
			await stream.write(objectWith(0x61));
			await stream.write(objectWith(0x62));
			await settle();

			await act(context);
			await context.subscription.ended;
			const reader = context.subscription.objects.getReader();
			for (let read = 0; read < left; read++) {
				expect((await reader.read()).done).toBe(false);
			}
			if (outcome === "closes") {
				expect(await reader.read()).toEqual(END_OF_STREAM);
			} else {
				expect(await rejection(reader.read())).toBeInstanceOf(
					SubscriptionEndedError,
				);
			}
		},
	);

	test("a malformed Track delivers the Objects decoded before the fault, then fails", async () => {
		const { peer, subscription } = await subscribed();
		const stream = peer.openUni();
		// One good Object, then one with the Mandatory Track Property 0x4000.
		await stream.write(hex("15 09 00 00 00 00 00 01 61"));
		await stream.write(hex("00 04 c0 40 00 01 01 62"));
		expect(await subscription.ended).toEqual({ type: "malformed-track" });

		const reader = subscription.objects.getReader();
		expect((await reader.read()).value?.object.payload).toEqual(utf8("a"));
		expect(await rejection(reader.read())).toBeInstanceOf(
			SubscriptionEndedError,
		);
	});

	// How the subscriber ends a subscription made with PUBLISH.
	test.each([
		{
			name: "a reset with code 1",
			act: (request) => request.reply.abort(fakeStreamError(1, "reset")),
			end: { type: "peer-reset", code: 1 },
		},
		{
			name: "STOP_SENDING with code 7 and nothing else (§5.1)",
			act: (request) => request.sent.cancel(fakeStreamError(7, "stop")),
			end: { type: "peer-reset", code: 7 },
		},
		{
			name: "a REQUEST_UPDATE, which is refused (§10.9.1)",
			act: (request) => request.reply.write(hex("02 00 04 01 01 10 00")),
			end: { type: "update-failed" },
		},
	] satisfies {
		name: string;
		act: (request: PeerBidiStream) => Promise<void>;
		end: PublishEnd;
	}[])(
		"tells a publisher how its subscription ended: $name",
		async ({ act, end }) => {
			const { request, track } = await published();
			await act(request);
			expect(await track.ended).toEqual(end);
		},
	);

	test("a FIN of the subscriber alone does not end a subscription made with PUBLISH (§5.1)", async () => {
		const { request, track } = await published();
		await request.reply.close();
		// The Track goes on: a subgroup can be opened and written.
		const subgroup = await track.openSubgroup(SUBGROUP);
		await subgroup.write({ objectId: 0n, payload: utf8("a") });
		// STOP_SENDING after the FIN still ends it.
		await request.sent.cancel(fakeStreamError(7, "stop"));
		expect(await track.ended).toEqual({ type: "peer-reset", code: 7 });
	});

	test("after its Track ended, a SubgroupWriter fails writes with PublishEndedError and lets close and reset pass", async () => {
		const { request, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		await request.reply.abort(fakeStreamError(1, "reset"));
		const end = await track.ended;

		for (const call of [
			subgroup.write({ objectId: 0n, payload: utf8("a") }),
			track.openSubgroup(SUBGROUP),
			track.done(2n, ""),
		]) {
			const error = await rejection(call);
			expect(error).toBeInstanceOf(PublishEndedError);
			expect(error).toMatchObject({ end });
		}
		await subgroup.close();
		await subgroup.reset();
	});

	// The code goes to the peer in a `WebTransportError`, which Chromium 156
	// clamps to 255: a code outside 0 to 255 is refused before the stream is
	// touched, and the stream can still be written and reset.
	test.each([
		{
			code: 256,
			message: "a stream is reset with a whole number from 0 to 255, not 256",
		},
		{
			code: -1,
			message: "a stream is reset with a whole number from 0 to 255, not -1",
		},
		{
			code: 1.5,
			message: "a stream is reset with a whole number from 0 to 255, not 1.5",
		},
		{
			code: Number.NaN,
			message: "a stream is reset with a whole number from 0 to 255, not NaN",
		},
	])(
		"reset($code) rejects with a RangeError and leaves the stream open",
		async ({ code, message }) => {
			const { peer, track } = await published();
			const subgroup = await track.openSubgroup(SUBGROUP);
			const stream = await peer.clientUni();
			await stream.read();

			const error = await rejection(subgroup.reset(code));
			expect(error).toBeInstanceOf(RangeError);
			expect(error).toMatchObject({ message });

			await subgroup.write({ objectId: 0n, payload: utf8("a") });
			expect((await stream.read()).done).toBe(false);
			await subgroup.reset(255);
			await expect(stream.read()).rejects.toMatchObject({
				streamErrorCode: 255,
			});
		},
	);

	test("a write that races the end of its Track fails with PublishEndedError too", async () => {
		const { request, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		// The reset is on its way while the application still writes.
		void request.reply.abort(fakeStreamError(1, "reset"));
		await track.ended;
		const error = await rejection(
			subgroup.write({ objectId: 0n, payload: utf8("a") }),
		);
		expect(error).toBeInstanceOf(PublishEndedError);
	});

	test("a closed subgroup stream of a live Track fails writes with SubgroupClosedError", async () => {
		const { track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		await subgroup.close();

		expect(
			await rejection(subgroup.write({ objectId: 0n, payload: utf8("a") })),
		).toBeInstanceOf(SubgroupClosedError);
		// Closing and resetting again does nothing.
		await subgroup.close();
		await subgroup.reset();
	});

	test("resets a subgroup stream the peer stopped, and later writes fail with SubgroupClosedError (§11.4.3)", async () => {
		const { peer, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		const stream = await peer.clientUni();
		await stream.read();

		await stream.cancel(fakeStreamError(5, "stop"));
		await settle();
		expect(
			await rejection(subgroup.write({ objectId: 0n, payload: utf8("a") })),
		).toBeInstanceOf(SubgroupClosedError);
		await subgroup.reset();
	});

	test("a subscription that received PUBLISH_DONE still delivers a late Object after STOP_SENDING on its request stream (§10.11)", async () => {
		const { peer, request, subscription } = await subscribed();
		// Stream Count 1: one subgroup stream is still to come.
		await request.reply.write(hex("0b 00 03 02 01 00"));
		await request.sent.cancel(fakeStreamError(7, "stop"));
		await settle();

		const stream = peer.openUni();
		await stream.write(hex("14 09 00 00 00 00 01 61"));
		await stream.close();
		const reader = subscription.objects.getReader();
		expect((await reader.read()).value?.object.payload).toEqual(utf8("a"));
		expect(await reader.read()).toEqual(END_OF_STREAM);
		expect(await subscription.ended).toMatchObject({ type: "publish-done" });
	});

	test("subscribe() still gets the REQUEST_ERROR of a peer that stopped the request stream first", async () => {
		const { peer, connection } = await connected();
		const subscribing = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		await request.sent.read();
		await request.sent.cancel(fakeStreamError(7, "stop"));
		await settle();
		await request.reply.write(hex("05 00 05 10 00 02 6e 6f"));
		await request.reply.close();
		expect(await rejection(subscribing)).toBeInstanceOf(RequestFailedError);
	});

	test("the streams failing as the peer closes the session are not taken for STOP_SENDING or resets", async () => {
		const { peer, connection, track } = await published();
		await track.openSubgroup(SUBGROUP);
		// The fake fails every stream before `closed` settles.
		peer.close({ closeCode: 0, reason: "bye" });

		const close: SessionClose = { type: "peer", code: 0, reason: "bye" };
		expect(await connection.closed).toEqual(close);
		expect(await track.ended).toEqual({ type: "session-closed", close });
		// This endpoint did not close the session with an error of its own.
		expect(peer.closeInfo()).toBeUndefined();
	});

	test("cancel() resets the request stream and ends the subscription as cancelled", async () => {
		const { request, subscription } = await subscribed();
		await subscription.cancel();
		await expect(request.sent.read()).rejects.toMatchObject({
			streamErrorCode: 1,
		});
		expect(await subscription.ended).toEqual({ type: "cancelled" });
		expect(await subscription.objects.getReader().read()).toEqual(
			END_OF_STREAM,
		);
	});

	test("fails the Objects stream of a malformed Track and resets with MALFORMED_TRACK (§2.4.2)", async () => {
		const { peer, request, subscription } = await subscribed();
		// An Object with the Mandatory Track Property 0x4000.
		await peer.openUni().write(hex("15 09 00 00 00 00 04 c0 40 00 01 01 61"));

		const end: SubscriptionEnd = { type: "malformed-track" };
		expect(await subscription.ended).toEqual(end);
		const error = await rejection(subscription.objects.getReader().read());
		expect(error).toBeInstanceOf(SubscriptionEndedError);
		expect(error).toMatchObject({ end });
		await expect(request.sent.read()).rejects.toMatchObject({
			streamErrorCode: 18,
		});
	});

	test("answers a request from the peer with REQUEST_ERROR NOT_SUPPORTED and stays open", async () => {
		const { peer } = await connected();
		const request = peer.openBidi();
		// A Standalone FETCH for ["a"] "b" from {1, 0} to {3, 0}, with Request
		// ID 1, whole: a FETCH is decoded before it is refused.
		await request.reply.write(
			hex("16 00 0c 01 01 01 01 61 01 62 01 00 03 00 00"),
		);
		expect(decodeControlMessage(await sentBytes(request.sent))).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode: 3n, retryInterval: 0n },
		});
		expect(peer.closeInfo()).toBeUndefined();
	});

	// `code` is the Session termination error code of §3.5.
	test.each([
		{
			name: "a rule of the draft is broken (an unknown stream type)",
			bytes: "06",
			code: 3,
		},
		{
			name: "a fetch stream comes for no FETCH of this endpoint",
			bytes: "05 00",
			code: 3,
		},
	])(
		"closes the transport and fails what waits when $name",
		async ({ bytes, code }) => {
			const { peer, connection } = await connected();
			const subscribing = connection.subscribe(TRACK);
			await peer.clientBidi();

			await peer.openUni().write(hex(bytes));
			const close = await connection.closed;
			expect(close).toMatchObject({ type: "error", error: { code } });
			expect(peer.closeInfo()).toMatchObject({ closeCode: code });
			const error = await rejection(subscribing);
			expect(error).toBeInstanceOf(SessionClosedError);
			expect(error).toMatchObject({ close });
		},
	);

	// The datagrams before the last are dropped, and reading goes on after
	// them; the last closes the session with PROTOCOL_VIOLATION (§11, §11.3.1).
	test.each([
		{
			name: "an unknown datagram type",
			datagrams: ["10"],
			reason: "unknown datagram type 0x10",
		},
		{
			name: "a Type with STATUS and END_OF_GROUP",
			datagrams: ["22"],
			reason: "datagram type 0x22 has both STATUS and END_OF_GROUP",
		},
		{ name: "an empty datagram", datagrams: [""], reason: "empty datagram" },
		{
			name: "an unknown type after an OBJECT_DATAGRAM and a PADDING DATAGRAM",
			datagrams: ["00 09 00 61", "f0 13 2b 3e 29 00", "40"],
			reason: "unknown datagram type 0x40",
		},
	])("closes the session for $name", async ({ datagrams, reason }) => {
		const { peer, connection } = await connected();
		for (const datagram of datagrams) {
			await settle();
			expect(peer.closeInfo()).toBeUndefined();
			peer.sendDatagram(hex(datagram));
		}
		expect(await connection.closed).toMatchObject({
			type: "error",
			error: { code: 3 },
		});
		expect(peer.closeInfo()).toEqual({ closeCode: 3, reason });
	});

	test("ends the session with INTERNAL_ERROR when a diagnostics hook throws", async () => {
		const { peer, connection, request } = await subscribed({
			diagnostics: {
				onEvent(event) {
					if (event.type === "publish-done") throw new Error("hook failed");
				},
			},
		});
		await request.reply.write(hex("0b 00 03 02 01 00"));
		expect(await connection.closed).toMatchObject({
			type: "error",
			error: { message: "hook failed" },
		});
		expect(peer.closeInfo()).toEqual({ closeCode: 1, reason: "hook failed" });
	});

	test("a session closed by this endpoint ends its subscriptions without an error", async () => {
		const { connection, subscription } = await subscribed();
		connection.close(0, "done");

		const close: SessionClose = { type: "local", code: 0, reason: "done" };
		expect(await connection.closed).toEqual(close);
		expect(await subscription.ended).toEqual({ type: "session-closed", close });
		expect(await subscription.objects.getReader().read()).toEqual(
			END_OF_STREAM,
		);
		// Cancelling after the end does nothing, as in the session.
		await subscription.cancel();
	});

	test("a session closed by the peer fails the Objects stream and reports the peer's code", async () => {
		const { peer, connection, subscription } = await subscribed();
		peer.close({ closeCode: 3, reason: "violation" });

		const close: SessionClose = { type: "peer", code: 3, reason: "violation" };
		const end: SubscriptionEnd = { type: "session-closed", close };
		expect(await connection.closed).toEqual(close);
		expect(await subscription.ended).toEqual(end);
		const error = await rejection(subscription.objects.getReader().read());
		expect(error).toBeInstanceOf(SubscriptionEndedError);
		expect(error).toMatchObject({ end });
	});

	test("a transport that fails ends the session as an error, and a published Track with it", async () => {
		const { peer, connection, track } = await published();
		peer.fail(new Error("connection lost"));

		const close = await connection.closed;
		expect(close).toMatchObject({
			type: "error",
			error: {
				message: "the transport session failed: Error: connection lost",
			},
		});
		expect(await track.ended).toEqual({ type: "session-closed", close });
		expect(await rejection(track.openSubgroup(SUBGROUP))).toBeInstanceOf(
			PublishEndedError,
		);
	});

	/**
	 * 40 subgroup streams of one Object each, Groups 0 to 39, sent to a
	 * subscription that nobody reads, with a queue of 16 and 8 streams of the
	 * peer read at once. Resolves with the streams, the last of which is left
	 * open, and a count of the Objects the session has handed over.
	 */
	async function flooded(options: SessionConnectOptions = {}): Promise<{
		peer: FakePeer;
		connection: MoqtConnection;
		request: PeerBidiStream;
		subscription: Subscription;
		streams: WritableStreamDefaultWriter<Uint8Array>[];
		handedOver(): number;
	}> {
		let objects = 0;
		const context = await subscribed({
			subscriptionHighWaterMark: 16,
			maxPeerUniStreams: 8,
			diagnostics: {
				onEvent(event) {
					if (event.type === "object") objects += 1;
				},
			},
			...options,
		});
		const streams: WritableStreamDefaultWriter<Uint8Array>[] = [];
		for (let group = 0; group < 40; group++) {
			const stream = context.peer.openUni();
			streams.push(stream);
			await stream.write(subgroupHeader(group));
			await stream.write(objectWith(0x61));
			// Closing is not awaited: it settles when the stream has been read.
			if (group < 39) void stream.close().catch(() => {});
		}
		await settle();
		return { ...context, streams, handedOver: () => objects };
	}

	// The queue takes 15 Objects without making their streams wait. One of
	// the 8 streams read at once is the peer's control stream, so 7 more
	// streams each hand over their Object and wait: 22. The other 18 stay in
	// the transport. Without the limit on streams all 40 were handed over.
	test("a subscription nobody reads holds the Objects of the streams read at once, and no more", async () => {
		const { peer, subscription, handedOver } = await flooded();
		expect(handedOver()).toBe(22);
		// The other 18 streams were not taken from the transport.
		expect(peer.untakenUniStreams()).toBe(18);

		// Reading lets the rest in: nothing was dropped.
		const reader = subscription.objects.getReader();
		const groups: bigint[] = [];
		for (let read = 0; read < 40; read++) {
			const next = await reader.read();
			if (next.value !== undefined) groups.push(next.value.groupId);
		}
		expect(groups.sort((a, b) => Number(a - b))).toEqual(
			Array.from({ length: 40 }, (_, group) => BigInt(group)),
		);
		expect(handedOver()).toBe(40);
		expect(peer.untakenUniStreams()).toBe(0);
	});

	// The last stream is one the connection had not taken from the transport
	// when the subscription ended. It has to be taken then and stopped, so
	// that neither it nor a stream that waited on the queue stays behind.
	test.each([
		{
			ending: "the application cancels",
			end: ({ subscription }: Awaited<ReturnType<typeof flooded>>) =>
				subscription.cancel(),
		},
		{
			ending: "PUBLISH_DONE arrives and the wait for late streams is over",
			end: ({ request }: Awaited<ReturnType<typeof flooded>>) =>
				request.reply.write(hex("0b 00 03 02 01 00")),
		},
	])(
		"streams that waited for a subscription nobody reads are stopped when $ending",
		async ({ end }) => {
			const context = await flooded({ publishDoneTimeoutMs: 5 });
			// Stream 39 is among the 18 that wait in the transport.
			expect(context.peer.untakenUniStreams()).toBe(18);
			await end(context);
			const waited = context.streams[39];
			if (waited === undefined) throw new Error("the last stream is missing");
			expect(await rejection(waited.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			await context.subscription.ended;
			expect(context.peer.untakenUniStreams()).toBe(0);
		},
	);

	test("closing the session stops taking the streams that waited", async () => {
		const { peer, connection } = await flooded();
		expect(peer.untakenUniStreams()).toBe(18);
		connection.close();
		await settle();
		expect(peer.closeInfo()).toEqual({ closeCode: 0, reason: "" });
		// The readers are forgotten at the close, which makes room; the loop
		// must not use it to take what waited.
		expect(peer.untakenUniStreams()).toBe(18);
	});

	// One place is the peer's control stream, so 1 would read no data stream
	// and 0 not even SETUP; NaN and Infinity would switch the limit off.
	test.each([
		{ value: 1, message: "maxPeerUniStreams is a whole number from 2, not 1" },
		{ value: 0, message: "maxPeerUniStreams is a whole number from 2, not 0" },
		{
			value: 2.5,
			message: "maxPeerUniStreams is a whole number from 2, not 2.5",
		},
		{
			value: Number.NaN,
			message: "maxPeerUniStreams is a whole number from 2, not NaN",
		},
		{
			value: Number.POSITIVE_INFINITY,
			message: "maxPeerUniStreams is a whole number from 2, not Infinity",
		},
	])(
		"connectOver rejects maxPeerUniStreams $value with a RangeError",
		async ({ value, message }) => {
			const error = await rejection(
				connectOver(createFakePeer().transport, { maxPeerUniStreams: value }),
			);
			expect(error).toBeInstanceOf(RangeError);
			expect(error).toMatchObject({ message });
		},
	);

	test.each([
		{ value: 0, message: "maxPeerBidiStreams is a whole number from 1, not 0" },
		{
			value: -1,
			message: "maxPeerBidiStreams is a whole number from 1, not -1",
		},
		{
			value: 2.5,
			message: "maxPeerBidiStreams is a whole number from 1, not 2.5",
		},
		{
			value: Number.NaN,
			message: "maxPeerBidiStreams is a whole number from 1, not NaN",
		},
		{
			value: Number.POSITIVE_INFINITY,
			message: "maxPeerBidiStreams is a whole number from 1, not Infinity",
		},
	])(
		"connectOver rejects maxPeerBidiStreams $value with a RangeError",
		async ({ value, message }) => {
			const error = await rejection(
				connectOver(createFakePeer().transport, { maxPeerBidiStreams: value }),
			);
			expect(error).toBeInstanceOf(RangeError);
			expect(error).toMatchObject({ message });
		},
	);

	test("without the option, 64 requests of the peer are read at once", async () => {
		const { peer } = await connected();
		// 65 streams with the start of a request each, which holds its place.
		for (let count = 0; count < 65; count++) {
			void peer.openBidi().reply.write(hex("16"));
		}
		await settle();
		expect(peer.untakenBidiStreams()).toBe(1);
	});

	// A request of the peer is held until its first message is whole, and a
	// FETCH until all of it is there. The limit bounds how many streams can
	// hold one at a time; the others are not taken from the transport.
	test("requests of the peer are read as many at once as the limit, and the rest waits in the transport", async () => {
		const { peer } = await connected({ maxPeerBidiStreams: 2 });
		// Five streams, each with the start of a Relative Joining FETCH: Request
		// IDs 1, 3, 5, 7, 9. No request is whole, so none is answered.
		const requests = [1, 3, 5, 7, 9].map((requestId) => {
			const request = peer.openBidi();
			void request.reply.write(hex(`16 00 05 0${requestId} 02`));
			return request;
		});
		await settle();
		expect(peer.untakenBidiStreams()).toBe(3);

		// The first becomes whole: it is refused, and keeps its place while the
		// peer has not ended its direction.
		const [first, second] = requests;
		if (first === undefined || second === undefined) {
			throw new Error("five requests were opened");
		}
		await first.reply.write(hex("00 00 00"));
		// INVALID_JOINING_REQUEST_ID: no SUBSCRIBE of the peer is accepted.
		expect(decodeControlMessage(await sentBytes(first.sent))).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode: 0x32n },
		});
		await settle();
		expect(peer.untakenBidiStreams()).toBe(3);

		// The peer ends its direction of the first: the third is read.
		await first.reply.close();
		await settle();
		expect(peer.untakenBidiStreams()).toBe(2);

		// A stream the peer resets gives up its place as well.
		await second.reply.abort(new Error("reset by the test"));
		await settle();
		expect(peer.untakenBidiStreams()).toBe(1);
		expect(peer.closeInfo()).toBeUndefined();
	});

	// A chunk can carry several Objects, and a stream hands all of them over
	// before it waits: the limit is on streams, not on Objects.
	test("a stream hands over every Object of a chunk before it waits for the queue", async () => {
		let objects = 0;
		const { peer } = await subscribed({
			subscriptionHighWaterMark: 2,
			maxPeerUniStreams: 2,
			diagnostics: {
				onEvent(event) {
					if (event.type === "object") objects += 1;
				},
			},
		});
		const stream = peer.openUni();
		await stream.write(
			Uint8Array.of(
				...subgroupHeader(0),
				...objectWith(0x61),
				...objectWith(0x62),
				...objectWith(0x63),
				...objectWith(0x64),
				...objectWith(0x65),
			),
		);
		await settle();
		expect(objects).toBe(5);
	});
});

test("a NEW_GROUP_REQUEST of the subscriber reaches the handler of the published track, the one before a handler was named too", async () => {
	const { request, track } = await published();
	// REQUEST_UPDATE with Request ID 1 and NEW_GROUP_REQUEST (0x32) = 7.
	await request.reply.write(hex("02 00 04 01 01 32 07"));
	expect(await sentHex(request.sent)).toBe("07 00 01 00");
	// The event follows the write of the answer.
	const afterTheWrite = () => new Promise((resolve) => setTimeout(resolve, 0));
	await afterTheWrite();
	const seen: bigint[] = [];
	track.onNewGroupRequest((group) => seen.push(group));
	expect(seen).toEqual([7n]);
	// Request ID 3, and 9.
	await request.reply.write(hex("02 00 04 03 01 32 09"));
	expect(await sentHex(request.sent)).toBe("07 00 01 00");
	await afterTheWrite();
	expect(seen).toEqual([7n, 9n]);
});
