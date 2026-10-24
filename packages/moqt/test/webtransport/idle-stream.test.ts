import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import {
	connectOver,
	type MoqtConnection,
	type SessionConnectOptions,
	type Subscription,
} from "../../src/webtransport/connection.ts";
import { hex, MOQTAIL_RELAY_SETUP, utf8 } from "../helpers.ts";
import { flush, pass, state } from "./clock.ts";
import { createFakePeer, type FakePeer } from "./fake-transport.ts";

// A subgroup stream the peer keeps open without bytes is stopped after 30
// seconds with STOP_SENDING INTERNAL_ERROR (0x0), counted from its last
// bytes while it is read; the session and its subscription go on.

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

/** SUBSCRIBE_OK with Track Alias `alias`, as in connection.test.ts. */
const subscribeOk = (alias: string): string => `04 00 02 ${alias} 00`;
/** Subgroup Header, Explicit Subgroup ID: Track Alias, Group 0, Subgroup 0, Publisher Priority 0. */
const header = (alias: string): string => `14 ${alias} 00 00 00`;
/** One Object with a one-byte payload, in the delta encoding of a sequential stream. */
const OBJECT = "00 01 61";

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

/** Subscribes, and has the peer accept with Track Alias `alias`. */
async function subscribe(
	peer: FakePeer,
	connection: MoqtConnection,
	alias: string,
): Promise<Subscription> {
	const subscribing = connection.subscribe(TRACK);
	const request = await peer.clientBidi();
	await request.sent.read();
	await request.reply.write(hex(subscribeOk(alias)));
	return subscribing;
}

async function open(connection: MoqtConnection): Promise<boolean> {
	return (await state(connection.closed)) === "pending";
}

beforeEach(() => {
	jest.useFakeTimers();
});
afterEach(() => {
	jest.useRealTimers();
});

