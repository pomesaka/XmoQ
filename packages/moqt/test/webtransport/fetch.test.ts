import { describe, expect, test } from "bun:test";
import {
	decodeControlMessage,
	encodeControlMessage,
	type FetchRange,
} from "../../src/core/control-message.ts";
import { MoqtError } from "../../src/core/error.ts";
import {
	connectOver,
	type Fetch,
	type FetchCallOptions,
	type MoqtConnection,
	type SessionConnectOptions,
} from "../../src/webtransport/connection.ts";
import {
	type FetchEnd,
	FetchEndedError,
	FetchStreamResetError,
	RequestAbortedError,
	RequestFailedError,
	SessionClosedError,
} from "../../src/webtransport/endings.ts";
import {
	decodeFetchBody,
	FetchBodyError,
	type FetchItem,
} from "../../src/webtransport/fetch-objects.ts";
import { byHand, hex, MOQTAIL_RELAY_SETUP, toHex, utf8 } from "../helpers.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };
/** From {1, 0} to the end of Group 2. */
const RANGE: FetchRange = {
	fetchType: "Standalone",
	...TRACK,
	startLocation: { group: 1n, object: 0n },
	endLocation: { group: 3n, object: 0n },
};
/** FETCH_OK: End Of Track 0, End Location {3, 0}, no parameters, no Track Properties. */
const FETCH_OK = "18 00 04 00 03 00 00";
/** FETCH_HEADER with Request ID 0, the first request of a session (§11.4.4). */
const FETCH_HEADER = "05 00";
const REQUEST_ERROR = encodeControlMessage({
	type: "REQUEST_ERROR",
	errorCode: 0x11n,
	retryInterval: 0n,
	errorReason: "no such range",
});
const END_OF_STREAM = { done: true, value: undefined } as const;

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

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
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

interface Fetching {
	peer: FakePeer;
	connection: MoqtConnection;
	/** The request stream of the FETCH, whose FETCH the peer has read. */
	request: PeerBidiStream;
	fetch: Fetch;
}

/** Sends a FETCH, which gets Request ID 0, and lets the peer read it. */
async function fetching(
	options: SessionConnectOptions = {},
	callOptions?: FetchCallOptions,
): Promise<Fetching> {
	const { peer, connection } = await connected(options);
	const fetch = await connection.fetch({ range: RANGE }, callOptions);
	const request = await peer.clientBidi();
	await request.sent.read();
	return { peer, connection, request, fetch };
}

/** The peer opens the fetch stream and writes its header. */
async function fetchStream(
	peer: FakePeer,
): Promise<WritableStreamDefaultWriter<Uint8Array>> {
	const stream = peer.openUni();
	await stream.write(hex(FETCH_HEADER));
	return stream;
}

/** Reads `body` to its end, or to the error it fails with. */
async function readAll(
	body: ReadableStream<Uint8Array>,
): Promise<{ bytes: string; error: unknown }> {
	const reader = body.getReader();
	const chunks: string[] = [];
	try {
		for (;;) {
			const next = await reader.read();
			if (next.done) return { bytes: chunks.join(" "), error: undefined };
			chunks.push(toHex(next.value));
		}
	} catch (error) {
		return { bytes: chunks.join(" "), error };
	}
}

