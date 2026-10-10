import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import {
	connectOver,
	type MoqtConnection,
	type PeerSubscribe,
	type PublishedTrack,
} from "../../src/webtransport/connection.ts";
import {
	PublishEndedError,
	RequestAbortedError,
	SessionClosedError,
} from "../../src/webtransport/endings.ts";
import { fixture, hex, MOQTAIL_RELAY_SETUP, toHex, utf8 } from "../helpers.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStopWithoutCode,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

/** The captured SUBSCRIBE with a Request ID of the server. */
function subscribeBytes(requestId: number): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	return hex(pairs.join(" "));
}

/** REQUEST_UPDATE with Request ID 3 and no parameters. */
const REQUEST_UPDATE = hex("02 00 02 03 00");

const SUBGROUP = {
	groupId: 0n,
	subgroupId: { mode: "Zero" },
	properties: false,
	endOfGroup: true,
} as const;

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

async function sentHex(
	reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<string> {
	const { value } = await reader.read();
	if (value === undefined) throw new Error("the stream ended");
	return toHex(value);
}

/** The message the client sent next on the stream, in a line. */
async function sentMessage(
	reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<string> {
	const decoded = decodeControlMessage(hex(await sentHex(reader)));
	if (decoded.status !== "ok") throw new Error("not one control message");
	const { value } = decoded;
	switch (value.type) {
		case "REQUEST_ERROR":
			return `REQUEST_ERROR ${value.errorCode} "${value.errorReason}"`;
		case "SUBSCRIBE_OK":
			return `SUBSCRIBE_OK alias ${value.trackAlias}`;
		case "PUBLISH_DONE":
			return `PUBLISH_DONE ${value.statusCode} streams ${value.streamCount}`;
		default:
			return value.type;
	}
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

async function connected(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
}> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport, {
		peerStreamEndTimeoutMs: 20,
	});
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

/** A connection that keeps what it is asked for, and whose peer sent the SUBSCRIBE with Request ID 1. */
async function asked(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	stream: PeerBidiStream;
	request: PeerSubscribe;
}> {
	const { peer, connection } = await connected();
	const requests: PeerSubscribe[] = [];
	connection.serve({ subscribe: (request) => requests.push(request) });
	const stream = peer.openBidi();
	await stream.reply.write(subscribeBytes(1));
	await settle();
	const [request] = requests;
	if (request === undefined) throw new Error("the handler was not called");
	return { peer, connection, stream, request };
}

/** That, with the SUBSCRIBE accepted and the SUBSCRIBE_OK read by the peer. */
async function serving(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	stream: PeerBidiStream;
	request: PeerSubscribe;
	track: PublishedTrack;
}> {
	const all = await asked();
	const track = await all.request.accept();
	await all.stream.sent.read();
	return { ...all, track };
}

describe("serve: a SUBSCRIBE of the peer", () => {
	test("is refused with NOT_SUPPORTED before `serve` was called", async () => {
		const { peer } = await connected();
		const stream = peer.openBidi();
		await stream.reply.write(subscribeBytes(1));
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 3 "this endpoint serves no SUBSCRIBE"',
		);
		expect(await stream.sent.read()).toEqual({ done: true, value: undefined });
		expect(peer.closeInfo()).toBeUndefined();
	});

	test("is given to the handler with its track and its parameters", async () => {
		const { request } = await asked();
		expect({
			requestId: request.requestId,
			namespace: request.trackNamespace.map((field) =>
				new TextDecoder().decode(field),
			),
			name: new TextDecoder().decode(request.trackName),
			parameters: request.parameters,
			aborted: request.signal.aborted,
		}).toEqual({
			requestId: 1n,
			namespace: ["xmoq", "phase0"],
			name: "object",
			parameters: {
				forward: true,
				subscriberPriority: 128,
				subscriptionFilter: { filterType: "LargestObject" },
				groupOrder: "Ascending",
			},
			aborted: false,
		});
	});

	test("accepted, is a Track to publish on: SUBSCRIBE_OK with LARGEST_OBJECT, an Object, PUBLISH_DONE", async () => {
		const { peer, stream, request } = await asked();
		const track = await request.accept({
			parameters: { largestObject: { group: 3n, object: 2n } },
		});
		expect(await sentHex(stream.sent)).toBe(
			fixture("subscribe-ok-largest-object-moqtail-relay"),
		);
		expect({
			requestId: track.requestId,
			trackAlias: track.trackAlias,
			forward: track.parameters.forward,
		}).toEqual({ requestId: 1n, trackAlias: 0n, forward: true });

		const subgroup = await track.openSubgroup(SUBGROUP);
		await subgroup.write({ objectId: 0n, payload: utf8("a") });
		await subgroup.close();
		const data = await peer.clientUni();
		// SUBGROUP_HEADER for Track Alias 0, Group 0, and the Object.
		expect(await sentHex(data)).toBe("78 00 00");
		expect(await sentHex(data)).toBe("00 01 61");

		const done = track.done(2n, "");
		expect(await sentMessage(stream.sent)).toBe("PUBLISH_DONE 2 streams 1");
		expect(await stream.sent.read()).toEqual({ done: true, value: undefined });
		// The subscriber finishes its direction.
		await stream.reply.close();
		expect(await done).toEqual({ type: "fin" });
		expect(await track.ended).toEqual({ type: "done" });
	});

	test("after PUBLISH_DONE, a subscriber that does not end its direction is stopped", async () => {
		const { stream, track } = await serving();
		expect(await track.done(2n, "")).toEqual({ type: "nothing-in-time" });
		const error = await rejection(stream.reply.write(new Uint8Array(1)));
		expect(error).toMatchObject({ streamErrorCode: 1 });
	});

	test("rejected, gets REQUEST_ERROR with the code and the reason, and a FIN", async () => {
		const { stream, request } = await asked();
		await request.reject(0x10n, "no such track");
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 16 "no such track"',
		);
		expect(await stream.sent.read()).toEqual({ done: true, value: undefined });
		expect(request.signal.aborted).toBe(false);
	});

	test.each([
		{
			name: "accept",
			again: (request: PeerSubscribe) => request.accept(),
		},
		{
			name: "reject",
			again: (request: PeerSubscribe) => request.reject(3n),
		},
	])("answered, cannot be answered again: $name", async ({ again }) => {
		const { request } = await serving();
		const error = await rejection(again(request));
		expect(error).toMatchObject({
			message: "the request was answered already",
		});
	});

	test("accepted with a parameter that is none of SUBSCRIBE_OK, still waits", async () => {
		const { stream, request } = await asked();
		expect(
			await rejection(request.accept({ parameters: { forward: true } })),
		).toBeInstanceOf(RangeError);
		await request.accept();
		expect(await sentMessage(stream.sent)).toBe("SUBSCRIBE_OK alias 0");
	});

	// Each row: a handler that fails, and what the peer is sent.
	const failure = new Error("the application failed");
	const failingHandlers: {
		name: string;
		handler: (request: PeerSubscribe) => void | Promise<void>;
		sent: string;
	}[] = [
		{
			name: "throws",
			handler: () => {
				throw failure;
			},
			sent: 'REQUEST_ERROR 0 "the publisher failed to handle the request"',
		},
		{
			name: "is `async` and throws",
			handler: async () => {
				throw failure;
			},
			sent: 'REQUEST_ERROR 0 "the publisher failed to handle the request"',
		},
		{
			name: "is `async` and throws after it accepted",
			handler: async (request) => {
				await request.accept();
				throw failure;
			},
			sent: "SUBSCRIBE_OK alias 0",
		},
	];
	test.each(failingHandlers)(
		"whose handler $name: the error is reported, and the request has one answer",
		async ({ handler, sent }) => {
			const { peer, connection } = await connected();
			const reported: unknown[] = [];
			const original = globalThis.reportError;
			globalThis.reportError = (error: unknown) => {
				reported.push(error);
			};
			try {
				connection.serve({ subscribe: handler });
				const stream = peer.openBidi();
				await stream.reply.write(subscribeBytes(1));
				expect(await sentMessage(stream.sent)).toBe(sent);
				await settle();
				expect(reported).toEqual([failure]);
				expect(peer.closeInfo()).toBeUndefined();
			} finally {
				globalThis.reportError = original;
			}
		},
	);

	test("`serve` called again replaces the handler for the requests that come after", async () => {
		const { peer, connection, stream } = await asked();
		connection.serve({});
		const second = peer.openBidi();
		await second.reply.write(subscribeBytes(3));
		// The Track of the first request is still asked for: the session says
		// so itself, whoever serves.
		expect(await sentMessage(second.sent)).toBe(
			'REQUEST_ERROR 25 "this endpoint is already the publisher of a subscription to the Track"',
		);
		await stream.reply.abort(fakeStreamError(1, "reset"));
		await settle();
		const third = peer.openBidi();
		await third.reply.write(subscribeBytes(5));
		expect(await sentMessage(third.sent)).toBe(
			'REQUEST_ERROR 3 "this endpoint serves no SUBSCRIBE"',
		);
	});
});

