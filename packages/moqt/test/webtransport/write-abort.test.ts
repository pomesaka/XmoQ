import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import {
	connectOver,
	type MoqtConnection,
	type PublishedTrack,
} from "../../src/webtransport/connection.ts";
import {
	PublishEndedError,
	SessionClosedError,
	SubgroupClosedError,
} from "../../src/webtransport/endings.ts";
import { hex, MOQTAIL_RELAY_SETUP, utf8 } from "../helpers.ts";
import {
	createFakePeer,
	type FakePeer,
	FakeWebTransportError,
	fakeStopWithoutCode,
	fakeStreamError,
	type PeerBidiStream,
} from "./fake-transport.ts";

// What the peer's STOP_SENDING looks like to a writer in Chromium 156, in
// its two forms (docs/experiments/2026-10-11-write-abort-in-chromium.md):
// a WebTransportError with the code, and, where a write was under way, a
// NetworkError without one.
const STOPS: {
	form: string;
	error: () => unknown;
	code: number | undefined;
}[] = [
	{ form: "with its code", error: () => fakeStreamError(5, "stop"), code: 5 },
	{
		form: "as a NetworkError without a code",
		error: fakeStopWithoutCode,
		code: undefined,
	},
];

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };
const SUBGROUP = {
	groupId: 0n,
	subgroupId: { mode: "Zero" },
	properties: false,
	endOfGroup: false,
} as const;
const OBJECT = { objectId: 0n, payload: utf8("a") };

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

/** What a promise rejects with. Fails the test when it resolves. */
async function rejection(promise: Promise<unknown>): Promise<unknown> {
	const outcome = await promise.then(
		() => ({ rejected: false }) as const,
		(error: unknown) => ({ rejected: true, error }) as const,
	);
	if (!outcome.rejected) throw new Error("the promise resolved");
	return outcome.error;
}

/** A session with a PUBLISH that the peer accepted. */
async function published(
	options: Parameters<typeof createFakePeer>[0] = {},
): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
	/** The stream this endpoint sent its SETUP on, as the peer has it. */
	control: ReadableStreamDefaultReader<Uint8Array>;
	request: PeerBidiStream;
	track: PublishedTrack;
}> {
	const peer = createFakePeer(options);
	const connecting = connectOver(peer.transport);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const connection = await connecting;
	const control = await peer.clientUni();
	const publishing = connection.publish({ ...TRACK, trackAlias: 1n });
	const request = await peer.clientBidi();
	await request.sent.read();
	await request.reply.write(hex("07 00 01 00"));
	return { peer, connection, control, request, track: await publishing };
}

