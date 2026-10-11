import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import {
	connectOver,
	type MoqtConnection,
	type SessionConnectOptions,
} from "../../src/webtransport/connection.ts";
import { fixture, hex, MOQTAIL_RELAY_SETUP } from "../helpers.ts";
import { flush, pass, state } from "./clock.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
} from "./fake-transport.ts";

async function connected(
	options: SessionConnectOptions = {},
	holdBidi?: Promise<void>,
): Promise<{ peer: FakePeer; connection: MoqtConnection }> {
	const peer = createFakePeer({ holdBidi });
	const connecting = connectOver(peer.transport, options);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

/** Whether the session is still open. */
async function open(connection: MoqtConnection): Promise<boolean> {
	return (await state(connection.closed)) === "pending";
}

beforeEach(() => {
	jest.useFakeTimers();
});
afterEach(() => {
	jest.useRealTimers();
});

describe("a stream of the peer that does not say what it is", () => {
	// Without the option the time is 10 seconds, as in the Rust
	// implementation. The codes are the Rust reader's: INTERNAL_ERROR (0x0)
	// for a unidirectional stream, CANCELLED (0x1) for a request.
	test.each([
		{ name: "its type alone", bytes: "14" },
		{ name: "its type and half of its Subgroup Header", bytes: "14 09" },
		{ name: "the type of a fetch stream", bytes: "05" },
		{ name: "half of its type", bytes: "80" },
	])(
		"a unidirectional stream that sent $name is stopped after 10 seconds, and the session stays",
		async ({ bytes }) => {
			const { peer, connection } = await connected();
			const stream = peer.openUni();
			await stream.write(hex(bytes));
			await pass(9_999);
			expect(await state(stream.closed)).toBe("pending");
			await pass(1);
			expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
			expect(await open(connection)).toBe(true);
		},
	);

	test.each([
		{ name: "half of its Request ID", bytes: "0d 00 05 80" },
		{ name: "a FETCH that is not whole", bytes: "16 00 05 01 02" },
	])(
		"a request that sent $name is given up after 10 seconds, in both directions",
		async ({ bytes }) => {
			const { peer, connection } = await connected();
			const request = peer.openBidi();
			await request.reply.write(hex(bytes));
			await pass(9_999);
			expect(await state(request.reply.closed)).toBe("pending");
			await pass(1);
			// STOP_SENDING for the peer's direction, and a reset of this one's.
			expect(await state(request.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await open(connection)).toBe(true);
		},
	);

	// The time is the one a peer has to end its direction after a refusal
	// (3 seconds), which is over before the one for the first bytes.
	test("a request that was refused on its Request ID and never comes whole is stopped, after its answer", async () => {
		const { peer, connection } = await connected();
		const request = peer.openBidi();
		// A TRACK_STATUS of 5 bytes of payload, of which two came.
		await request.reply.write(hex("0d 00 05 01 00"));
		await flush();
		// REQUEST_ERROR, and the end of this endpoint's direction.
		expect((await request.sent.read()).value?.[0]).toBe(0x05);
		expect(await request.sent.read()).toEqual({ done: true, value: undefined });
		await pass(2_999);
		expect(await state(request.reply.closed)).toBe("pending");
		await pass(1);
		expect(await state(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		// The time for the first bytes finds nothing left to stop.
		await pass(3_600_000);
		expect(await open(connection)).toBe(true);
	});

	// "Not stopped" is looked at long after the time, so that a clock that
	// is a little late does not pass for one that was stopped.
	test.each([
		{
			name: "a subgroup stream with its whole header",
			bytes: "14 09 00 00 00",
		},
		{ name: "a padding stream", bytes: "f0 13 2b 3e 28" },
	])("$name has said what it is and stays open", async ({ bytes }) => {
		const { peer, connection } = await connected();
		// A SUBSCRIBE without an answer yet, so that the Track Alias of the
		// subgroup stream can still be established and its stream is held.
		void connection.subscribe({ trackNamespace: [], trackName: hex("61") });
		const stream = peer.openUni();
		await stream.write(hex(bytes));
		await pass(3_600_000);
		expect(await state(stream.closed)).toBe("pending");
		expect(await open(connection)).toBe(true);
	});

	test("a request that came whole and was refused keeps its stream for the time its peer has to end its direction, and no longer", async () => {
		const { peer, connection } = await connected();
		const request = peer.openBidi();
		await request.reply.write(hex("0d 00 02 01 00"));
		await pass(2_999);
		expect(await state(request.reply.closed)).toBe("pending");
		await pass(1);
		expect(await state(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await open(connection)).toBe(true);
	});

	// The stop is an output, and outputs are performed in order: one that
	// waits, here a request whose stream the peer gives no credit for, holds
	// the stop back, and the stream is read on meanwhile. What arrives then
	// is what the peer would have sent anyway, and must not be read as a
	// stream or a request from its start, which would close the session.
	test.each([
		{
			name: "a subgroup stream, the rest of its header",
			kind: "uni",
			before: "5d 00",
			rest: "00 00 00 00 01 61",
		},
		{
			name: "a request, the rest of its first message",
			kind: "bidi",
			before: "03 00 1f",
			rest: "01 02 04 78 6d 6f 71",
		},
	] as const)(
		"what a stopped stream still sends while the stop waits to be performed is dropped: $name",
		async ({ kind, before, rest }) => {
			let release: () => void = () => {};
			const held = new Promise<void>((resolve) => {
				release = resolve;
			});
			const { peer, connection } = await connected({}, held);
			const writer = kind === "uni" ? peer.openUni() : peer.openBidi().reply;
			await writer.write(hex(before));
			await flush();
			// Only now the request that keeps every later output waiting: the
			// stream is being read, and goes on being read behind it.
			void connection.publishNamespace([hex("61")]).catch(() => {});
			await pass(10_000);
			// The stop has not reached the stream: the peer can still write.
			await writer.write(hex(rest));
			await flush();
			expect(await open(connection)).toBe(true);
			release();
			await flush();
			expect(await state(writer.closed)).toMatchObject({
				streamErrorCode: kind === "uni" ? 0 : 1,
			});
			expect(await open(connection)).toBe(true);
		},
	);

	// The session learns of a stream when it is taken, not with its first
	// byte: one that sends nothing at all is stopped like the others.
	test("streams that send no byte at all are stopped after 10 seconds", async () => {
		const { peer, connection } = await connected();
		const stream = peer.openUni();
		const request = peer.openBidi();
		await pass(9_999);
		expect(await state(stream.closed)).toBe("pending");
		expect(await state(request.reply.closed)).toBe("pending");
		await pass(1);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
		expect(await state(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await open(connection)).toBe(true);
	});

	test("the peer's control stream is never stopped", async () => {
		const { connection } = await connected();
		await pass(3_600_000);
		expect(await open(connection)).toBe(true);
	});

	// As in the Rust implementation: the time runs from when a stream is
	// read, and one that waits in the transport for a place has none yet.
	test("the time of a stream starts when it is taken from the transport, not when the peer opened it", async () => {
		const { peer, connection } = await connected({ maxPeerBidiStreams: 1 });
		const first = peer.openBidi();
		const second = peer.openBidi();
		await first.reply.write(hex("0d"));
		await second.reply.write(hex("0d"));
		await flush();
		expect(peer.untakenBidiStreams()).toBe(1);

		await pass(10_000);
		expect(await state(first.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		// The place is free, and the second is read from now on.
		expect(peer.untakenBidiStreams()).toBe(0);
		await pass(9_999);
		expect(await state(second.reply.closed)).toBe("pending");
		await pass(1);
		expect(await state(second.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await open(connection)).toBe(true);
	});

	test("the place of a stream that was stopped is free for the next one", async () => {
		// Two places: the peer's control stream and one more.
		const { peer } = await connected({ maxPeerUniStreams: 2 });
		const silent = peer.openUni();
		await silent.write(hex("14"));
		const next = peer.openUni();
		await next.write(hex("14"));
		await flush();
		expect(peer.untakenUniStreams()).toBe(1);
		await pass(10_000);
		expect(peer.untakenUniStreams()).toBe(0);
	});

	test("`firstBytesTimeoutMs` sets the time", async () => {
		const { peer } = await connected({ firstBytesTimeoutMs: 250 });
		const stream = peer.openUni();
		await stream.write(hex("14"));
		await pass(249);
		expect(await state(stream.closed)).toBe("pending");
		await pass(1);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
	});

	// A timer that is left behind fires for a stream that is gone. The
	// session does nothing then, so only the count of timers shows it.
	test.each<{
		name: string;
		act: (peer: FakePeer, connection: MoqtConnection) => Promise<void>;
	}>([
		{
			name: "a unidirectional stream has said what it is",
			act: async (peer) => {
				await peer.openUni().write(hex("f0 13 2b 3e 28"));
			},
		},
		{
			name: "a request has come whole and its peer has ended its direction",
			act: async (peer) => {
				const request = peer.openBidi();
				await request.reply.write(hex("0d 00 02 01 00"));
				await flush();
				await request.reply.close();
			},
		},
		{
			name: "a request was refused and its peer has reset its direction",
			act: async (peer) => {
				const request = peer.openBidi();
				await request.reply.write(hex("0d 00 02 01 00"));
				await flush();
				await request.reply.abort(fakeStreamError(1, "reset"));
			},
		},
		{
			name: "a SUBSCRIBE was rejected by the application and its peer has ended its direction",
			act: async (peer, connection) => {
				connection.serve({
					subscribe: (request) => void request.reject(0x10n),
				});
				const request = peer.openBidi();
				// The captured SUBSCRIBE, with a Request ID of the server.
				const pairs = fixture("subscribe-moqtail-rs").split(" ");
				pairs[3] = "01";
				await request.reply.write(hex(pairs.join(" ")));
				await flush();
				await request.reply.close();
			},
		},
		{
			name: "a request was refused and the session has ended",
			act: async (peer, connection) => {
				await peer.openBidi().reply.write(hex("0d 00 02 01 00"));
				await flush();
				connection.close();
			},
		},
		{
			name: "a unidirectional stream has ended without a byte",
			act: (peer) => peer.openUni().close(),
		},
		{
			name: "a unidirectional stream was reset inside its type",
			act: async (peer) => {
				const stream = peer.openUni();
				await stream.write(hex("80"));
				await flush();
				await stream.abort(fakeStreamError(1, "reset"));
			},
		},
		{
			name: "a request has ended without a byte",
			act: (peer) => peer.openBidi().reply.close(),
		},
		{
			name: "a request was reset inside its first message",
			act: async (peer) => {
				const request = peer.openBidi();
				await request.reply.write(hex("0d"));
				await flush();
				await request.reply.abort(fakeStreamError(1, "reset"));
			},
		},
	])("no timer is left when $name", async ({ act }) => {
		const { peer, connection } = await connected();
		await flush();
		expect(jest.getTimerCount()).toBe(0);
		await act(peer, connection);
		await flush();
		expect(jest.getTimerCount()).toBe(0);
	});

	// Zero is a time like any other and not "never": the stream is stopped
	// by the next timer, before it can say anything in a later read.
	test("with `firstBytesTimeoutMs` 0 a stream that has not said what it is in its first read is stopped at once", async () => {
		const { peer, connection } = await connected({ firstBytesTimeoutMs: 0 });
		const stream = peer.openUni();
		await stream.write(hex("14"));
		await pass(0);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
		expect(await open(connection)).toBe(true);
	});

	test("a session that ends takes its timers with it", async () => {
		const { peer, connection } = await connected();
		const stream = peer.openUni();
		await stream.write(hex("14"));
		connection.close();
		await flush();
		expect(jest.getTimerCount()).toBe(0);
	});

	test("the time that ran out is reported to the diagnostics, and to nothing else", async () => {
		const events: string[] = [];
		const { peer } = await connected({
			diagnostics: {
				onEvent: (event) => {
					if (event.type === "peer-stream-timed-out") {
						events.push(`${event.kind} ${event.peerStream}`);
					}
				},
			},
		});
		await peer.openUni().write(hex("14"));
		await peer.openBidi().reply.write(hex("0d"));
		await pass(10_000);
		// The streams of the peer are numbered as they are taken, of whatever
		// kind: its control stream is 0.
		expect(events.sort()).toEqual(["bidi 2", "uni 1"]);
	});
});