// §10.9 lets a subscriber send a REQUEST_UPDATE on the stream of its
// SUBSCRIBE at any time, so the two can arrive in one read. The session has
// refused the update, and ended the request with that, before the connection
// hears of the SUBSCRIBE.
describe("serve: a SUBSCRIBE and a REQUEST_UPDATE that arrive in one read", () => {
	const rows: {
		name: string;
		serve: ((connection: MoqtConnection) => void) | undefined;
	}[] = [
		{ name: "`serve` was never called", serve: undefined },
		{
			name: "the handler rejects at once",
			serve: (connection) =>
				connection.serve({
					subscribe: (request) => {
						request.reject(0x10n).catch(() => {});
					},
				}),
		},
		{
			name: "the handler accepts at once",
			serve: (connection) =>
				connection.serve({
					subscribe: (request) => {
						request.accept().catch(() => {});
					},
				}),
		},
		{
			name: "the handler throws",
			serve: (connection) =>
				connection.serve({
					subscribe: () => {
						throw new Error("the application failed");
					},
				}),
		},
	];
	test.each(rows)(
		"$name: the update is refused, and the session stays open",
		async ({ serve }) => {
			const { peer, connection } = await connected();
			const original = globalThis.reportError;
			globalThis.reportError = () => {};
			try {
				serve?.(connection);
				const stream = peer.openBidi();
				await stream.reply.write(
					new Uint8Array([...subscribeBytes(1), ...REQUEST_UPDATE]),
				);
				expect(await sentMessage(stream.sent)).toBe(
					'REQUEST_ERROR 3 "REQUEST_UPDATE is not implemented"',
				);
				expect(await stream.sent.read()).toEqual({
					done: true,
					value: undefined,
				});
				await settle();
				expect(peer.closeInfo()).toBeUndefined();
				expect(
					await Promise.race([connection.closed, settle()]),
				).toBeUndefined();
			} finally {
				globalThis.reportError = original;
			}
		},
	);
});