describe("MoqtConnection.fetch", () => {
	test("sends the FETCH on a stream of its own and resolves before any answer", async () => {
		const { peer, connection } = await connected();
		const fetch = await connection.fetch({
			range: RANGE,
			parameters: { subscriberPriority: 7 },
		});
		expect(fetch.requestId).toBe(0n);
		const request = await peer.clientBidi();
		const { value } = await request.sent.read();
		expect(decodeControlMessage(value ?? new Uint8Array(0))).toMatchObject({
			status: "ok",
			value: {
				type: "FETCH",
				requestId: 0n,
				range: RANGE,
				parameters: { subscriberPriority: 7 },
			},
		});
	});

	// §10.13: the answer and the stream come in either order.
	test.each([
		{ name: "the answer, then the stream", answerFirst: true },
		{ name: "the stream, then the answer", answerFirst: false },
	])(
		"gives the FETCH_OK and the bytes of the fetch stream: $name",
		async ({ answerFirst }) => {
			const { peer, request, fetch } = await fetching();
			if (answerFirst) await request.reply.write(hex(FETCH_OK));
			const stream = await fetchStream(peer);
			await stream.write(hex("aa bb"));
			const reader = fetch.body.getReader();
			// The bytes are there before the answer, where the stream came first.
			expect(await reader.read()).toEqual({ done: false, value: hex("aa bb") });
			if (!answerFirst) await request.reply.write(hex(FETCH_OK));
			expect(await fetch.response).toEqual({
				endOfTrack: false,
				endLocation: { group: 3n, object: 0n },
				parameters: {},
				trackProperties: [],
			});
			await stream.write(hex("cc"));
			await stream.close();
			expect(await reader.read()).toEqual({ done: false, value: hex("cc") });
			expect(await reader.read()).toEqual(END_OF_STREAM);
			expect(await fetch.ended).toEqual({ type: "served" });
			// The request stream is finished: nothing more is asked on it.
			expect(await request.sent.read()).toEqual(END_OF_STREAM);
		},
	);

	test("the stream that ends before the answer leaves the FETCH open until the answer comes", async () => {
		const { peer, request, fetch } = await fetching();
		const stream = await fetchStream(peer);
		await stream.write(hex("aa"));
		await stream.close();
		expect(await readAll(fetch.body)).toEqual({
			bytes: "aa",
			error: undefined,
		});
		let ended: FetchEnd | undefined;
		void fetch.ended.then((end) => {
			ended = end;
		});
		await settle();
		expect(ended).toBeUndefined();
		await request.reply.write(hex(FETCH_OK));
		expect(await fetch.ended).toEqual({ type: "served" });
	});

	test("a fetch stream nobody reads is read no further than a few chunks", async () => {
		const received: string[] = [];
		const { peer, fetch } = await fetching({
			diagnostics: {
				onBytes: (direction, stream) => {
					if (direction === "received") received.push(stream);
				},
			},
		});
		const stream = await fetchStream(peer);
		// The fetch stream is the peer's second unidirectional stream.
		const chunksRead = (): number =>
			received.filter((name) => name === "peer-uni-1").length;
		for (let chunk = 0; chunk < 10; chunk++) {
			await stream.write(Uint8Array.of(chunk));
		}
		await settle();
		// The header, and the four chunks `body` holds: the connection waits
		// on the fourth and reads no fifth.
		expect(chunksRead()).toBe(5);

		// Reading lets the rest in: nothing was dropped.
		await stream.close();
		expect(await readAll(fetch.body)).toEqual({
			bytes: "00 01 02 03 04 05 06 07 08 09",
			error: undefined,
		});
		expect(chunksRead()).toBe(11);
	});

	// With two streams read at once, one is the control stream and the other
	// the fetch stream, so a third stream waits in the transport.
	test.each<{
		name: string;
		letGo: (context: Fetching) => Promise<unknown>;
	}>([
		{
			name: "a reader cancels `body`",
			letGo: ({ fetch }) => fetch.body.cancel(),
		},
		{ name: "`cancel()`", letGo: ({ fetch }) => fetch.cancel() },
	])(
		"$name: the fetch stream is stopped and its place is free at once, also while `body` is full",
		async ({ letGo }) => {
			const context = await fetching({ maxPeerUniStreams: 2 });
			const { peer, request, fetch } = context;
			const stream = await fetchStream(peer);
			// More than `body` holds: the connection waits on it.
			for (let chunk = 0; chunk < 10; chunk++) {
				await stream.write(Uint8Array.of(chunk));
			}
			const next = peer.openUni();
			await settle();
			expect(peer.untakenUniStreams()).toBe(1);

			await letGo(context);
			// §5.2: STOP_SENDING for the data stream, and the request stream
			// reset. Both carry CANCELLED.
			expect(await rejection(stream.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await rejection(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await fetch.ended).toEqual({ type: "cancelled" });
			await settle();
			expect(peer.untakenUniStreams()).toBe(0);
			void next;
		},
	);

	test("`cancel()` fails `body` for whoever reads it, and `response` with it", async () => {
		const { peer, fetch } = await fetching();
		const stream = await fetchStream(peer);
		await stream.write(hex("aa"));
		const reader = fetch.body.getReader();
		expect(await reader.read()).toEqual({ done: false, value: hex("aa") });
		const pending = reader.read();
		await fetch.cancel();
		const error = await rejection(pending);
		expect(error).toBeInstanceOf(FetchEndedError);
		expect(error).toMatchObject({ end: { type: "cancelled" } });
		const unanswered = await rejection(fetch.response);
		expect(unanswered).toBeInstanceOf(RequestAbortedError);
		expect(unanswered).toMatchObject({ abort: { type: "cancelled" } });
		// A second cancel does nothing.
		await fetch.cancel();
	});

	test("the publisher resets the fetch stream: `body` gives what came and fails with the code", async () => {
		const { peer, request, fetch } = await fetching();
		await request.reply.write(hex(FETCH_OK));
		const stream = await fetchStream(peer);
		await stream.write(hex("aa"));
		await settle();
		await stream.abort(fakeStreamError(9, "reset"));
		const read = await readAll(fetch.body);
		expect(read.bytes).toBe("aa");
		expect(read.error).toBeInstanceOf(FetchStreamResetError);
		expect(read.error).toMatchObject({ code: 9 });
		expect(await fetch.ended).toEqual({ type: "stream-reset", code: 9 });
	});

	test("REQUEST_ERROR: `response` and `body` fail, and a stream that was on its way is stopped", async () => {
		const { peer, request, fetch } = await fetching();
		await request.reply.write(REQUEST_ERROR);
		const refused = await rejection(fetch.response);
		expect(refused).toBeInstanceOf(RequestFailedError);
		expect(refused).toMatchObject({ errorCode: 0x11n });
		const end = await fetch.ended;
		expect(end).toMatchObject({
			type: "rejected",
			error: { errorCode: 0x11n },
		});
		const read = await readAll(fetch.body);
		expect(read.error).toBeInstanceOf(FetchEndedError);
		expect(read.error).toMatchObject({ end: { type: "rejected" } });
		// §5.2: "some Objects could be received even if the FETCH results in
		// error". The stream is stopped, and the session stays.
		const stream = await fetchStream(peer);
		expect(await rejection(stream.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		expect(peer.closeInfo()).toBeUndefined();
	});

	test.each([
		{
			name: "resets the request stream (§5.2)",
			act: (request: PeerBidiStream) =>
				request.reply.abort(fakeStreamError(4, "reset")),
			end: { type: "peer-reset", code: 4 },
		},
		{
			name: "finishes the request stream without an answer",
			act: (request: PeerBidiStream) => request.reply.close(),
			end: { type: "peer-closed" },
		},
	] satisfies {
		name: string;
		act: (request: PeerBidiStream) => Promise<void>;
		end: FetchEnd;
	}[])(
		"the publisher $name: the FETCH ends, and its fetch stream is stopped",
		async ({ act, end }) => {
			const { peer, request, fetch } = await fetching();
			const stream = await fetchStream(peer);
			await stream.write(hex("aa"));
			await settle();
			await act(request);
			expect(await fetch.ended).toEqual(end);
			expect(await rejection(stream.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			// What came before is read first, then the error says why it ends.
			const read = await readAll(fetch.body);
			expect(read.bytes).toBe("aa");
			expect(read.error).toBeInstanceOf(FetchEndedError);
			expect(read.error).toMatchObject({ end });
			expect(await rejection(fetch.response)).toBeInstanceOf(
				RequestAbortedError,
			);
		},
	);

	test("a FETCH_OK with a Mandatory Track Property ends the FETCH and not the session (§2.5.1)", async () => {
		const { peer, request, fetch } = await fetching();
		await request.reply.write(
			encodeControlMessage({
				type: "FETCH_OK",
				endOfTrack: false,
				endLocation: { group: 3n, object: 0n },
				parameters: {},
				trackProperties: [{ type: 0x4000n, value: 1n }],
			}),
		);
		expect(await fetch.ended).toEqual({ type: "unsupported-track-property" });
		expect(await rejection(fetch.response)).toMatchObject({
			abort: { type: "unsupported-track-property" },
		});
		expect(await rejection(request.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
		expect(peer.closeInfo()).toBeUndefined();
	});

	test("a FETCH_OK whose Immutable Properties cannot be parsed ends the FETCH with MALFORMED_TRACK and not the session (§12.7, §2.4.2)", async () => {
		const { peer, request, fetch } = await fetching();
		await request.reply.write(
			encodeControlMessage({
				type: "FETCH_OK",
				endOfTrack: false,
				endLocation: { group: 3n, object: 0n },
				parameters: {},
				// Type 0x11 with a Length of 5 and one byte.
				trackProperties: [{ type: 0x0bn, value: hex("11 05 61") }],
			}),
		);
		expect(await fetch.ended).toEqual({ type: "malformed-track" });
		expect(await rejection(fetch.response)).toMatchObject({
			abort: { type: "malformed-track" },
		});
		expect(await rejection(request.sent.read())).toMatchObject({
			streamErrorCode: 18,
		});
		expect(peer.closeInfo()).toBeUndefined();
	});

	// A `body` that ends with the session says so by the class of its error.
	test.each<{
		name: string;
		close: (context: Fetching) => void;
		closedBy: string;
		bytes: string;
	}>([
		{
			name: "the peer closes the session: what came is read first",
			close: ({ peer }) => peer.close({ closeCode: 0, reason: "bye" }),
			closedBy: "peer",
			bytes: "aa",
		},
		{
			name: "this endpoint closes it: what was not read is dropped",
			close: ({ connection }) => connection.close(),
			closedBy: "local",
			bytes: "",
		},
	])("$name", async ({ close, closedBy, bytes }) => {
		const context = await fetching();
		const stream = await fetchStream(context.peer);
		await stream.write(hex("aa"));
		await settle();
		close(context);
		expect(await context.fetch.ended).toMatchObject({
			type: "session-closed",
			close: { type: closedBy },
		});
		const read = await readAll(context.fetch.body);
		expect(read.bytes).toBe(bytes);
		expect(read.error).toBeInstanceOf(SessionClosedError);
		expect(await rejection(context.fetch.response)).toBeInstanceOf(
			SessionClosedError,
		);
	});

	// The two ways this endpoint ends a FETCH while `body` holds bytes nobody
	// has read: they are dropped, and the first read fails.
	test.each<{
		name: string;
		end: (fetch: Fetch, controller: AbortController) => Promise<void> | void;
	}>([
		{ name: "`cancel()`", end: (fetch) => fetch.cancel() },
		{ name: "the signal", end: (_fetch, controller) => controller.abort() },
	])("$name drops what `body` holds and was not read", async ({ end }) => {
		const controller = new AbortController();
		const { peer, fetch } = await fetching({}, { signal: controller.signal });
		const stream = await fetchStream(peer);
		await stream.write(hex("aa"));
		await settle();
		await end(fetch, controller);
		const read = await readAll(fetch.body);
		expect(read.bytes).toBe("");
		expect(read.error).toBeInstanceOf(FetchEndedError);
		expect(read.error).toMatchObject({ end: { type: "cancelled" } });
	});

	test("a FETCH that is over lets go of its signal", async () => {
		const controller = new AbortController();
		const { signal } = controller;
		const removed: string[] = [];
		const remove = signal.removeEventListener.bind(signal);
		signal.removeEventListener = (
			...args: Parameters<AbortSignal["removeEventListener"]>
		): void => {
			removed.push(args[0]);
			remove(...args);
		};
		const { peer, request, fetch } = await fetching({}, { signal });
		expect(removed).toEqual([]);
		await request.reply.write(hex(FETCH_OK));
		const stream = await fetchStream(peer);
		await stream.close();
		expect(await fetch.ended).toEqual({ type: "served" });
		await settle();
		expect(removed).toEqual(["abort"]);
	});

	test("a FETCH that names no Group Order has Ascending (§10.2.8)", async () => {
		const { fetch } = await fetching();
		expect(fetch.groupOrder).toBe("Ascending");
	});

	test("a signal that aborts cancels the FETCH", async () => {
		const controller = new AbortController();
		const { request, fetch } = await fetching(
			{},
			{ signal: controller.signal },
		);
		controller.abort();
		expect(await fetch.ended).toEqual({ type: "cancelled" });
		expect(await rejection(request.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
	});

	test("with a signal that has aborted nothing is sent, and no Request ID is used", async () => {
		const { peer, connection } = await connected();
		const reason = new Error("too late");
		expect(
			await rejection(
				connection.fetch(
					{ range: RANGE },
					{ signal: AbortSignal.abort(reason) },
				),
			),
		).toBe(reason);
		// The next request is the first one of the session.
		const fetch = await connection.fetch({ range: RANGE });
		expect(fetch.requestId).toBe(0n);
		const request = await peer.clientBidi();
		await request.sent.read();
	});

	test.each<{ name: string; range: FetchRange }>([
		{
			name: "a range that ends before it starts",
			range: {
				...RANGE,
				fetchType: "Standalone",
				startLocation: { group: 2n, object: 0n },
				endLocation: { group: 1n, object: 0n },
			},
		},
		{
			name: "a Joining Request ID that is no SUBSCRIBE of the connection",
			range: {
				fetchType: "RelativeJoining",
				joiningRequestId: 8n,
				joiningStart: 0n,
			},
		},
	])(
		"rejects with a RangeError, and sends nothing, for $name",
		async ({ range }) => {
			const { connection } = await connected();
			expect(await rejection(connection.fetch({ range }))).toBeInstanceOf(
				RangeError,
			);
			expect((await connection.fetch({ range: RANGE })).requestId).toBe(0n);
		},
	);

	test("a fetch stream for a FETCH this endpoint never sent closes the session", async () => {
		const { peer, connection } = await connected();
		await peer.openUni().write(hex(FETCH_HEADER));
		expect(await connection.closed).toMatchObject({
			type: "error",
			error: { code: 3 },
		});
	});
});

/** The items of a decoded body in a few words. */
function brief(item: FetchItem): string {
	return item.type === "object"
		? `{${item.object.groupId}, ${item.object.objectId}} ${toHex(item.object.payload)}`
		: `${item.kind} to {${item.location.group}, ${item.location.object}}`;
}

/** Reads the decoded items to the end, or to the error. */
async function decodeAll(
	items: ReadableStream<FetchItem>,
): Promise<{ items: string[]; error: unknown }> {
	const reader = items.getReader();
	const read: string[] = [];
	try {
		for (;;) {
			const next = await reader.read();
			if (next.done) return { items: read, error: undefined };
			read.push(brief(next.value));
		}
	} catch (error) {
		return { items: read, error };
	}
}

describe("decodeFetchBody", () => {
	/** The bytes of a hand-written fetch stream for Request ID 0, after its header. */
	const afterHeader = (name: string): Uint8Array => {
		const whole = byHand(name);
		if (!whole.startsWith(FETCH_HEADER)) {
			throw new Error(`${name} does not start with ${FETCH_HEADER}`);
		}
		return hex(whole.slice(FETCH_HEADER.length));
	};

	// fixtures/moqt-18-by-hand/README.md has what the file decodes to.
	test("decodes Objects and End of Range indicators, however the bytes are cut", async () => {
		const { peer, connection, request, fetch } = await fetching();
		await request.reply.write(hex(FETCH_OK));
		const stream = await fetchStream(peer);
		const items = decodeFetchBody(connection, fetch);
		const read = decodeAll(items);
		for (const byte of afterHeader("fetch-stream-end-of-range")) {
			await stream.write(Uint8Array.of(byte));
		}
		await stream.close();
		expect(await read).toEqual({
			items: [
				"{0, 0} aa",
				"NonExistent to {0, 5}",
				"{0, 6} bb",
				"Unknown to {2, 0}",
				"{2, 3} cc",
			],
			error: undefined,
		});
		expect(await fetch.ended).toEqual({ type: "served" });
	});

	test("counts the Group IDs down for a FETCH that asked for Descending", async () => {
		// The file is for Request ID 4: the third request of this session.
		const { peer, connection } = await connected();
		await connection.fetch({ range: RANGE });
		await connection.fetch({ range: RANGE });
		const fetch = await connection.fetch({
			range: RANGE,
			parameters: { groupOrder: "Descending" },
		});
		expect(fetch.requestId).toBe(4n);
		// The order is the FETCH's: whoever decodes cannot give another.
		expect(fetch.groupOrder).toBe("Descending");
		const stream = peer.openUni();
		await stream.write(hex(byHand("fetch-stream-descending")));
		await stream.close();
		expect(await decodeAll(decodeFetchBody(connection, fetch))).toEqual({
			items: ["{9, 0} aa", "{7, 1} bb", "{6, 5} cc"],
			error: undefined,
		});
	});

	// `1c 00 00 07 01 aa` is Object {0, 0} with the payload aa.
	test.each<{
		name: string;
		bytes: string;
		fin: boolean;
		items: string[];
	}>([
		{
			// §11.4.4.1: flags 0x00 take every field from the prior Object.
			name: "a first Object that refers to a prior one",
			bytes: "00 01 aa",
			fin: false,
			items: [],
		},
		{
			name: "Serialization Flags the draft does not define",
			bytes: "1c 00 00 07 01 aa 80 8d",
			fin: false,
			items: ["{0, 0} aa"],
		},
		{
			// Flags 0x00: the Object after {0, 0}, with one byte of payload. All
			// of it comes in one read, and both Objects are delivered first.
			name: "the same after two Objects that came in the same read",
			bytes: "1c 00 00 07 01 aa 00 01 bb 80 8d",
			fin: false,
			items: ["{0, 0} aa", "{0, 1} bb"],
		},
		{
			name: "a stream that ends inside an Object",
			bytes: "1c 00 00 07 01 aa 1c 01",
			fin: true,
			items: ["{0, 0} aa"],
		},
	])(
		"closes the session with PROTOCOL_VIOLATION for $name",
		async ({ bytes, fin, items }) => {
			const { peer, connection, fetch } = await fetching();
			const stream = await fetchStream(peer);
			await stream.write(hex(bytes));
			if (fin) await stream.close();
			const read = await decodeAll(decodeFetchBody(connection, fetch));
			expect(read.items).toEqual(items);
			expect(read.error).toBeInstanceOf(MoqtError);
			expect(read.error).toMatchObject({ code: 3 });
			// As for a fault the session finds: an error, not this endpoint's
			// own close.
			expect(await connection.closed).toMatchObject({
				type: "error",
				error: { code: 3 },
			});
			expect(peer.closeInfo()).toMatchObject({ closeCode: 3 });
		},
	);

	test.each<{
		name: string;
		bytes: string;
		scope: "track" | "stream";
	}>([
		{
			// Flags 0x3c: with Properties. A Prior Object ID Gap (0x3e) of 1 on
			// Object 0 (§12.9).
			name: "Object Properties that make the Track malformed",
			bytes: "3c 00 00 07 02 3e 01 00",
			scope: "track",
		},
		{
			// A payload of 2 bytes against a limit of 1.
			name: "an Object over a limit of this endpoint",
			bytes: "1c 00 00 07 02 aa bb",
			scope: "stream",
		},
	])(
		"cancels the FETCH, and leaves the session open, for $name",
		async ({ bytes, scope }) => {
			const { peer, connection, request, fetch } = await fetching();
			const stream = await fetchStream(peer);
			await stream.write(hex(bytes));
			const read = await decodeAll(
				decodeFetchBody(connection, fetch, {
					limits: { maxObjectPayloadLength: 1 },
				}),
			);
			expect(read.error).toBeInstanceOf(FetchBodyError);
			expect(read.error).toMatchObject({ scope });
			expect(await fetch.ended).toEqual({ type: "cancelled" });
			expect(await rejection(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(peer.closeInfo()).toBeUndefined();
		},
	);

	test("a FETCH that is cancelled inside an Object fails the result and does not close the session", async () => {
		const { peer, connection, fetch } = await fetching();
		const stream = await fetchStream(peer);
		await stream.write(hex("1c 00 00 07 01 aa 1c 01"));
		const read = decodeAll(decodeFetchBody(connection, fetch));
		await settle();
		await fetch.cancel();
		const { items, error } = await read;
		expect(items).toEqual(["{0, 0} aa"]);
		expect(error).toBeInstanceOf(FetchEndedError);
		expect(peer.closeInfo()).toBeUndefined();
	});

	test("cancelling the result cancels the FETCH", async () => {
		const { connection, request, fetch } = await fetching();
		await decodeFetchBody(connection, fetch).cancel();
		expect(await fetch.ended).toEqual({ type: "cancelled" });
		expect(await rejection(request.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
	});
});

test("a subscription of the session hands on what it holds before it fails, when a fetch stream closes the session", async () => {
	const { peer, connection } = await connected();
	// SUBSCRIBE (Request ID 0), answered with SUBSCRIBE_OK, Track Alias 9.
	const subscribing = connection.subscribe(TRACK);
	const subscribe = await peer.clientBidi();
	await subscribe.sent.read();
	await subscribe.reply.write(hex("04 00 02 09 00"));
	const subscription = await subscribing;
	// One Object for it, not read yet: a subgroup stream of Group 0, Subgroup
	// 0, and Object 0 with the payload 61.
	await peer.openUni().write(hex("14 09 00 00 00 00 01 61"));
	await settle();
	// FETCH (Request ID 2), whose stream breaks §11.4.4.1: a first Object
	// that refers to a prior one.
	const fetch = await connection.fetch({ range: RANGE });
	await (await peer.clientBidi()).sent.read();
	const stream = peer.openUni();
	await stream.write(hex("05 02"));
	await stream.write(hex("00 01 aa"));
	await decodeAll(decodeFetchBody(connection, fetch));
	const reader = subscription.objects.getReader();
	const first = await reader.read();
	expect({
		first: first.done ? undefined : toHex(first.value.object.payload),
		after: await rejection(reader.read()),
	}).toMatchObject({ first: "61", after: { name: "SubscriptionEndedError" } });
});
