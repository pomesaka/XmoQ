import { describe, expect, test } from "bun:test";
import {
	connectOver,
	type MoqtConnection,
	type NamespaceChange,
	type NamespaceSubscription,
} from "../../src/webtransport/connection.ts";
import {
	RequestAbortedError,
	RequestFailedError,
} from "../../src/webtransport/endings.ts";
import {
	fromALibrary,
	hex,
	MOQTAIL_RELAY_SETUP,
	toHex,
	utf8,
} from "../helpers.ts";
import { flush, state } from "./clock.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

// SUBSCRIBE_NAMESPACE (§10.18) through the connection. What the session does
// with the bytes is in test/core/session-subscribe-namespace.test.ts.

const PREFIX = [utf8("xmoq"), utf8("chat")];
/** moqtail's SUBSCRIBE_NAMESPACE for (`xmoq`, `chat`), with the Request ID 0 of a client's first request. */
const SUBSCRIBE_NAMESPACE_0 = "50 00 0d 00 02 04 78 6d 6f 71 04 63 68 61 74 00";
const NAMESPACE = fromALibrary("namespace-moqtail-ts");
const NAMESPACE_DONE = fromALibrary("namespace-done-moqtail-ts");
const REQUEST_OK = "07 00 01 00";
const ALICE = [utf8("xmoq"), utf8("chat"), utf8("room"), utf8("alice")];

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

/** A SUBSCRIBE_NAMESPACE for (`xmoq`, `chat`) that the peer answered with REQUEST_OK. */
async function subscribed(signal?: AbortSignal): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	stream: PeerBidiStream;
	subscription: NamespaceSubscription;
}> {
	const { peer, connection } = await connected();
	const asking = connection.subscribeNamespace(
		{ trackNamespacePrefix: PREFIX },
		{ signal },
	);
	const stream = await peer.clientBidi();
	const { value } = await stream.sent.read();
	if (value === undefined) throw new Error("the stream ended");
	expect(toHex(value)).toBe(SUBSCRIBE_NAMESPACE_0);
	await stream.reply.write(hex(REQUEST_OK));
	return { peer, connection, stream, subscription: await asking };
}

/** Reads every change until `changes` closes. */
async function allChanges(
	subscription: NamespaceSubscription,
): Promise<NamespaceChange[]> {
	const changes: NamespaceChange[] = [];
	for await (const change of subscription.changes) changes.push(change);
	return changes;
}

