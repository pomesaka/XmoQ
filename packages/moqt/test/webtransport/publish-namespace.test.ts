import { describe, expect, test } from "bun:test";
import {
	connectOver,
	type MoqtConnection,
	type PeerSubscribe,
} from "../../src/webtransport/connection.ts";
import {
	RequestAbortedError,
	RequestFailedError,
} from "../../src/webtransport/endings.ts";
import { fixture, hex, MOQTAIL_RELAY_SETUP, toHex } from "../helpers.ts";
import { flush, state } from "./clock.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

const NAMESPACE = [hex("61")];
/** PUBLISH_NAMESPACE of (`a`) with Request ID 0, and with Request ID 2. */
const ANNOUNCE_0 = "06 00 05 00 01 01 61 00";
const ANNOUNCE_2 = "06 00 05 02 01 01 61 00";
const REQUEST_OK = "07 00 01 00";
/** REQUEST_ERROR DOES_NOT_EXIST (0x10), Retry Interval 0, no reason. */
const REQUEST_ERROR = "05 00 03 10 00 00";

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

/** A signal whose `removeEventListener` calls are counted. */
function countedSignal(): {
	controller: AbortController;
	removed: string[];
} {
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
	return { controller, removed };
}

/** The session goes on: the same namespace is announced again, with the next Request ID, and accepted. */
async function announcesAgain(
	peer: FakePeer,
	connection: MoqtConnection,
): Promise<void> {
	const again = connection.publishNamespace(NAMESPACE);
	const stream = await peer.clientBidi();
	expect(await sentHex(stream)).toBe(ANNOUNCE_2);
	await stream.reply.write(hex(REQUEST_OK));
	expect(await state(again)).toBe("resolved");
	expect(await state(connection.closed)).toBe("pending");
}