describe("serve: a SUBSCRIBE that goes before it is answered", () => {
	// Each row: what the peer does, and what `signal` is aborted with.
	test.each([
		{
			name: "the peer resets its direction",
			act: ({ stream }: { stream: PeerBidiStream; peer: FakePeer }) =>
				stream.reply.abort(fakeStreamError(1, "reset")),
			reason: {
				name: "RequestAbortedError",
				abort: { type: "peer-reset", code: 1 },
			},
		},
		{
			name: "the peer stops the stream",
			act: ({ stream }: { stream: PeerBidiStream; peer: FakePeer }) =>
				stream.sent.cancel(fakeStreamError(7, "stop")),
			reason: {
				name: "RequestAbortedError",
				abort: { type: "peer-reset", code: 7 },
			},
		},
		{
			name: "the peer stops the stream and the browser passes no code",
			act: ({ stream }: { stream: PeerBidiStream; peer: FakePeer }) =>
				stream.sent.cancel(fakeStopWithoutCode()),
			reason: {
				name: "RequestAbortedError",
				abort: { type: "peer-reset", code: undefined },
			},
		},
		{
			name: "the peer sends a REQUEST_UPDATE",
			act: ({ stream }: { stream: PeerBidiStream; peer: FakePeer }) =>
				stream.reply.write(REQUEST_UPDATE),
			reason: { name: "RequestAbortedError", abort: { type: "update-failed" } },
		},
		{
			name: "the peer closes the session",
			act: ({ peer }: { stream: PeerBidiStream; peer: FakePeer }) =>
				Promise.resolve(peer.close({ closeCode: 0, reason: "bye" })),
			reason: { name: "SessionClosedError", close: { type: "peer", code: 0 } },
		},
	])(
		"$name: `signal` aborts, and an answer is rejected with its reason",
		async ({ act, reason }) => {
			const { peer, stream, request } = await asked();
			await act({ stream, peer });
			await settle();
			expect(request.signal.aborted).toBe(true);
			expect(request.signal.reason).toMatchObject(reason);
			expect(await rejection(request.accept())).toBe(request.signal.reason);
			expect(await rejection(request.reject(3n))).toBe(request.signal.reason);
		},
	);

	test("an answer given while the end of the request is still on its way is rejected with the reason of `signal`", async () => {
		const peer = createFakePeer();
		const requests: PeerSubscribe[] = [];
		let accepting: Promise<unknown> | undefined;
		const connecting = connectOver(peer.transport, {
			diagnostics: {
				// Called with the bytes of the REQUEST_UPDATE before the session
				// has them. The answer is given right after the session took
				// them, and before the event that ends the request is handled:
				// the microtask runs between the two.
				onBytes(direction, _stream, data) {
					if (direction !== "received" || data !== REQUEST_UPDATE) return;
					queueMicrotask(() => {
						accepting = requests[0]?.accept();
						// Observed at once; the test reads it below.
						accepting?.catch(() => {});
					});
				},
			},
		});
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		connection.serve({
			subscribe: (request) => {
				requests.push(request);
			},
		});
		const stream = peer.openBidi();
		await stream.reply.write(subscribeBytes(1));
		await settle();
		const [request] = requests;
		if (request === undefined) throw new Error("the handler was not called");
		expect(request.signal.aborted).toBe(false);
		await stream.reply.write(REQUEST_UPDATE);
		await settle();
		if (accepting === undefined) throw new Error("`accept` was not called");
		const error = await rejection(accepting);
		expect(error).toBeInstanceOf(RequestAbortedError);
		expect(error).toBe(request.signal.reason);
	});

	test("the classes of the reasons are the ones an application can tell apart", async () => {
		const reset = await asked();
		await reset.stream.reply.abort(fakeStreamError(1, "reset"));
		await settle();
		expect(reset.request.signal.reason).toBeInstanceOf(RequestAbortedError);
		const closed = await asked();
		closed.connection.close();
		expect(closed.request.signal.reason).toBeInstanceOf(SessionClosedError);
	});

	test("a REQUEST_UPDATE before the answer is refused on the stream, which is then finished", async () => {
		const { stream } = await asked();
		await stream.reply.write(REQUEST_UPDATE);
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 3 "REQUEST_UPDATE is not implemented"',
		);
		expect(await stream.sent.read()).toEqual({ done: true, value: undefined });
	});

	test("a subscriber that only finishes its direction is still answered", async () => {
		const { stream, request } = await asked();
		await stream.reply.close();
		await settle();
		expect(request.signal.aborted).toBe(false);
		await request.accept();
		expect(await sentMessage(stream.sent)).toBe("SUBSCRIBE_OK alias 0");
	});
});

