import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	connectOver,
	type MoqtConnection,
	type SessionConnectOptions,
} from "../../src/webtransport/connection.ts";
import { RequestAbortedError } from "../../src/webtransport/endings.ts";
import { hex, MOQTAIL_RELAY_SETUP } from "../helpers.ts";
import { pass, state } from "./clock.ts";
import { createFakePeer, type FakePeer } from "./fake-transport.ts";

const TRACK = { trackNamespace: [hex("61")], trackName: hex("62") };

async function connected(
	options: SessionConnectOptions = {},
	hold: {
		holdBidi?: Promise<void>;
		holdBidiFrom?: number;
		failBidi?: () => unknown;
	} = {},
): Promise<{ peer: FakePeer; connection: MoqtConnection }> {
	const peer = createFakePeer(hold);
	const connecting = connectOver(peer.transport, options);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

/** `fixtures/request-answer-wait/`, which the Rust client reads too: what a PUBLISH nobody answered is ended with. */
const UNANSWERED_PUBLISH_DONE = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"..",
		"fixtures",
		"request-answer-wait",
		"publish-done-of-an-unanswered-publish.hex",
	),
	"utf8",
).trim();

const SUBSCRIBE_OK_ALIAS_9 = "04 00 02 09 00";
/** FETCH_OK: End Of Track 0, End Location {3, 0}, no parameters, no Track Properties. */
const FETCH_OK = "18 00 04 00 03 00 00";
const RANGE = {
	fetchType: "Standalone",
	...TRACK,
	startLocation: { group: 0n, object: 0n },
	endLocation: { group: 1n, object: 0n },
} as const;

beforeEach(() => {
	jest.useFakeTimers();
});
afterEach(() => {
	jest.useRealTimers();
});