describe("withdrawing a namespace with the signal of publishNamespace (§6.2)", () => {
	test.each<{ name: string; answered: boolean; call: string }>([
		{ name: "before the answer", answered: false, call: "cancelled" },
		{ name: "after the answer", answered: true, call: "resolved" },
	])(
		"$name: both directions of the stream end with CANCELLED, the session forgets it, and stays open",
		async ({ answered, call }) => {
			const { peer, connection } = await connected();
			const { controller, removed } = countedSignal();
			const announcing = connection.publishNamespace(NAMESPACE, undefined, {
				signal: controller.signal,
			});
			const stream = await peer.clientBidi();
			expect(await sentHex(stream)).toBe(ANNOUNCE_0);
			if (answered) {
				await stream.reply.write(hex(REQUEST_OK));
				expect(await state(announcing)).toBe("resolved");
			}
			controller.abort();
			// §3.3.2: "abruptly terminating any directions of a stream that are
			// still open by resetting or sending STOP_SENDING".
			expect(await state(stream.sent.read())).toMatchObject({
				streamErrorCode: 1,
			});
			expect(await state(stream.reply.closed)).toMatchObject({
				streamErrorCode: 1,
			});
			const outcome = await state(announcing);
			if (call === "resolved") expect(outcome).toBe("resolved");
			else {
				expect(outcome).toBeInstanceOf(RequestAbortedError);
				expect(outcome).toMatchObject({ abort: { type: call } });
			}
			expect(removed).toEqual(["abort"]);
			await announcesAgain(peer, connection);
		},
	);

	test.each<{
		name: string;
		end: (stream: PeerBidiStream) => Promise<void>;
		call: string;
	}>([
		{
			name: "refused with REQUEST_ERROR",
			end: (stream) => stream.reply.write(hex(REQUEST_ERROR)),
			call: "refused",
		},
		{
			name: "reset by the peer after the answer",
			end: async (stream) => {
				await stream.reply.write(hex(REQUEST_OK));
				await flush();
				await stream.reply.abort(fakeStreamError(5, "reset"));
			},
			call: "resolved",
		},
		{
			// §6.2: the namespace is withdrawn "by cancelling the request (see
			// Section 3.3.2)", STOP_SENDING among the ways.
			name: "stopped by the peer after the answer",
			end: async (stream) => {
				await stream.reply.write(hex(REQUEST_OK));
				await flush();
				await stream.sent.cancel(fakeStreamError(5, "stop"));
			},
			call: "resolved",
		},
	])(
		"$name: the request lets go of the signal, which then does nothing",
		async ({ end, call }) => {
			const { peer, connection } = await connected();
			const { controller, removed } = countedSignal();
			const announcing = connection.publishNamespace(NAMESPACE, undefined, {
				signal: controller.signal,
			});
			const stream = await peer.clientBidi();
			await stream.sent.read();
			await end(stream);
			const outcome = await state(announcing);
			if (call === "resolved") expect(outcome).toBe("resolved");
			else expect(outcome).toBeInstanceOf(RequestFailedError);
			expect(removed).toEqual(["abort"]);
			controller.abort();
			await flush();
			expect(await state(connection.closed)).toBe("pending");
			await announcesAgain(peer, connection);
		},
	);

	test("a FIN of the peer after the answer withdraws nothing: the signal still withdraws the namespace", async () => {
		const { peer, connection } = await connected();
		const { controller, removed } = countedSignal();
		const announcing = connection.publishNamespace(NAMESPACE, undefined, {
			signal: controller.signal,
		});
		const stream = await peer.clientBidi();
		expect(await sentHex(stream)).toBe(ANNOUNCE_0);
		await stream.reply.write(hex(REQUEST_OK));
		expect(await state(announcing)).toBe("resolved");
		await stream.reply.close();
		await flush();
		expect(removed).toEqual([]);
		controller.abort();
		expect(await state(stream.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
		expect(removed).toEqual(["abort"]);
		expect(await state(connection.closed)).toBe("pending");
	});

	test("the session closed: the request lets go of the signal", async () => {
		const { peer, connection } = await connected();
		const { controller, removed } = countedSignal();
		const announcing = connection.publishNamespace(NAMESPACE, undefined, {
			signal: controller.signal,
		});
		const stream = await peer.clientBidi();
		await stream.sent.read();
		await stream.reply.write(hex(REQUEST_OK));
		expect(await state(announcing)).toBe("resolved");
		connection.close();
		expect(removed).toEqual(["abort"]);
		controller.abort();
		expect(await connection.closed).toMatchObject({ type: "local" });
	});

	test("with a signal that has aborted nothing is sent, and no Request ID is used", async () => {
		const { peer, connection } = await connected();
		const reason = new Error("too late");
		expect(
			await state(
				connection.publishNamespace(NAMESPACE, undefined, {
					signal: AbortSignal.abort(reason),
				}),
			),
		).toBe(reason);
		const announcing = connection.publishNamespace(NAMESPACE);
		const stream = await peer.clientBidi();
		expect(await sentHex(stream)).toBe(ANNOUNCE_0);
		await stream.reply.write(hex(REQUEST_OK));
		expect(await state(announcing)).toBe("resolved");
	});

	test("a subscription the peer made and the application serves goes on after the withdrawal", async () => {
		const { peer, connection } = await connected();
		const asked: PeerSubscribe[] = [];
		connection.serve({ subscribe: (request) => asked.push(request) });
		const controller = new AbortController();
		const announcing = connection.publishNamespace(NAMESPACE, undefined, {
			signal: controller.signal,
		});
		const announcement = await peer.clientBidi();
		await announcement.sent.read();
		await announcement.reply.write(hex(REQUEST_OK));
		await announcing;
		// The captured SUBSCRIBE of moqtail-rs, with Request ID 1 of the server.
		const pairs = fixture("subscribe-moqtail-rs").split(" ");
		pairs[3] = "01";
		const subscribe = peer.openBidi();
		await subscribe.reply.write(hex(pairs.join(" ")));
		await flush();
		const [request] = asked;
		if (request === undefined) throw new Error("the handler was not called");
		const track = await request.accept();
		await subscribe.sent.read();

		controller.abort();
		expect(await state(announcement.sent.read())).toMatchObject({
			streamErrorCode: 1,
		});
		expect(await state(track.ended)).toBe("pending");
		const subgroup = await track.openSubgroup({
			groupId: 0n,
			subgroupId: { mode: "Zero" },
			properties: false,
			endOfGroup: true,
		});
		await subgroup.close();
		expect(await state(subscribe.reply.closed)).toBe("pending");
		expect(await state(connection.closed)).toBe("pending");
	});
});