describe("a stream this endpoint writes is stopped by the peer", () => {
	test.each(STOPS)(
		"a subgroup stream that was stopped $form does not keep PUBLISH_DONE back, and is counted in it",
		async ({ error }) => {
			const { peer, request, track } = await published();
			const subgroup = await track.openSubgroup(SUBGROUP);
			const stream = await peer.clientUni();
			await stream.read();
			await stream.cancel(error());
			await settle();

			// The stream is over for the session: a write finds it closed.
			const refused = await rejection(subgroup.write(OBJECT));
			expect(refused).toBeInstanceOf(SubgroupClosedError);
			// The subscriber finishes its direction, so that `done()` settles.
			await request.reply.close();
			expect(await track.done(0x2n, "")).toEqual({ type: "fin" });
			const sent = await request.sent.read();
			expect(
				decodeControlMessage(sent.value ?? new Uint8Array(0)),
			).toMatchObject({
				status: "ok",
				// §10.11: the streams that were opened, the stopped one among them.
				value: { type: "PUBLISH_DONE", statusCode: 0x2n, streamCount: 1n },
			});
		},
	);

	// The write is made before the connection has learned of the stop, so
	// it is the transport that rejects it, with the stream's error.
	test.each(STOPS)(
		"a write that meets a stop $form fails with SubgroupClosedError, which says that it was a stop and has the code",
		async ({ error, code }) => {
			const { peer, track } = await published();
			const subgroup = await track.openSubgroup(SUBGROUP);
			const stream = await peer.clientUni();
			await stream.read();
			void stream.cancel(error());
			const refused = await rejection(subgroup.write(OBJECT));
			expect(refused).toBeInstanceOf(SubgroupClosedError);
			expect(refused).toMatchObject({ metStop: true, code });
			// Later writes find the stream closed, and `reset()` is safe.
			const later = await rejection(subgroup.write(OBJECT));
			expect(later).toBeInstanceOf(SubgroupClosedError);
			expect(later).toMatchObject({ metStop: false });
			await subgroup.reset();
		},
	);

	test.each(STOPS)(
		"the request stream of a PUBLISH that is stopped $form ends the subscription (§5.1)",
		async ({ error, code }) => {
			const { request, track } = await published();
			const subgroup = await track.openSubgroup(SUBGROUP);
			await request.sent.cancel(error());
			expect(await track.ended).toEqual({ type: "peer-reset", code });
			const refused = await rejection(subgroup.write(OBJECT));
			expect(refused).toBeInstanceOf(PublishEndedError);
		},
	);

	test("the control stream that fails with a NetworkError is not taken for a stop: the session stays open", async () => {
		const { connection, control } = await published();
		await control.cancel(fakeStopWithoutCode());
		await settle();
		let closed = false;
		void connection.closed.then(() => {
			closed = true;
		});
		await settle();
		expect(closed).toBe(false);
	});

	test("the control stream that is stopped with a code is a violation of the peer (§3.3)", async () => {
		const { connection, control } = await published();
		await control.cancel(fakeStreamError(0, "stop"));
		expect(await connection.closed).toMatchObject({
			type: "error",
			error: { code: 3 },
		});
	});

	// `reset()` aborts the writer with a stream error that has this
	// endpoint's own code, and a write that is under way is rejected with
	// that very error (Chromium 156, spikes/wt-write-abort/out/own-reset-20.txt).
	// It is no stop of the subscriber.
	test("a write that is under way when this endpoint resets the stream does not say that it met a stop", async () => {
		let stalled = false;
		const { track } = await published({ stallWrites: () => stalled });
		const subgroup = await track.openSubgroup(SUBGROUP);
		stalled = true;
		const writing = subgroup.write(OBJECT);
		await settle();
		await subgroup.reset(5);
		const refused = await rejection(writing);
		expect(refused).toBeInstanceOf(SubgroupClosedError);
		expect(refused).toMatchObject({ metStop: false, code: undefined });
	});

	// Chromium rejects `createUnidirectionalStream` with a NetworkError too,
	// the name a stopped stream has. That ends the session here, and the
	// caller has to hear that, not that a stream was stopped.
	test("a stream that cannot be opened ends the session, and is not taken for a stream the subscriber stopped", async () => {
		let fail = false;
		const { connection, track } = await published({
			failUni: () =>
				fail
					? new DOMException("Failed to create send stream", "NetworkError")
					: undefined,
		});
		fail = true;
		const refused = await rejection(track.openSubgroup(SUBGROUP));
		expect(refused).toBeInstanceOf(SessionClosedError);
		expect(await connection.closed).toMatchObject({ type: "error" });
	});

	// A browser that fails a stream with an error of the session and then
	// does not settle `closed` must not leave the write without an answer.
	test("a write that fails with an error of the session is answered also when the session's end is not reported", async () => {
		const { peer, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		const stream = await peer.clientUni();
		await stream.read();
		void stream.cancel(FakeWebTransportError.sessionEnded());
		const refused = await rejection(subgroup.write(OBJECT));
		expect(refused).toBeInstanceOf(SessionClosedError);
		expect(refused).toMatchObject({ close: { type: "error" } });
	});

	test("a write that fails because the session ended fails with SessionClosedError", async () => {
		const { peer, track } = await published();
		const subgroup = await track.openSubgroup(SUBGROUP);
		// The streams fail a task before `closed` settles, as in a browser.
		peer.close({ closeCode: 0, reason: "bye" });
		const refused = await rejection(subgroup.write(OBJECT));
		expect(refused).toBeInstanceOf(SessionClosedError);
		expect(refused).toMatchObject({ close: { type: "peer", reason: "bye" } });
	});
});
