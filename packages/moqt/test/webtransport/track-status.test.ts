import { describe, expect, test } from "bun:test";
import {
	connectOver,
	type MoqtConnection,
} from "../../src/webtransport/connection.ts";
import {
	RequestAbortedError,
	RequestFailedError,
	SessionClosedError,
} from "../../src/webtransport/endings.ts";
import { fixture, hex, MOQTAIL_RELAY_SETUP, toHex } from "../helpers.ts";
import { flush, state } from "./clock.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

// TRACK_STATUS (§10.14) through the connection. What the session does with
// the bytes is in test/core/session-track-status.test.ts; the wait for an
// answer is in answer-wait.test.ts, and the connection's token in
// connection.test.ts.

const TRACK = { trackNamespace: [hex("61")], trackName: hex("62") };
/** TRACK_STATUS of (`a`) `b`, Request ID 0, no parameters. */
const TRACK_STATUS_0 = "0d 00 07 00 01 01 61 01 62 00";
const END_OF_STREAM: { done: true; value: undefined } = {
	done: true,
	value: undefined,
};

async function connected(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
}> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

async function sentHex(stream: PeerBidiStream): Promise<string> {
	const { value } = await stream.sent.read();
	if (value === undefined) throw new Error("the stream ended");
	return toHex(value);
}

/** The session goes on: another TRACK_STATUS, with the next Request ID, is answered. */
async function asksAgain(
	peer: FakePeer,
	connection: MoqtConnection,
): Promise<void> {
	const again = connection.trackStatus(TRACK);
	const stream = await peer.clientBidi();
	expect(await sentHex(stream)).toBe("0d 00 07 02 01 01 61 01 62 00");
	await stream.reply.write(hex("07 00 01 00"));
	expect(await again).toEqual({
		largestObject: undefined,
		trackProperties: [],
	});
	expect(await state(connection.closed)).toBe("pending");
}