describe("a request the peer does not answer", () => {
	test.each<{
		name: string;
		options: SessionConnectOptions;
		ask: (connection: MoqtConnection) => Promise<unknown>;
		waitMs: number;
	}>([
		{
			name: "SUBSCRIBE",
			options: {},
			ask: (connection) => connection.subscribe(TRACK),
			waitMs: 10_000,
		},
		{
			name: "SUBSCRIBE with a RENDEZVOUS_TIMEOUT of 3 seconds",
			options: {},
			ask: (connection) =>
				connection.subscribe({
					...TRACK,
					parameters: { rendezvousTimeout: 3000n },
				}),
			waitMs: 13_000,
		},
		{
			name: "PUBLISH_NAMESPACE",
			options: {},
			ask: (connection) => connection.publishNamespace(TRACK.trackNamespace),
			waitMs: 10_000,
		},
		{
			name: "SUBSCRIBE with a wait that is set",
			options: { answerTimeoutMs: 1500 },
			ask: (connection) => connection.subscribe(TRACK),
			waitMs: 1500,
		},
	])(
		"$name is given up after $waitMs ms, both directions of its stream end with CANCELLED, and the session stays open",
		async ({ options, ask, waitMs }) => {
			const { peer, connection } = await connected(options);
			const asking = ask(connection);
			const request = await peer.clientBidi();
			await request.sent.read();
			await pass(waitMs - 1);
			expect(await state(asking)).toBe("pending");
			await pass(1);
			const error = await state(asking);
			expect(error).toBeInstanceOf(RequestAbortedError);
			expect(error).toMatchObject({ abort: { type: "no-answer-in-time" } });
			// §5.1, §6.2: the reset, and STOP_SENDING for the peer's direction.
			expect(await state(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(request.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(connection.closed)).toBe("pending");

			// The session goes on: a request after it is answered.
			const announcing = connection.publishNamespace([hex("63")]);
			const next = await peer.clientBidi();
			await next.sent.read();
			await next.reply.write(hex("07 00 01 00"));
			expect(await state(announcing)).toBe("resolved");
		},
	);

	test.each<{ name: string; fillTimeout: bigint | undefined; waitMs: number }>([
		{ name: "FETCH", fillTimeout: undefined, waitMs: 10_000 },
		{
			name: "FETCH with a FILL_TIMEOUT of 20 seconds",
			fillTimeout: 20_000n,
			waitMs: 30_000,
		},
	])(
		"$name is given up after $waitMs ms: `response` and `ended` say so, and the session stays open",
		async ({ fillTimeout, waitMs }) => {
			const { peer, connection } = await connected();
			const fetch = await connection.fetch({
				range: {
					fetchType: "Standalone",
					...TRACK,
					startLocation: { group: 0n, object: 0n },
					endLocation: { group: 1n, object: 0n },
				},
				parameters: fillTimeout === undefined ? {} : { fillTimeout },
			});
			const request = await peer.clientBidi();
			await request.sent.read();
			await pass(waitMs - 1);
			expect(await state(fetch.ended)).toBe("pending");
			await pass(1);
			expect(await fetch.ended).toEqual({ type: "no-answer-in-time" });
			const error = await state(fetch.response);
			expect(error).toBeInstanceOf(RequestAbortedError);
			expect(error).toMatchObject({ abort: { type: "no-answer-in-time" } });
			// §5.2: "It MUST send STOP_SENDING for the bidi request stream".
			expect(await state(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(request.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(connection.closed)).toBe("pending");
		},
	);

	test("PUBLISH is ended after 10000 ms with PUBLISH_DONE and a FIN, and the subscriber's direction is stopped 3 seconds later", async () => {
		const { peer, connection } = await connected();
		const publishing = connection.publish({ ...TRACK, trackAlias: 1n });
		const request = await peer.clientBidi();
		await request.sent.read();
		await pass(9_999);
		expect(await state(publishing)).toBe("pending");
		await pass(1);
		const error = await state(publishing);
		expect(error).toBeInstanceOf(RequestAbortedError);
		expect(error).toMatchObject({ abort: { type: "no-answer-in-time" } });
		// PUBLISH_DONE: INTERNAL_ERROR (0x0), Stream Count 0, "no answer in time"
		// (`0b 00 14 00 00 11` and the 17 bytes of the reason).
		expect(await request.sent.read()).toEqual({
			done: false,
			value: hex(UNANSWERED_PUBLISH_DONE),
		});
		expect(await request.sent.read()).toEqual({ done: true, value: undefined });
		// Not stopped at once: the subscriber gets the time `done()` gives it.
		await pass(2_999);
		expect(await state(request.reply.closed)).toBe("pending");
		await pass(1);
		expect(await state(request.reply.closed)).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await state(connection.closed)).toBe("pending");
	});

	test.each<{
		name: string;
		ask: (connection: MoqtConnection) => Promise<unknown>;
	}>([
		{ name: "SUBSCRIBE", ask: (connection) => connection.subscribe(TRACK) },
		{
			name: "PUBLISH",
			ask: (connection) => connection.publish({ ...TRACK, trackAlias: 1n }),
		},
		{ name: "FETCH", ask: (connection) => connection.fetch({ range: RANGE }) },
	])(
		"$name that waits for a stream the peer does not allow fails after 10000 ms as not sent, and the session stays open",
		async ({ ask }) => {
			let release = (): void => {};
			const holdBidi = new Promise<void>((resolve) => {
				release = resolve;
			});
			const { peer, connection } = await connected({}, { holdBidi });
			const asking = ask(connection);
			await pass(9_999);
			expect(await state(asking)).toBe("pending");
			await pass(1);
			const error = await state(asking);
			expect(error).toBeInstanceOf(RequestAbortedError);
			expect(error).toMatchObject({ abort: { type: "not-sent-in-time" } });
			expect(await state(connection.closed)).toBe("pending");
			// The stream comes after all: nothing is written on it, and both
			// directions are given up with CANCELLED.
			release();
			const late = await peer.clientBidi();
			expect(await state(late.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(late.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(connection.closed)).toBe("pending");
		},
	);

	// What Chromium does where the peer allows no further stream: the call
	// is refused at once, and the request asks again.
	test.each<{ name: string; refusedForMs: number; outcome: string }>([
		{
			name: "allowed after 3 seconds: the request is sent then",
			refusedForMs: 3_000,
			outcome: "sent",
		},
		{
			name: "never allowed: it fails after 10000 ms as not sent",
			refusedForMs: 3_600_000,
			outcome: "not-sent-in-time",
		},
	])(
		"a request whose stream the transport refuses to make is asked for again, $name",
		async ({ refusedForMs, outcome }) => {
			let refused = true;
			const { peer, connection } = await connected(
				{},
				{
					failBidi: () =>
						refused
							? new DOMException(
									"Failed to create bidirectional stream.",
									"NetworkError",
								)
							: undefined,
				},
			);
			const asking = connection.subscribe(TRACK);
			setTimeout(() => {
				refused = false;
			}, refusedForMs);
			await pass(2_999);
			expect(await state(asking)).toBe("pending");
			expect(await state(connection.closed)).toBe("pending");
			await pass(7_001);
			if (outcome === "sent") {
				const request = await peer.clientBidi();
				await request.sent.read();
				await request.reply.write(hex(SUBSCRIBE_OK_ALIAS_9));
				expect(await state(asking)).toBe("resolved");
			} else {
				expect(await state(asking)).toMatchObject({
					abort: { type: outcome },
				});
			}
			expect(await state(connection.closed)).toBe("pending");
		},
	);

	test("a wait for a stream that is set", async () => {
		const { connection } = await connected(
			{ openStreamTimeoutMs: 1500 },
			{ holdBidi: new Promise<void>(() => {}) },
		);
		const asking = connection.subscribe(TRACK);
		await pass(1_499);
		expect(await state(asking)).toBe("pending");
		await pass(1);
		expect(await state(asking)).toMatchObject({
			abort: { type: "not-sent-in-time" },
		});
	});

	test("a request whose stream came in time has its 10 seconds for the answer from when it was written", async () => {
		let release = (): void => {};
		const holdBidi = new Promise<void>((resolve) => {
			release = resolve;
		});
		const { peer, connection } = await connected({}, { holdBidi });
		const asking = connection.subscribe(TRACK);
		await pass(9_000);
		release();
		const request = await peer.clientBidi();
		await request.sent.read();
		await pass(9_999);
		expect(await state(asking)).toBe("pending");
		await pass(1);
		expect(await state(asking)).toMatchObject({
			abort: { type: "no-answer-in-time" },
		});
	});

	// A request that waits for a stream holds nothing else back: neither an
	// answer of the peer nor a request that is sent after it on a stream
	// that is there.
	test("an answer is handed on while another request waits for its stream", async () => {
		const { peer, connection } = await connected(
			{},
			{ holdBidi: new Promise<void>(() => {}), holdBidiFrom: 1 },
		);
		const first = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		await request.sent.read();
		const second = connection.publishNamespace([hex("63")]);
		await pass(1_000);
		await request.reply.write(hex(SUBSCRIBE_OK_ALIAS_9));
		expect(await state(first)).toBe("resolved");
		expect(await state(second)).toBe("pending");
		expect(await state(connection.closed)).toBe("pending");
		void second.catch(() => {});
	});

	test("a FETCH that was answered is not ended by the time: its body goes on after it", async () => {
		const { peer, connection } = await connected();
		const fetch = await connection.fetch({ range: RANGE });
		const request = await peer.clientBidi();
		await request.sent.read();
		await pass(9_999);
		await request.reply.write(hex(FETCH_OK));
		const stream = peer.openUni();
		await stream.write(hex("05 00"));
		await stream.write(hex("aa"));
		await pass(3_600_000);
		expect(await state(fetch.ended)).toBe("pending");
		const reader = fetch.body.getReader();
		expect(await reader.read()).toEqual({ done: false, value: hex("aa") });
		await stream.write(hex("bb"));
		expect(await reader.read()).toEqual({ done: false, value: hex("bb") });
		expect(await state(connection.closed)).toBe("pending");
	});

	test("an answer in time ends the wait: nothing happens when the time is over", async () => {
		const { peer, connection } = await connected();
		const announcing = connection.publishNamespace(TRACK.trackNamespace);
		const request = await peer.clientBidi();
		await request.sent.read();
		await pass(9_999);
		await request.reply.write(hex("07 00 01 00"));
		expect(await state(announcing)).toBe("resolved");
		await pass(3_600_000);
		expect(await state(request.sent.read())).toBe("pending");
		expect(await state(connection.closed)).toBe("pending");
	});
});
