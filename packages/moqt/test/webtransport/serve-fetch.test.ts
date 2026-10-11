import { describe, expect, jest, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { createFetchStreamDecoder } from "../../src/core/fetch.ts";
import {
	connectOver,
	type FetchWriter,
	type MoqtConnection,
	type PeerFetch,
} from "../../src/webtransport/connection.ts";
import {
	RequestAbortedError,
	type ServedFetchEnd,
	ServedFetchEndedError,
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

/**
 * The captured Standalone FETCH with Request ID 1, one of the server, for
 * {3, 4} to {5, 1}, and its one parameter (SUBSCRIBER_PRIORITY 200) or the
 * ones given.
 */
function fetchBytes(parameters = "01 20 c8", requestId = 1): Uint8Array {
	const pairs = fixture("fetch-standalone-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	pairs.splice(25, 7, "03", "04", "05", "01", ...parameters.split(" "));
	pairs[2] = (pairs.length - 3).toString(16).padStart(2, "0");
	return hex(pairs.join(" "));
}

/** REQUEST_UPDATE with Request ID 3 and no parameters, written by hand: no capture of one exists. */
const REQUEST_UPDATE = hex("02 00 02 03 00");

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
		case "FETCH_OK":
			return `FETCH_OK end {${value.endLocation.group}, ${value.endLocation.object}} endOfTrack ${value.endOfTrack}`;
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

const END = { done: true, value: undefined } as const;

/**
 * What a reader of the fetch stream gets from the next `chunks` chunks the
 * client wrote, after the FETCH_HEADER for Request ID 1: decoded with the
 * decoder that reads moqtail's fetch streams (test/core/fetch.test.ts).
 */
async function nextItems(
	data: ReadableStreamDefaultReader<Uint8Array>,
	chunks: number,
	decoder = createFetchStreamDecoder(
		{ maxObjectPayloadLength: 100, maxObjectPropertiesLength: 100 },
		"Ascending",
	),
): Promise<string[]> {
	decoder.push(hex("05 01"));
	const items: string[] = [];
	for (let chunk = 0; chunk < chunks; chunk++) {
		for (const item of decoder.push(hex(await sentHex(data))).items) {
			if (item.type === "object") {
				items.push(
					`{${item.object.groupId}, ${item.object.objectId}} ${new TextDecoder().decode(item.object.payload)}`,
				);
			} else if (item.type === "end-of-range") {
				items.push(
					`${item.kind} to {${item.location.group}, ${item.location.object}}`,
				);
			}
		}
	}
	return items;
}

const object = (groupId: bigint, objectId: bigint, text: string) => ({
	groupId,
	subgroupId: 0n,
	objectId,
	publisherPriority: 0,
	properties: [],
	payload: utf8(text),
});

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

/** A connection that keeps the FETCHes it is asked with, and whose peer sent one. */
async function asked(parameters?: string): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	stream: PeerBidiStream;
	request: PeerFetch;
}> {
	const { peer, connection } = await connected();
	const requests: PeerFetch[] = [];
	connection.serve({
		fetch: (request) => {
			requests.push(request);
		},
	});
	const stream = peer.openBidi();
	await stream.reply.write(fetchBytes(parameters));
	await settle();
	const [request] = requests;
	if (request === undefined) throw new Error("the handler was not called");
	return { peer, connection, stream, request };
}

/** That, accepted with what was asked for; the FETCH_OK and the FETCH_HEADER are read. */
async function serving(): Promise<
	Awaited<ReturnType<typeof asked>> & {
		writer: FetchWriter;
		data: ReadableStreamDefaultReader<Uint8Array>;
	}
> {
	const all = await asked();
	const writer = await all.request.accept({
		endLocation: { group: 5n, object: 1n },
	});
	await all.stream.sent.read();
	const data = await all.peer.clientUni();
	await data.read();
	return { ...all, writer, data };
}

describe("serve: a Standalone FETCH of the peer", () => {
	test("is refused with NOT_SUPPORTED while nothing serves a FETCH", async () => {
		const { peer, connection } = await connected();
		connection.serve({ subscribe: () => {} });
		const stream = peer.openBidi();
		await stream.reply.write(fetchBytes());
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 3 "this endpoint serves no FETCH"',
		);
		expect(await stream.sent.read()).toEqual(END);
		expect(peer.closeInfo()).toBeUndefined();
	});

	test("is given to the handler with its range as the FETCH has it, its order and its FILL_TIMEOUT", async () => {
		// FILL_TIMEOUT 0 (0x0a) and GROUP_ORDER Descending (0x22).
		const { request } = await asked("02 0a 00 18 02");
		expect({
			requestId: request.requestId,
			name: new TextDecoder().decode(request.trackName),
			start: request.startLocation,
			end: request.endLocation,
			groupOrder: request.groupOrder,
			fillTimeout: request.fillTimeout,
			aborted: request.signal.aborted,
		}).toEqual({
			requestId: 1n,
			name: "object",
			start: { group: 3n, object: 4n },
			end: { group: 5n, object: 1n },
			groupOrder: "Descending",
			fillTimeout: 0n,
			aborted: false,
		});
	});

	test("without a GROUP_ORDER or a FILL_TIMEOUT, is in ascending order and has no time", async () => {
		const { request } = await asked();
		expect([request.groupOrder, request.fillTimeout]).toEqual([
			"Ascending",
			undefined,
		]);
	});

	test("accepted, gets FETCH_OK, the fetch stream at once, what is written, and both streams finished", async () => {
		const { peer, stream, request } = await asked();
		const writer = await request.accept({
			endLocation: { group: 5n, object: 1n },
			endOfTrack: true,
		});
		expect(await sentMessage(stream.sent)).toBe(
			"FETCH_OK end {5, 1} endOfTrack true",
		);
		const data = await peer.clientUni();
		// The Stream Type of a fetch stream and the Request ID: before anything
		// was written.
		expect(await sentHex(data)).toBe("05 01");
		await writer.write(object(3n, 4n, "a"));
		await writer.write(object(3n, 5n, "b"));
		await writer.endOfRange("NonExistent", { group: 4n, object: 9n });
		expect(await nextItems(data, 3)).toEqual([
			"{3, 4} a",
			"{3, 5} b",
			"NonExistent to {4, 9}",
		]);
		await writer.close();
		expect(await data.read()).toEqual(END);
		expect(await writer.ended).toEqual({ type: "served" });
		// This endpoint's direction of the request stream stays open until
		// the subscriber has ended its own.
		const finished = stream.sent.read();
		expect(await Promise.race([finished, settle()])).toBeUndefined();
		await stream.reply.close();
		expect(await finished).toEqual(END);
	});

	test("served, finishes its direction of the request stream when the subscriber does not end its own in time, and has the subscriber's stopped after that time again", async () => {
		const { stream, writer } = await serving();
		jest.useFakeTimers();
		try {
			await writer.close();
			const finished = stream.sent.read();
			const turns = async (): Promise<void> => {
				for (let turn = 0; turn < 200; turn++) await Promise.resolve();
			};
			await turns();
			jest.advanceTimersByTime(19);
			await turns();
			expect(await Promise.race([finished, Promise.resolve("open")])).toBe(
				"open",
			);
			jest.advanceTimersByTime(1);
			await turns();
			// WHY not awaited alone: the clock stands still here, and a read
			// that never comes would hold the test for good.
			expect(await Promise.race([finished, Promise.resolve("open")])).toEqual(
				END,
			);
			// The subscriber's direction is still its own.
			await stream.reply.write(new Uint8Array(0));
			jest.advanceTimersByTime(19);
			await turns();
			expect(jest.getTimerCount()).toBe(1);
			jest.advanceTimersByTime(1);
			await turns();
			expect(
				await rejection(stream.reply.write(new Uint8Array(1))),
			).toMatchObject({ streamErrorCode: 1 });
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
	});

	test("rejected, gets REQUEST_ERROR with the code and the reason, and a FIN", async () => {
		const { stream, request } = await asked();
		await request.reject(0x10n, "nothing of that Track");
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 16 "nothing of that Track"',
		);
		expect(await stream.sent.read()).toEqual(END);
	});

	test("accepted with an End Location before its Start Location, still waits", async () => {
		const { stream, request } = await asked();
		expect(
			await rejection(
				request.accept({ endLocation: { group: 3n, object: 3n } }),
			),
		).toBeInstanceOf(RangeError);
		await request.accept({ endLocation: { group: 3n, object: 4n } });
		expect(await sentMessage(stream.sent)).toBe(
			"FETCH_OK end {3, 4} endOfTrack false",
		);
	});

	test("an Object that goes back is refused, and the writer goes on", async () => {
		const { writer, data } = await serving();
		await writer.write(object(3n, 5n, "a"));
		expect(
			await rejection(writer.write(object(3n, 5n, "again"))),
		).toBeInstanceOf(RangeError);
		await writer.write(object(3n, 6n, "b"));
		// The Object after the refused one follows the one before it.
		expect(await nextItems(data, 2)).toEqual(["{3, 5} a", "{3, 6} b"]);
		expect(await Promise.race([writer.ended, settle()])).toBeUndefined();
	});

	test("a REQUEST_UPDATE before the answer gets its REQUEST_OK after the FETCH_OK", async () => {
		const { stream, request } = await asked();
		await stream.reply.write(REQUEST_UPDATE);
		await settle();
		expect(request.signal.aborted).toBe(false);
		await request.accept({ endLocation: { group: 5n, object: 1n } });
		expect([
			await sentMessage(stream.sent),
			await sentMessage(stream.sent),
		]).toEqual(["FETCH_OK end {5, 1} endOfTrack false", "REQUEST_OK"]);
	});

	test("65 REQUEST_UPDATEs before the answer: the FETCH is refused with EXCESSIVE_LOAD, and `signal` aborts", async () => {
		const { stream, request } = await asked();
		const updates: number[] = [];
		for (let index = 0; index < 65; index++) {
			const id = 3 + index * 2;
			updates.push(0x02, 0x00, id < 128 ? 0x02 : 0x03);
			if (id < 128) updates.push(id, 0x00);
			else updates.push(0x80, id, 0x00);
		}
		await stream.reply.write(new Uint8Array(updates));
		expect(await sentMessage(stream.sent)).toBe(
			'REQUEST_ERROR 9 "too many REQUEST_UPDATE messages before the answer"',
		);
		expect(await stream.sent.read()).toEqual(END);
		expect(request.signal.reason).toMatchObject({
			name: "RequestAbortedError",
			abort: { type: "update-failed" },
		});
	});

	test("a handler that throws refuses the FETCH with INTERNAL_ERROR", async () => {
		const { peer, connection } = await connected();
		const original = globalThis.reportError;
		const reported: unknown[] = [];
		globalThis.reportError = (error: unknown) => {
			reported.push(error);
		};
		try {
			const failure = new Error("the application failed");
			connection.serve({
				fetch: async () => {
					throw failure;
				},
			});
			const stream = peer.openBidi();
			await stream.reply.write(fetchBytes());
			expect(await sentMessage(stream.sent)).toBe(
				'REQUEST_ERROR 0 "the publisher failed to handle the request"',
			);
			expect(reported).toEqual([failure]);
		} finally {
			globalThis.reportError = original;
		}
	});
});

describe("serve: a FETCH the subscriber gives up", () => {
	// Before the answer: `signal` aborts.
	test.each([
		{
			name: "it resets the request stream",
			act: (stream: PeerBidiStream) =>
				stream.reply.abort(fakeStreamError(1, "reset")),
			abort: { type: "peer-reset", code: 1 },
		},
		{
			name: "it stops the request stream",
			act: (stream: PeerBidiStream) =>
				stream.sent.cancel(fakeStreamError(7, "stop")),
			abort: { type: "peer-reset", code: 7 },
		},
	])(
		"before the answer, $name: `signal` aborts and an answer is rejected",
		async ({ act, abort }) => {
			const { stream, request } = await asked();
			await act(stream);
			await settle();
			expect(request.signal.reason).toBeInstanceOf(RequestAbortedError);
			expect(request.signal.reason).toMatchObject({ abort });
			expect(
				await rejection(
					request.accept({ endLocation: { group: 5n, object: 1n } }),
				),
			).toBe(request.signal.reason);
		},
	);

	// While it is served: both streams are reset (§5.2), and the writer ends.
	test.each([
		{
			name: "it resets the request stream",
			act: ({ stream }: Awaited<ReturnType<typeof serving>>) =>
				stream.reply.abort(fakeStreamError(1, "reset")),
			code: 1,
		},
		{
			name: "it stops the request stream",
			act: ({ stream }: Awaited<ReturnType<typeof serving>>) =>
				stream.sent.cancel(fakeStreamError(7, "stop")),
			code: 7,
		},
		{
			name: "it stops the fetch stream alone",
			act: ({ data }: Awaited<ReturnType<typeof serving>>) =>
				data.cancel(fakeStreamError(7, "stop")),
			code: 7,
		},
		{
			name: "it stops the fetch stream and the browser passes no code",
			act: ({ data }: Awaited<ReturnType<typeof serving>>) =>
				data.cancel(fakeStopWithoutCode()),
			code: undefined,
		},
	])(
		"while it is served, $name: the writer ends, and its calls fail",
		async ({ act, code }) => {
			const all = await serving();
			await act(all);
			const end: ServedFetchEnd = { type: "peer-cancelled", code };
			expect(await all.writer.ended).toEqual(end);
			const error = await rejection(all.writer.write(object(3n, 4n, "a")));
			expect(error).toBeInstanceOf(ServedFetchEndedError);
			expect(error).toMatchObject({ end });
			// Neither needs doing on a FETCH that is over.
			await all.writer.close();
			await all.writer.reset();
			await settle();
			expect(all.peer.closeInfo()).toBeUndefined();
		},
	);

	test("a write that the subscriber's stop fails is rejected with how the FETCH ended", async () => {
		const { writer, data } = await serving();
		// Not awaited: the write is made while the stop is on its way.
		void data.cancel(fakeStreamError(7, "stop"));
		const error = await rejection(writer.write(object(3n, 4n, "a")));
		expect(error).toBeInstanceOf(ServedFetchEndedError);
		expect(error).toMatchObject({ end: { type: "peer-cancelled", code: 7 } });
	});
});

describe("serve: a FETCH this endpoint ends, and the end of the session", () => {
	test("`reset` resets both streams with the code, and the writer ends", async () => {
		const { stream, writer, data } = await serving();
		await writer.reset(0);
		expect(await writer.ended).toEqual({ type: "reset" });
		expect(await rejection(data.read())).toMatchObject({ streamErrorCode: 0 });
		expect(await rejection(stream.sent.read())).toMatchObject({
			streamErrorCode: 0,
		});
		expect(await rejection(writer.write(object(3n, 4n, "a")))).toBeInstanceOf(
			ServedFetchEndedError,
		);
	});

	test.each([{ code: -1 }, { code: 256 }, { code: 1.5 }])(
		"`reset($code)` is refused, and the FETCH goes on",
		async ({ code }) => {
			const { writer, data } = await serving();
			expect(await rejection(writer.reset(code))).toBeInstanceOf(RangeError);
			await writer.write(object(3n, 4n, "a"));
			expect(await nextItems(data, 1)).toEqual(["{3, 4} a"]);
		},
	);

	test("the end of the session ends the writer and aborts a FETCH that waits", async () => {
		const served = await serving();
		served.connection.close(0, "bye");
		expect(await served.writer.ended).toMatchObject({
			type: "session-closed",
			close: { type: "local", code: 0, reason: "bye" },
		});
		expect(
			await rejection(served.writer.write(object(3n, 4n, "a"))),
		).toBeInstanceOf(ServedFetchEndedError);
		const waiting = await asked();
		waiting.connection.close();
		expect(waiting.request.signal.reason).toBeInstanceOf(SessionClosedError);
	});

	test("the place of the request stream is free again when the peer has ended its direction", async () => {
		const peer = createFakePeer();
		const connecting = connectOver(peer.transport, { maxPeerBidiStreams: 1 });
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		const requests: PeerFetch[] = [];
		connection.serve({
			fetch: (request) => {
				requests.push(request);
			},
		});
		const first = peer.openBidi();
		await first.reply.write(fetchBytes());
		await settle();
		const writer = await requests[0]?.accept({
			endLocation: { group: 5n, object: 1n },
		});
		const second = peer.openBidi();
		await second.reply.write(fetchBytes(undefined, 3));
		await settle();
		expect(peer.untakenBidiStreams()).toBe(1);
		await writer?.close();
		await first.reply.close();
		await settle();
		expect(peer.untakenBidiStreams()).toBe(0);
		expect(requests.length).toBe(2);
	});
});

/** A handler's promise, held for the test to wait for. */
interface Handled {
	done: Promise<unknown> | undefined;
}

// What a handler lets through after the subscriber gave the FETCH up. The
// rejection of a write then says how the FETCH ended, which is no error of
// the application; anything else it throws is.
describe("serve: a handler that is writing when the FETCH ends", () => {
	const mistake = new Error("the application failed");
	const rows: {
		name: string;
		/** What the handler does with the writer once the peer has cancelled. */
		afterwards: (writer: FetchWriter) => Promise<void>;
		reported: string[];
	}[] = [
		{
			name: "lets the rejection of its write through: not reported",
			afterwards: (writer) => writer.write(object(3n, 4n, "a")),
			reported: [],
		},
		{
			name: "throws something else: reported",
			afterwards: () => Promise.reject(mistake),
			reported: ["Error: the application failed"],
		},
	];
	test.each(rows)(
		"the subscriber cancels, and the handler $name",
		async ({ afterwards, reported }) => {
			const { peer, connection } = await connected();
			const seen: string[] = [];
			const original = globalThis.reportError;
			globalThis.reportError = (error: unknown) => {
				seen.push(String(error));
			};
			try {
				let cancelled: () => void = () => {};
				const waited = new Promise<void>((resolve) => {
					cancelled = resolve;
				});
				const handled: Handled = { done: undefined };
				connection.serve({
					fetch: (request) => {
						handled.done = (async () => {
							const writer = await request.accept({
								endLocation: { group: 5n, object: 1n },
							});
							await waited;
							await afterwards(writer);
						})();
						return handled.done;
					},
				});
				const stream = peer.openBidi();
				await stream.reply.write(fetchBytes());
				await stream.sent.read();
				await stream.reply.abort(fakeStreamError(1, "reset"));
				await settle();
				cancelled();
				await rejection(handled.done ?? Promise.resolve());
				await settle();
				expect(seen).toEqual(reported);
				expect(peer.closeInfo()).toBeUndefined();
			} finally {
				globalThis.reportError = original;
			}
		},
	);

	test("a handler that writes after it reset the writer itself is reported: that is its own mistake", async () => {
		const { peer, connection } = await connected();
		const seen: string[] = [];
		const original = globalThis.reportError;
		globalThis.reportError = (error: unknown) => {
			seen.push(error instanceof ServedFetchEndedError ? error.end.type : "?");
		};
		try {
			const handled: Handled = { done: undefined };
			connection.serve({
				fetch: (request) => {
					handled.done = (async () => {
						const writer = await request.accept({
							endLocation: { group: 5n, object: 1n },
						});
						await writer.reset();
						await writer.write(object(3n, 4n, "a"));
					})();
					return handled.done;
				},
			});
			const stream = peer.openBidi();
			await stream.reply.write(fetchBytes());
			await settle();
			await rejection(handled.done ?? Promise.resolve());
			await settle();
			expect(seen).toEqual(["reset"]);
		} finally {
			globalThis.reportError = original;
		}
	});
});

describe("serve: writes of a fetch stream that are not waited for", () => {
	test("16 can be under way; one more is refused, an indicator as well, and the writer is as it was", async () => {
		let stalled = false;
		const peer = createFakePeer({ stallWrites: () => stalled });
		const connecting = connectOver(peer.transport);
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		const requests: PeerFetch[] = [];
		connection.serve({
			fetch: (request) => {
				requests.push(request);
			},
		});
		const stream = peer.openBidi();
		await stream.reply.write(fetchBytes());
		await settle();
		const writer = await requests[0]?.accept({
			endLocation: { group: 5n, object: 1n },
		});
		if (writer === undefined) throw new Error("the handler was not called");
		// The transport takes no write from here on, as for a subscriber that
		// reads nothing.
		stalled = true;
		const underWay = Array.from({ length: 16 }, (_unused, index) =>
			writer.write(object(3n, BigInt(4 + index), "a")),
		);
		for (const each of underWay) each.catch(() => {});
		const refused = await rejection(writer.write(object(3n, 20n, "a")));
		expect(refused).toBeInstanceOf(RangeError);
		expect(refused).toMatchObject({
			message:
				"16 writes of the stream are under way: wait for a write before the next",
		});
		expect(
			await rejection(
				writer.endOfRange("NonExistent", { group: 4n, object: 0n }),
			),
		).toBeInstanceOf(RangeError);
		// Neither was a write: none of the 16 has settled, and the FETCH goes
		// on.
		expect(await Promise.race([writer.ended, settle()])).toBeUndefined();
		await writer.reset();
		expect(await writer.ended).toEqual({ type: "reset" });
	});
});

describe("serve: writes of a fetch stream that are waited for", () => {
	test("more than 16, one after the other, are all taken: a write that settled is no longer under way", async () => {
		const { writer, data } = await serving();
		for (let index = 0; index < 40; index++) {
			await writer.write(object(3n, BigInt(4 + index), "a"));
			await data.read();
		}
		await writer.close();
		expect(await writer.ended).toEqual({ type: "served" });
	});
});

describe("serve: a write that fails is no longer under way", () => {
	test("after 20 writes that were refused for their order, 16 can still be under way", async () => {
		let stalled = false;
		const peer = createFakePeer({ stallWrites: () => stalled });
		const connecting = connectOver(peer.transport);
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		const requests: PeerFetch[] = [];
		connection.serve({
			fetch: (request) => {
				requests.push(request);
			},
		});
		const stream = peer.openBidi();
		await stream.reply.write(fetchBytes());
		await settle();
		const writer = await requests[0]?.accept({
			endLocation: { group: 5n, object: 1n },
		});
		if (writer === undefined) throw new Error("the handler was not called");
		await writer.write(object(3n, 4n, "a"));
		for (let index = 0; index < 20; index++) {
			expect(
				await rejection(writer.write(object(3n, 4n, "again"))),
			).toMatchObject({ message: "Object IDs must increase within a Group" });
		}
		stalled = true;
		const underWay = Array.from({ length: 16 }, (_unused, index) =>
			writer.write(object(3n, BigInt(5 + index), "a")),
		);
		for (const each of underWay) each.catch(() => {});
		await settle();
		// The 17th is the first that is refused for the count.
		expect(await rejection(writer.write(object(3n, 30n, "a")))).toMatchObject({
			message:
				"16 writes of the stream are under way: wait for a write before the next",
		});
		await writer.reset();
	});

	test("with 16 writes under way when the session ends, a further write is rejected for the end, not for the count", async () => {
		let stalled = false;
		const peer = createFakePeer({ stallWrites: () => stalled });
		const connecting = connectOver(peer.transport);
		await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
		const connection = await connecting;
		const requests: PeerFetch[] = [];
		connection.serve({
			fetch: (request) => {
				requests.push(request);
			},
		});
		const stream = peer.openBidi();
		await stream.reply.write(fetchBytes());
		await settle();
		const writer = await requests[0]?.accept({
			endLocation: { group: 5n, object: 1n },
		});
		if (writer === undefined) throw new Error("the handler was not called");
		stalled = true;
		const underWay = Array.from({ length: 16 }, (_unused, index) =>
			writer.write(object(3n, BigInt(4 + index), "a")),
		);
		for (const each of underWay) each.catch(() => {});
		await settle();
		connection.close(0, "bye");
		const error = await rejection(writer.write(object(3n, 30n, "a")));
		expect(error).toBeInstanceOf(ServedFetchEndedError);
		expect(error).toMatchObject({ end: { type: "session-closed" } });
	});
});

describe("serve: the wait after `close()`", () => {
	test("a subscriber that had finished its direction before leaves no timer behind", async () => {
		const { stream, writer } = await serving();
		await stream.reply.close();
		await settle();
		jest.useFakeTimers();
		try {
			await writer.close();
			for (let turn = 0; turn < 200; turn++) await Promise.resolve();
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
	});

	test("a subscriber that ends its direction while it is waited for leaves no timer behind, and this endpoint's direction is finished", async () => {
		const { stream, writer } = await serving();
		jest.useFakeTimers();
		try {
			await writer.close();
			await stream.reply.close();
			for (let turn = 0; turn < 200; turn++) await Promise.resolve();
			expect(jest.getTimerCount()).toBe(0);
			expect(
				await Promise.race([stream.sent.read(), Promise.resolve("open")]),
			).toEqual(END);
		} finally {
			jest.useRealTimers();
		}
	});

	test("a subscriber that stops this endpoint's direction while it is waited for: both directions end at once, with CANCELLED, and no timer is left", async () => {
		const { stream, writer } = await serving();
		jest.useFakeTimers();
		try {
			await writer.close();
			// STOP_SENDING for this endpoint's direction of the request stream.
			void stream.sent.cancel(fakeStreamError(7, "stop"));
			for (let turn = 0; turn < 200; turn++) await Promise.resolve();
			expect(jest.getTimerCount()).toBe(0);
			expect(
				await rejection(stream.reply.write(new Uint8Array(1))),
			).toMatchObject({ streamErrorCode: 1 });
		} finally {
			jest.useRealTimers();
		}
	});

	test("a subscriber that has not finished is waited for with one timer", async () => {
		const { writer } = await serving();
		jest.useFakeTimers();
		try {
			await writer.close();
			for (let turn = 0; turn < 200; turn++) await Promise.resolve();
			expect(jest.getTimerCount()).toBe(1);
		} finally {
			jest.useRealTimers();
		}
	});
});
