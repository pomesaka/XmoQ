import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import {
	connectOver,
	type MoqtConnection,
} from "../../src/webtransport/connection.ts";
import {
	fixture,
	fixtureTable,
	fromALibrary,
	hex,
	MOQTAIL_RELAY_SETUP,
} from "../helpers.ts";
import { flush, pass, state } from "./clock.ts";
import { createFakePeer, type FakePeer } from "./fake-transport.ts";

// A request of the peer that was refused is kept until its stream is gone
// (test/core/session-refused-requests.test.ts). A peer that does not end
// its direction of that stream is stopped after `peerStreamEndTimeoutMs`,
// the time a subscriber has after PUBLISH_DONE. That no timer is left when
// the peer ends its direction in time is in first-bytes.test.ts, with the
// other timers of a stream of the peer.

/** The captured SUBSCRIBE with a Request ID of the server. */
function subscribeBytes(requestId: number): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	return hex(pairs.join(" "));
}

/** The captured Standalone FETCH with a Request ID of the server. */
function fetchBytes(requestId: number): Uint8Array {
	const pairs = fixture("fetch-standalone-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	return hex(pairs.join(" "));
}

/** A SUBSCRIBE_TRACKS as far as its Request ID, whole: a request that is not implemented. */
const subscribeTracks = (requestId: number): Uint8Array =>
	hex(`51 00 02 ${requestId.toString(16).padStart(2, "0")} 00`);

// The time is the default, 3 seconds, on a clock that is moved by hand.
beforeEach(() => {
	jest.useFakeTimers();
});
afterEach(() => {
	jest.useRealTimers();
});

async function connected(
	options: Parameters<typeof connectOver>[1] = {},
): Promise<{ peer: FakePeer; connection: MoqtConnection }> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport, options);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	await peer.clientUni();
	return { peer, connection };
}

/** The ways the request with the Request ID 1 is refused. */
const REFUSED: {
	name: string;
	serve?: (connection: MoqtConnection) => void;
	bytes: Uint8Array;
}[] = [
	{
		name: "a SUBSCRIBE the application rejects",
		serve: (connection) =>
			connection.serve({
				subscribe: (request) => void request.reject(0x10n),
			}),
		bytes: subscribeBytes(1),
	},
	{
		name: "a FETCH the application rejects",
		serve: (connection) =>
			connection.serve({ fetch: (request) => void request.reject(0x10n) }),
		bytes: fetchBytes(1),
	},
	{
		name: "a SUBSCRIBE that nothing serves",
		bytes: subscribeBytes(1),
	},
	{
		name: "a TRACK_STATUS, which the connection answers none of",
		bytes: hex(fixture("track-status-moqtail-relay")),
	},
	{
		name: "a request the session does not implement",
		bytes: subscribeTracks(1),
	},
	{
		name: "a request that was refused on its start and does not come whole",
		bytes: hex("51 00 05 01 00"),
	},
	{
		name: "a SUBSCRIBE that a REQUEST_UPDATE before its answer ended",
		serve: (connection) => connection.serve({ subscribe: () => {} }),
		bytes: new Uint8Array([...subscribeBytes(1), ...hex("02 00 02 03 00")]),
	},
];

// WHY NOT_SUPPORTED: the connection is a client, and answers what a
// SUBSCRIBE would get for no Track (§10.14), whatever it serves.
test("a TRACK_STATUS of the peer is refused with NOT_SUPPORTED, and a FIN", async () => {
	const { peer, connection } = await connected();
	connection.serve({ subscribe: () => {} });
	const request = peer.openBidi();
	await request.reply.write(hex(fixture("track-status-moqtail-relay")));
	await flush();
	const answer = (await request.sent.read()).value;
	expect(
		answer === undefined ? undefined : decodeControlMessage(answer),
	).toMatchObject({
		status: "ok",
		value: { type: "REQUEST_ERROR", errorCode: 0x3n },
	});
	expect(await request.sent.read()).toEqual({ done: true, value: undefined });
	expect(await state(connection.closed)).toBe("pending");
});

/** A captured request with the Request ID 1, the server's first. */
function withRequestIdOne(name: string): Uint8Array {
	const pairs = fixture(name).split(" ");
	pairs[3] = "01";
	return hex(pairs.join(" "));
}

