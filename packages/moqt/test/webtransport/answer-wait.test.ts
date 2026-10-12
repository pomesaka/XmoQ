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
	hold: { holdBidi?: Promise<void>; holdBidiFrom?: number } = {},
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

	test("the time begins when the request is written: one that waits for a stream is not given up, and has its 10 seconds from then", async () => {
		let release = (): void => {};
		const holdBidi = new Promise<void>((resolve) => {
			release = resolve;
		});
		const { peer, connection } = await connected({}, { holdBidi });
		const asking = connection.subscribe(TRACK);
		await pass(3_600_000);
		expect(await state(asking)).toBe("pending");
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

	// Outputs are performed in one line: the event of an answer waits there
	// behind a request whose stream the peer gives no credit for.
	test("an answer that came is not timed out while another request waits for its stream", async () => {
		let release = (): void => {};
		const holdBidi = new Promise<void>((resolve) => {
			release = resolve;
		});
		const { peer, connection } = await connected(
			{},
			{ holdBidi, holdBidiFrom: 1 },
		);
		const first = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		await request.sent.read();
		// It waits for a stream, and everything behind it with it.
		const second = connection.publishNamespace([hex("63")]);
		await pass(1_000);
		await request.reply.write(hex(SUBSCRIBE_OK_ALIAS_9));
		await pass(3_600_000);
		expect(await state(first)).toBe("pending");
		expect(await state(request.sent.read())).toBe("pending");
		release();
		expect(await state(first)).toBe("resolved");
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
