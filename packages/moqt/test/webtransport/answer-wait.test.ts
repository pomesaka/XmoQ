import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
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
): Promise<{ peer: FakePeer; connection: MoqtConnection }> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport, options);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

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
			name: "PUBLISH",
			options: {},
			ask: (connection) => connection.publish({ ...TRACK, trackAlias: 1n }),
			waitMs: 10_000,
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
		"$name is given up after $waitMs ms, its stream is reset with CANCELLED, and the session stays open",
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
			expect(await state(request.sent.read())).toMatchObject({
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
			expect(await state(request.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(connection.closed)).toBe("pending");
		},
	);

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