describe("trackStatus", () => {
	test.each<{
		name: string;
		answer: string;
		largestObject: { group: bigint; object: bigint } | undefined;
		trackProperties: { type: bigint; value: bigint }[];
	}>([
		{
			name: "answered as the moqtail relay answers: LARGEST_OBJECT {0, 0}, no Track Properties",
			answer: fixture("request-ok-track-status-moqtail-relay"),
			largestObject: { group: 0n, object: 0n },
			trackProperties: [],
		},
		{
			name: "answered with LARGEST_OBJECT {4, 1} and the Track Property 2 = 5",
			answer: "07 00 06 01 09 04 01 02 05",
			largestObject: { group: 4n, object: 1n },
			trackProperties: [{ type: 2n, value: 5n }],
		},
		{
			name: "answered without LARGEST_OBJECT, with the Track Property 2 = 5",
			answer: "07 00 03 00 02 05",
			largestObject: undefined,
			trackProperties: [{ type: 2n, value: 5n }],
		},
		{
			name: "answered without LARGEST_OBJECT and without Track Properties",
			answer: "07 00 01 00",
			largestObject: undefined,
			trackProperties: [],
		},
	])(
		"$name: resolves with them, and the client finishes its direction",
		async ({ answer, largestObject, trackProperties }) => {
			const { peer, connection } = await connected();
			const asking = connection.trackStatus(TRACK);
			const stream = await peer.clientBidi();
			expect(await sentHex(stream)).toBe(TRACK_STATUS_0);
			await stream.reply.write(hex(answer));
			expect(await asking).toEqual({ largestObject, trackProperties });
			// §10.14: "The bidi stream is closed with a FIN after TRACK_STATUS_OK".
			expect(await stream.sent.read()).toEqual(END_OF_STREAM);
			await stream.reply.close();
			await asksAgain(peer, connection);
		},
	);

	test.each<{
		name: string;
		end: (stream: PeerBidiStream) => Promise<void>;
		error: typeof RequestFailedError | typeof RequestAbortedError;
		fields: Record<string, unknown>;
	}>([
		{
			name: "refused with REQUEST_ERROR DOES_NOT_EXIST",
			// REQUEST_ERROR 0x10, Retry Interval 0, no reason.
			end: (stream) => stream.reply.write(hex("05 00 03 10 00 00")),
			error: RequestFailedError,
			fields: { errorCode: 0x10n },
		},
		{
			name: "the peer finishes the stream without an answer",
			end: (stream) => stream.reply.close(),
			error: RequestAbortedError,
			fields: { abort: { type: "peer-closed" } },
		},
		{
			name: "the peer resets the stream",
			end: (stream) => stream.reply.abort(fakeStreamError(5, "reset")),
			error: RequestAbortedError,
			fields: { abort: { type: "peer-reset", code: 5 } },
		},
	])(
		"$name: rejects, and the session stays open",
		async ({ end, error, fields }) => {
			const { peer, connection } = await connected();
			const asking = connection.trackStatus(TRACK);
			const stream = await peer.clientBidi();
			await stream.sent.read();
			await end(stream);
			const outcome = await state(asking);
			expect(outcome).toBeInstanceOf(error);
			expect(outcome).toMatchObject(fields);
			await asksAgain(peer, connection);
		},
	);

	test("a parameter other than AUTHORIZATION_TOKEN is refused with a RangeError, and nothing is sent (§10.2.2)", async () => {
		const { peer, connection } = await connected();
		await expect(
			connection.trackStatus({
				...TRACK,
				parameters: { rendezvousTimeout: 1n },
			}),
		).rejects.toBeInstanceOf(RangeError);
		// No Request ID was used up.
		const asking = connection.trackStatus(TRACK);
		const stream = await peer.clientBidi();
		expect(await sentHex(stream)).toBe(TRACK_STATUS_0);
		await stream.reply.write(hex("07 00 01 00"));
		expect(await state(asking)).toBe("resolved");
	});

	test("the signal aborts before the answer: both directions end with CANCELLED, and the call rejects", async () => {
		const { peer, connection } = await connected();
		const controller = new AbortController();
		const asking = connection.trackStatus(TRACK, {
			signal: controller.signal,
		});
		const stream = await peer.clientBidi();
		await stream.sent.read();
		controller.abort();
		expect(await state(stream.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await state(stream.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		const outcome = await state(asking);
		expect(outcome).toBeInstanceOf(RequestAbortedError);
		expect(outcome).toMatchObject({ abort: { type: "cancelled" } });
		await asksAgain(peer, connection);
	});

	test("the signal aborts after the answer: nothing happens", async () => {
		const { peer, connection } = await connected();
		const controller = new AbortController();
		const asking = connection.trackStatus(TRACK, {
			signal: controller.signal,
		});
		const stream = await peer.clientBidi();
		await stream.sent.read();
		await stream.reply.write(hex("07 00 01 00"));
		expect(await state(asking)).toBe("resolved");
		controller.abort();
		await flush();
		// The FIN of §10.14, and no reset after it.
		expect(await stream.sent.read()).toEqual(END_OF_STREAM);
		expect(await state(stream.reply.closed)).toBe("pending");
		expect(await state(connection.closed)).toBe("pending");
	});

	test("with a signal that has aborted nothing is sent, and the call rejects with its reason", async () => {
		const { peer, connection } = await connected();
		const reason = new Error("too late");
		expect(
			await state(
				connection.trackStatus(TRACK, { signal: AbortSignal.abort(reason) }),
			),
		).toBe(reason);
		const asking = connection.trackStatus(TRACK);
		const stream = await peer.clientBidi();
		expect(await sentHex(stream)).toBe(TRACK_STATUS_0);
		await stream.reply.write(hex("07 00 01 00"));
		expect(await state(asking)).toBe("resolved");
	});

	test("the session closes before the answer: the call rejects with SessionClosedError", async () => {
		const { peer, connection } = await connected();
		const asking = connection.trackStatus(TRACK);
		const stream = await peer.clientBidi();
		await stream.sent.read();
		connection.close();
		const outcome = await state(asking);
		expect(outcome).toBeInstanceOf(SessionClosedError);
		expect(outcome).toMatchObject({ close: { type: "local" } });
	});
});