/** The requests of `fixtures/client-refusals/refusals.tsv`, as the peer sends them. */
const REQUESTS: Record<string, Uint8Array> = {
	subscribe: subscribeBytes(1),
	publish: withRequestIdOne("publish-moqtail-rs"),
	"publish-namespace": withRequestIdOne("publish-namespace-moqtail-ts"),
	fetch: fetchBytes(1),
	// Request ID 1 already.
	"subscribe-namespace": hex(fromALibrary("subscribe-namespace-moqtail-ts")),
	"track-status": hex(fixture("track-status-moqtail-relay")),
	// SUBSCRIBE_TRACKS (§10.19), written by hand: Request ID 1, an empty
	// prefix, no parameters. Of it only the Request ID is read.
	"subscribe-tracks": hex("51 00 03 01 00 00"),
};

const CLIENT_REFUSALS = fixtureTable("client-refusals", "refusals.tsv");

test("the table of client refusals has its rows", () => {
	expect(CLIENT_REFUSALS.length).toBe(13);
});

// The rows of `fixtures/client-refusals/refusals.tsv`, which the Rust client
// reads as well (`refusal_code`).
test.each(CLIENT_REFUSALS)(
	"a request %s of the peer, served %s, is refused with %s",
	async (name, serves, code) => {
		const { peer, connection } = await connected();
		switch (serves) {
			case "yes":
				connection.serve({ subscribe: () => {} });
				break;
			case "no":
				break;
			default:
				throw new Error(`no serves ${serves}`);
		}
		const bytes = REQUESTS[name ?? ""];
		if (bytes === undefined) throw new Error(`no request ${name}`);
		const request = peer.openBidi();
		await request.reply.write(bytes);
		await flush();
		const answer = (await request.sent.read()).value;
		expect(
			answer === undefined ? undefined : decodeControlMessage(answer),
		).toMatchObject({
			status: "ok",
			value: { type: "REQUEST_ERROR", errorCode: BigInt(code ?? "no code") },
		});
		expect(await state(connection.closed)).toBe("pending");
	},
);

describe("a request of the peer that was refused, whose peer does not end its direction", () => {
	test.each(REFUSED)(
		"$name: the peer's direction is stopped (CANCELLED) when the time is over, and not before",
		async ({ serve, bytes }) => {
			const { peer, connection } = await connected();
			serve?.(connection);
			const request = peer.openBidi();
			await request.reply.write(bytes);
			await flush();
			// REQUEST_ERROR, and the end of this endpoint's direction.
			expect((await request.sent.read()).value?.[0]).toBe(0x05);
			expect(await request.sent.read()).toEqual({
				done: true,
				value: undefined,
			});
			await pass(2_999);
			expect(await state(request.reply.closed)).toBe("pending");
			await pass(1);
			expect(await state(request.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(connection.closed)).toBe("pending");
		},
	);

	test.each(REFUSED)(
		"$name: its place among the peer's streams is free again when the time is over",
		async ({ serve, bytes }) => {
			const { peer, connection } = await connected({ maxPeerBidiStreams: 1 });
			serve?.(connection);
			const request = peer.openBidi();
			await request.reply.write(bytes);
			await flush();
			// A second request waits in the transport.
			const second = peer.openBidi();
			await second.reply.write(subscribeTracks(5));
			await pass(2_999);
			expect(peer.untakenBidiStreams()).toBe(1);
			await pass(1);
			expect(peer.untakenBidiStreams()).toBe(0);
			expect((await second.sent.read()).value?.[0]).toBe(0x05);
		},
	);

	test("a request that is not kept, for as many being kept as may be, is stopped as well", async () => {
		const { peer } = await connected({ limits: { maxPeerRequests: 1 } });
		const first = peer.openBidi();
		await first.reply.write(subscribeTracks(1));
		const second = peer.openBidi();
		await second.reply.write(subscribeTracks(3));
		await pass(2_999);
		expect(await state(second.reply.closed)).toBe("pending");
		await pass(1);
		for (const request of [first, second]) {
			expect(await state(request.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
		}
	});

	test("a request that is not kept leaves no timer when its peer has ended its direction", async () => {
		const { peer } = await connected({ limits: { maxPeerRequests: 1 } });
		const first = peer.openBidi();
		await first.reply.write(subscribeTracks(1));
		const second = peer.openBidi();
		await second.reply.write(subscribeTracks(3));
		await flush();
		expect(jest.getTimerCount()).toBe(2);
		await second.reply.close();
		await flush();
		expect(jest.getTimerCount()).toBe(1);
		await first.reply.close();
		await flush();
		expect(jest.getTimerCount()).toBe(0);
	});
});
