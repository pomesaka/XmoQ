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
	FakeStreamError,
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
async function published(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	request: PeerBidiStream;
	track: PublishedTrack;
}> {
	const { peer, connection } = await connected();
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
			act: (request) => request.reply.abort(new FakeStreamError(5, "reset")),
			abort: { type: "peer-reset", code: 5 },
		},
		{
			name: "a SUBSCRIBE_OK with an unknown Mandatory Track Property (§2.5.1)",
			act: (request) => request.reply.write(hex("04 00 06 09 00 c0 40 00 01")),
			abort: { type: "unsupported-track-property" },
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

		await track.done(2n, "");
		expect(await sentHex(publishRequest.sent)).toBe("0b 00 03 02 01 00");
		expect(await publishRequest.sent.read()).toEqual(END_OF_STREAM);
		expect(await track.ended).toEqual({ type: "done" });
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
			act: (request) => request.reply.abort(new FakeStreamError(5, "reset")),
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
			act: ({ request }) =>
				request.reply.abort(new FakeStreamError(5, "reset")),
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
			name: "a FIN",
			act: (request) => request.reply.close(),
			end: { type: "peer-closed" },
		},
		{
			name: "a reset with code 1",
			act: (request) => request.reply.abort(new FakeStreamError(1, "reset")),
			end: { type: "peer-reset", code: 1 },
		},
		{
			name: "STOP_SENDING with code 7 and nothing else (§5.1)",
			act: (request) => request.sent.cancel(new FakeStreamError(7, "stop")),
			end: { type: "peer-reset", code: 7 },
		},
		{
			name: "a REQUEST_UPDATE, which is refused (§10.9.1)",
			act: (request) => request.reply.write(hex("02 00 02 01 00")),
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

	test("after its Track ended, a SubgroupWriter fails writes with PublishEndedError and lets close and reset pass", async () => {
		const { request, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		await request.reply.abort(new FakeStreamError(1, "reset"));
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

	test("a write that races the end of its Track fails with PublishEndedError too", async () => {
		const { request, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		// The reset is on its way while the application still writes.
		void request.reply.abort(new FakeStreamError(1, "reset"));
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

		await stream.cancel(new FakeStreamError(5, "stop"));
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
		await request.sent.cancel(new FakeStreamError(7, "stop"));
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
		await request.sent.cancel(new FakeStreamError(7, "stop"));
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
		await expect(request.sent.read()).rejects.toThrow("stream error 1");
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
		await expect(request.sent.read()).rejects.toThrow("stream error 18");
	});

	test("answers a request from the peer with REQUEST_ERROR NOT_SUPPORTED and stays open", async () => {
		const { peer } = await connected();
		const request = peer.openBidi();
		// The start of a FETCH with Request ID 1.
		await request.reply.write(hex("16 00 02 01 00"));
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
			name: "the peer uses a stream type that is not implemented (FETCH_HEADER)",
			bytes: "05 00",
			code: 1,
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
});