describe("subscribeNamespace", () => {
	test("resolves at REQUEST_OK, hands on NAMESPACE and NAMESPACE_DONE with the whole namespace, and ends with the peer's FIN", async () => {
		const { stream, subscription, connection } = await subscribed();
		expect(subscription.requestId).toBe(0n);
		await stream.reply.write(
			hex(`${NAMESPACE} ${NAMESPACE_DONE} ${NAMESPACE}`),
		);
		await stream.reply.close();
		expect(await allChanges(subscription)).toEqual([
			{ trackNamespace: ALICE, done: false },
			{ trackNamespace: ALICE, done: true },
			{ trackNamespace: ALICE, done: false },
		]);
		expect(await subscription.ended).toEqual({ type: "peer-closed" });
		expect(await state(connection.closed)).toBe("pending");
	});

	// The changes are written in one go and the FIN after them; nothing of
	// `changes` is read before `ended` is looked at.
	test.each([
		{ written: 63, endedUnread: "resolved" },
		// The 64th fills `changes`: the request stream is not read on, and
		// the FIN behind it waits for a read.
		{ written: 64, endedUnread: "pending" },
	])(
		"a reader that does not read: $written changes leave `ended` $endedUnread",
		async ({ written, endedUnread }) => {
			const { stream, subscription } = await subscribed();
			await stream.reply.write(hex(Array(written).fill(NAMESPACE).join(" ")));
			await stream.reply.close();
			await flush();
			expect(
				(await state(subscription.ended)) === "pending"
					? "pending"
					: "resolved",
			).toBe(endedUnread);
			// Read, every change comes, and the FIN after them.
			expect((await allChanges(subscription)).length).toBe(written);
			expect(await subscription.ended).toEqual({ type: "peer-closed" });
		},
	);

	test("the peer resets the stream after the answer: `ended` has its code", async () => {
		// WHY nothing is written before the reset: data that was not read
		// when a reset came may be dropped (RFC 9000, §3.2), and this fake
		// drops it.
		const { stream, subscription } = await subscribed();
		await stream.reply.abort(fakeStreamError(5, "reset"));
		expect(await allChanges(subscription)).toEqual([]);
		expect(await subscription.ended).toEqual({ type: "peer-reset", code: 5 });
	});

	test.each<{
		name: string;
		cancel: (
			subscription: NamespaceSubscription,
			controller: AbortController,
		) => Promise<void>;
	}>([
		{ name: "cancel()", cancel: (subscription) => subscription.cancel() },
		{
			name: "the signal of the call",
			cancel: async (_, controller) => controller.abort(),
		},
		{
			name: "a reader that cancels `changes`",
			cancel: (subscription) => subscription.changes.cancel(),
		},
	])(
		"$name after the answer: both directions end with CANCELLED, and `ended` says cancelled",
		async ({ cancel }) => {
			const controller = new AbortController();
			const { stream, subscription, connection } = await subscribed(
				controller.signal,
			);
			await cancel(subscription, controller);
			expect(await state(stream.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(stream.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await subscription.ended).toEqual({ type: "cancelled" });
			// What the peer sent before it saw the reset reaches nobody.
			await flush();
			expect(await state(connection.closed)).toBe("pending");
		},
	);

	test("refused with PREFIX_OVERLAP: rejects with RequestFailedError, and the session stays open", async () => {
		const { peer, connection } = await connected();
		const asking = connection.subscribeNamespace({
			trackNamespacePrefix: PREFIX,
		});
		const stream = await peer.clientBidi();
		await stream.sent.read();
		// REQUEST_ERROR 0x30, Retry Interval 0, no reason.
		await stream.reply.write(hex("05 00 03 30 00 00"));
		const outcome = await state(asking);
		expect(outcome).toBeInstanceOf(RequestFailedError);
		expect(outcome).toMatchObject({ errorCode: 0x30n });
		expect(await state(connection.closed)).toBe("pending");
	});

	test("the signal aborts before the answer: both directions end with CANCELLED, and the call rejects", async () => {
		const { peer, connection } = await connected();
		const controller = new AbortController();
		const asking = connection.subscribeNamespace(
			{ trackNamespacePrefix: PREFIX },
			{ signal: controller.signal },
		);
		const stream = await peer.clientBidi();
		await stream.sent.read();
		controller.abort();
		expect(await state(stream.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
		const outcome = await state(asking);
		expect(outcome).toBeInstanceOf(RequestAbortedError);
		expect(outcome).toMatchObject({ abort: { type: "cancelled" } });
	});

	test("a parameter other than AUTHORIZATION_TOKEN is refused with a RangeError, and nothing is sent (§10.2.2)", async () => {
		const { peer, connection } = await connected();
		await expect(
			connection.subscribeNamespace({
				trackNamespacePrefix: PREFIX,
				parameters: { rendezvousTimeout: 1n },
			}),
		).rejects.toBeInstanceOf(RangeError);
		// No Request ID was used up.
		connection.subscribeNamespace({ trackNamespacePrefix: PREFIX });
		const stream = await peer.clientBidi();
		const { value } = await stream.sent.read();
		expect(value === undefined ? "" : toHex(value)).toBe(SUBSCRIBE_NAMESPACE_0);
	});

	test("the session closes after the answer: `changes` closes, and `ended` says how", async () => {
		const { subscription, connection } = await subscribed();
		connection.close();
		expect(await allChanges(subscription)).toEqual([]);
		expect(await subscription.ended).toMatchObject({
			type: "session-closed",
			close: { type: "local" },
		});
	});
});