describe("serve: an accepted SUBSCRIBE that the subscriber ends", () => {
	// Each row: what the subscriber does, and how the Track ends.
	test.each([
		{
			name: "it resets its direction",
			act: (stream: PeerBidiStream) =>
				stream.reply.abort(fakeStreamError(1, "reset")),
			end: { type: "peer-reset", code: 1 },
		},
		{
			name: "it stops the stream (§5.1)",
			act: (stream: PeerBidiStream) =>
				stream.sent.cancel(fakeStreamError(7, "stop")),
			end: { type: "peer-reset", code: 7 },
		},
		{
			name: "it sends a REQUEST_UPDATE (§10.9.1)",
			act: (stream: PeerBidiStream) => stream.reply.write(REQUEST_UPDATE),
			end: { type: "update-failed" },
		},
	])("$name: the Track ends, and its calls fail", async ({ act, end }) => {
		const { stream, track } = await serving();
		const subgroup = await track.openSubgroup(SUBGROUP);
		await act(stream);
		expect(await track.ended).toEqual(end);
		const error = await rejection(
			subgroup.write({ objectId: 0n, payload: utf8("a") }),
		);
		expect(error).toBeInstanceOf(PublishEndedError);
		expect(error).toMatchObject({ end });
		expect(await rejection(track.openSubgroup(SUBGROUP))).toBeInstanceOf(
			PublishEndedError,
		);
	});

	// A relay cancels by resetting its direction and stopping this
	// endpoint's. Chromium 156 gives the reset its code and the stop none
	// (docs/experiments/2026-10-11-write-abort-in-chromium.md), and the page
	// sees them in either order.
	test.each([
		{
			name: "the reset is seen a moment before the stop: its code",
			apart: true,
			end: { type: "peer-reset", code: 5 },
		},
		{
			name: "the two come at once, and the stop is seen first: no code",
			apart: false,
			end: { type: "peer-reset", code: undefined },
		},
	])(
		"it resets its direction and stops the stream; $name",
		async ({ apart, end }) => {
			const { peer, stream, track } = await serving();
			const reset = stream.reply.abort(fakeStreamError(5, "reset"));
			if (apart) await settle();
			// The one that is seen second finds a stream this endpoint has reset
			// by then, and may fail.
			await Promise.allSettled([
				reset,
				stream.sent.cancel(fakeStopWithoutCode()),
			]);
			expect(await track.ended).toEqual(end);
			await settle();
			expect(peer.closeInfo()).toBeUndefined();
		},
	);

	test("a subscriber that only finishes its direction leaves the Track as it is", async () => {
		const { peer, stream, track } = await serving();
		await stream.reply.close();
		await settle();
		const subgroup = await track.openSubgroup(SUBGROUP);
		await subgroup.close();
		expect(await sentHex(await peer.clientUni())).toBe("78 00 00");
		// The subscriber had finished before: `done()` has that at once.
		expect(await track.done(2n, "")).toEqual({ type: "fin" });
	});

	test("the end of the session ends the Track", async () => {
		const { connection, track } = await serving();
		connection.close(0, "bye");
		expect(await track.ended).toMatchObject({
			type: "session-closed",
			close: { type: "local", code: 0, reason: "bye" },
		});
	});

	test("the place of the stream among the peer's streams is free again when the peer has ended its direction", async () => {
		const peer = createFakePeer();
		const connecting = connectOver(peer.transport, { maxPeerBidiStreams: 1 });
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		const requests: PeerSubscribe[] = [];
		connection.serve({ subscribe: (request) => requests.push(request) });
		const first = peer.openBidi();
		await first.reply.write(subscribeBytes(1));
		await settle();
		await requests[0]?.accept();
		// A second request waits in the transport while the first stream is
		// read.
		const second = peer.openBidi();
		await second.reply.write(hex("0d 00 02 03 00"));
		await settle();
		expect(peer.untakenBidiStreams()).toBe(1);
		await first.reply.close();
		await settle();
		expect(peer.untakenBidiStreams()).toBe(0);
	});
});