describe("a subgroup stream of the peer that sends nothing", () => {
	test.each([
		{ name: "its header", bytes: header("09") },
		{ name: "its header and an Object", bytes: `${header("09")} ${OBJECT}` },
	])(
		"one that sent $name and then nothing is stopped with INTERNAL_ERROR after 30 seconds, and the session and the subscription stay",
		async ({ bytes }) => {
			const abandoned: string[] = [];
			const { peer, connection } = await connected({
				diagnostics: {
					onEvent: (event) => {
						if (event.type === "subgroup-abandoned") {
							abandoned.push(event.reason);
						}
					},
				},
			});
			const subscription = await subscribe(peer, connection, "09");
			const stream = peer.openUni();
			await stream.write(hex(bytes));
			await pass(29_999);
			expect(await state(stream.closed)).toBe("pending");
			await pass(1);
			expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
			expect(abandoned).toEqual(["no bytes within the idle time"]);
			expect(await state(subscription.ended)).toBe("pending");
			expect(await open(connection)).toBe(true);
		},
	);

	test("bytes before the 30 seconds are over keep it, and the time counts again from them", async () => {
		const { peer, connection } = await connected();
		await subscribe(peer, connection, "09");
		const stream = peer.openUni();
		await stream.write(hex(header("09")));
		await pass(29_999);
		await stream.write(hex(OBJECT));
		await pass(29_999);
		expect(await state(stream.closed)).toBe("pending");
		await pass(1);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
	});

	test.each([
		{ timeout: 250, before: 249 },
		{ timeout: 120_000, before: 119_999 },
	])(
		"`idleSubgroupStreamTimeoutMs` $timeout sets the time",
		async ({ timeout, before }) => {
			const { peer, connection } = await connected({
				idleSubgroupStreamTimeoutMs: timeout,
			});
			await subscribe(peer, connection, "09");
			const stream = peer.openUni();
			await stream.write(hex(header("09")));
			await pass(before);
			expect(await state(stream.closed)).toBe("pending");
			await pass(1);
			expect(await state(stream.closed)).toMatchObject({
				streamErrorCode: 0,
			});
		},
	);

	test.each([
		{
			value: 0,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not 0",
		},
		{
			value: -1,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not -1",
		},
		{
			value: 1.5,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not 1.5",
		},
		{
			value: Number.NaN,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not NaN",
		},
		{
			value: Number.POSITIVE_INFINITY,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not Infinity",
		},
		{
			value: 2_147_483_648,
			message:
				"idleSubgroupStreamTimeoutMs is a whole number from 1 to 2147483647, not 2147483648",
		},
	])(
		"`idleSubgroupStreamTimeoutMs` $value is a RangeError",
		async ({ value, message }) => {
			const peer = createFakePeer();
			await expect(
				connectOver(peer.transport, { idleSubgroupStreamTimeoutMs: value }),
			).rejects.toThrow(new RangeError(message));
		},
	);

	test("the other streams of the subscription and another subscription go on", async () => {
		const { peer, connection } = await connected();
		const first = await subscribe(peer, connection, "09");
		const second = await subscribe(peer, connection, "0a");
		const silent = peer.openUni();
		await silent.write(hex(header("09")));
		const sending = peer.openUni();
		await sending.write(hex(header("0a")));
		await pass(20_000);
		await sending.write(hex(OBJECT));
		await pass(10_000);
		expect(await state(silent.closed)).toMatchObject({ streamErrorCode: 0 });
		expect(await state(sending.closed)).toBe("pending");

		// The first subscription reads a stream that comes after.
		const later = peer.openUni();
		await later.write(hex(`${header("09")} ${OBJECT}`));
		await flush();
		const firstObject = await first.objects.getReader().read();
		expect(firstObject.value?.object.payload).toEqual(utf8("a"));
		const secondObject = await second.objects.getReader().read();
		expect(secondObject.value?.object.payload).toEqual(utf8("a"));
		expect(await state(first.ended)).toBe("pending");
		expect(await state(second.ended)).toBe("pending");
		expect(await open(connection)).toBe(true);
	});

	test("what it held before its SUBSCRIBE_OK is dropped with it", async () => {
		// WHY the longest wait for an answer: the SUBSCRIBE has to stay
		// unanswered for longer than the idle time.
		const { peer, connection } = await connected({
			answerTimeoutMs: 2_147_483_647,
		});
		const subscribing = connection.subscribe(TRACK);
		const request = await peer.clientBidi();
		await request.sent.read();
		const stream = peer.openUni();
		await stream.write(hex(`${header("09")} ${OBJECT}`));
		await pass(30_000);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });

		await request.reply.write(hex(subscribeOk("09")));
		const subscription = await subscribing;
		expect(await state(subscription.objects.getReader().read())).toBe(
			"pending",
		);
	});

	// The time runs while a read waits for the peer, not while the stream
	// waits for this endpoint.
	test("a stream held back by a subscription whose queue is full is not stopped, and has the time again once it is read", async () => {
		const { peer, connection } = await connected({
			subscriptionHighWaterMark: 1,
		});
		const subscription = await subscribe(peer, connection, "09");
		const stream = peer.openUni();
		// Two Objects fill a queue of one: the stream's reader waits.
		await stream.write(hex(`${header("09")} ${OBJECT} ${OBJECT}`));
		await pass(3_600_000);
		expect(await state(stream.closed)).toBe("pending");

		const reader = subscription.objects.getReader();
		await reader.read();
		await reader.read();
		await pass(29_999);
		expect(await state(stream.closed)).toBe("pending");
		await pass(1);
		expect(await state(stream.closed)).toMatchObject({ streamErrorCode: 0 });
	});

	test.each<{ name: string; end: "fin" | "idle" }>([
		{ name: "ends with a FIN", end: "fin" },
		{ name: "is stopped for being idle", end: "idle" },
	])("no timer is left when a subgroup stream $name", async ({ end }) => {
		const { peer, connection } = await connected();
		await subscribe(peer, connection, "09");
		await flush();
		const before = jest.getTimerCount();
		const stream = peer.openUni();
		await stream.write(hex(header("09")));
		await flush();
		if (end === "fin") await stream.close();
		else await pass(30_000);
		await flush();
		expect(jest.getTimerCount()).toBe(before);
	});
});
