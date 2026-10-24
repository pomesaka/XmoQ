import { expect, test } from "bun:test";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import {
	createStreamIo,
	type StreamIo,
} from "../../src/webtransport/stream-io.ts";
import { fixture, hex, MOQTAIL_RELAY_SETUP } from "../helpers.ts";
import {
	createFakePeer,
	type FakePeer,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

/** The captured SUBSCRIBE with Request ID 1, one of a server. */
function subscribeBytes(): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = "01";
	return hex(pairs.join(" "));
}

/**
 * A session and the streams of a fake transport, joined as the connection
 * joins them: outputs are performed in order, and events are dropped.
 */
async function joined(): Promise<{
	peer: FakePeer;
	session: Session;
	io: StreamIo;
	/** Performs what a call of the application led to. */
	apply(outputs: SessionOutput[]): Promise<void>;
}> {
	const peer = createFakePeer();
	const session = createSession();
	let queue: Promise<void> = Promise.resolve();
	const apply = (outputs: SessionOutput[]): Promise<void> => {
		queue = queue.then(async () => {
			for (const output of outputs) {
				if (output.type === "event") continue;
				if (output.type === "close-session") {
					throw new Error(`the session closed: ${output.reason}`);
				}
				await io.perform(output);
			}
		});
		return queue;
	};
	const io = createStreamIo({
		transport: peer.transport,
		maxPeerUniStreams: 8,
		maxPeerBidiStreams: 8,
		firstBytesTimeoutMs: 10_000,
		idleSubgroupStreamTimeoutMs: 30_000,
		openBidiTimeoutMs: 10_000,
		trace: () => {},
		input: (action) => apply(action(session)),
		fatal: (error) => {
			throw error;
		},
	});
	io.acceptPeerStreams();
	await apply(session.start());
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	await settle();
	return { peer, session, io, apply };
}

/** The peer has sent its SUBSCRIBE, and the session has taken its stream over. */
async function asked(): Promise<
	Awaited<ReturnType<typeof joined>> & { stream: PeerBidiStream; held: number }
> {
	const all = await joined();
	const held = all.io.heldStreams();
	const stream = all.peer.openBidi();
	await stream.reply.write(subscribeBytes());
	await settle();
	return { ...all, stream, held };
}

// A stream of the peer that the session took over is kept under two
// numbers. Each row ends it in both directions, in an order, and what is
// kept has to be what was kept before the stream came.
const rows: {
	name: string;
	steps: ((all: Awaited<ReturnType<typeof asked>>) => Promise<unknown>)[];
}[] = [
	{
		name: "this endpoint finishes (PUBLISH_DONE), then the peer",
		steps: [
			({ session, apply }) => apply(session.acceptSubscribe(1n, {}).outputs),
			({ session, apply }) => apply(session.publishDone(1n, 2n, "")),
			({ stream }) => stream.reply.close(),
		],
	},
	{
		name: "the peer finishes, then this endpoint (PUBLISH_DONE)",
		steps: [
			({ session, apply }) => apply(session.acceptSubscribe(1n, {}).outputs),
			({ stream }) => stream.reply.close(),
			({ session, apply }) => apply(session.publishDone(1n, 2n, "")),
		],
	},
	{
		name: "this endpoint rejects the request, then the peer finishes",
		steps: [
			({ session, apply }) => apply(session.rejectPeerRequest(1n, 0x10n, "")),
			({ stream }) => stream.reply.close(),
		],
	},
	{
		name: "the peer resets its direction",
		steps: [
			({ session, apply }) => apply(session.acceptSubscribe(1n, {}).outputs),
			({ stream }) => stream.reply.abort(fakeStreamError(1, "reset")),
		],
	},
];

test("a stream of the peer that was taken over is kept while it is open", async () => {
	const { io, held } = await asked();
	expect(io.heldStreams()).toBeGreaterThan(held);
});

test.each(rows)(
	"nothing is kept of a stream that was taken over once $name",
	async (row) => {
		const all = await asked();
		for (const step of row.steps) {
			await step(all);
			await settle();
		}
		expect(all.io.heldStreams()).toBe(all.held);
		expect(all.session.heldStreams()).toBe(1);
	},
);
