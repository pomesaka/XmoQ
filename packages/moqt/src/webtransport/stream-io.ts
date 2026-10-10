import type { Session } from "../core/session.ts";
import type { PeerStreamKind, SessionOutput } from "../core/session-output.ts";
import { createStreamSlots } from "./stream-slots.ts";
import type { MoqtTransport } from "./transport.ts";

/**
 * Something the caller has to wait for before it produces more for the same
 * destination.
 *
 * WHY an object around the promise: an async function that returns a promise
 * resolves with that promise's value, so `perform` could not hand a pending
 * promise back to be awaited later. It would be awaited on the spot, inside
 * the queue that serializes all outputs, and one slow stream would hold up
 * every other.
 */
export interface Backpressure {
	settled: Promise<void>;
}

/** The outputs that act on a stream of the transport. */
export type StreamOutput = Exclude<
	SessionOutput,
	{ type: "close-session" | "event" }
>;

export interface StreamIoOptions {
	transport: MoqtTransport;
	/**
	 * Unidirectional streams of the peer read at once. Further ones are not
	 * taken from the transport until one of these has ended.
	 */
	maxPeerUniStreams: number;
	/**
	 * Bidirectional streams of the peer read at once: its requests. Further
	 * ones are not taken from the transport until one of these has ended.
	 */
	maxPeerBidiStreams: number;
	/**
	 * How long a stream of the peer has, from the moment it is taken from
	 * the transport, to say what it is. One that has not is stopped.
	 */
	firstBytesTimeoutMs: number;
	/** Called with the bytes of every stream. */
	trace(direction: "sent" | "received", stream: string, data: Uint8Array): void;
	/**
	 * Reports input to the session and performs its outputs. Resolves when
	 * the source of the input may be read further. Never rejects.
	 */
	input(action: (session: Session) => SessionOutput[]): Promise<void>;
	/** Ends the session because this endpoint cannot go on. */
	fatal(error: Error): void;
}

/**
 * The streams of one transport session: performs the outputs that act on a
 * stream, and reads every stream to report what arrives. It knows nothing of
 * requests or subscriptions.
 */
export interface StreamIo {
	/**
	 * Performs one output. Stream creation is awaited so later outputs find the
	 * stream; the write itself is returned as backpressure.
	 */
	perform(output: StreamOutput): Promise<Backpressure | undefined>;
	/** Starts accepting the streams the peer opens. */
	acceptPeerStreams(): void;
	/**
	 * How many streams something is kept of, in either direction. Nothing
	 * acts on it; a test reads it to see that a stream that has ended in
	 * both directions is forgotten.
	 */
	heldStreams(): number;
	/** Forgets every stream. The transport session is closing and takes them down. */
	clear(): void;
}

/** A stream the peer opened, while its receiving side is read. */
interface PeerStream {
	reader: ReadableStreamDefaultReader<unknown>;
	/** Runs out when the stream has not said what it is in time; cleared when it has. */
	firstBytesTimer: ReturnType<typeof setTimeout>;
}

interface LocalStream {
	/** Name of the stream in `trace`. */
	name: string;
	writer: WritableStreamDefaultWriter<Uint8Array>;
	/** Present on bidirectional streams until their read side has ended. */
	reader: ReadableStreamDefaultReader<unknown> | undefined;
	/** This endpoint sent FIN. */
	sendClosed: boolean;
}

type StreamKind = "local" | "peer-uni" | "peer-bidi";

/** Name of a stream in `trace`. */
function streamName(kind: StreamKind, stream: number): string {
	return `${kind}-${stream}`;
}

function toError(value: unknown): Error {
	return value instanceof Error ? value : new Error(String(value));
}

const EMPTY = new Uint8Array(0);

export function createStreamIo(options: StreamIoOptions): StreamIo {
	const {
		transport,
		maxPeerUniStreams,
		maxPeerBidiStreams,
		firstBytesTimeoutMs,
		trace,
		input,
		fatal,
	} = options;
	const localStreams = new Map<number, LocalStream>();
	const peerUniStreams = new Map<number, PeerStream>();
	const peerBidiWriters = new Map<
		number,
		WritableStreamDefaultWriter<Uint8Array>
	>();
	/** The peer's bidirectional streams, until their receiving sides have ended. */
	const peerBidiStreams = new Map<number, PeerStream>();
	/**
	 * The peer's bidirectional streams the session took over
	 * (`adopt-peer-stream`), with the number it gave each, until their
	 * receiving sides have ended.
	 */
	const adoptedPeerStreams = new Map<number, number>();
	/**
	 * Starts the time a stream of the peer has to say what it is. The timer
	 * is kept with the stream (`PeerStream`), so that it goes when the
	 * stream does.
	 *
	 * WHY: a stream that never says anything would hold one of the places
	 * for the streams read at once for as long as the session lasts, and as
	 * many of them as there are places would keep every later stream of the
	 * peer from being read. The time runs from when the stream is taken: one
	 * that waits in the transport for a place has none yet, as in the Rust
	 * implementation (`first_bytes_timeout` in crates/moqt-io).
	 *
	 * The session decides whether the stream is stopped: it may have said
	 * what it is in a read whose outputs are still in the queue.
	 *
	 * A browser runs the timer late in a page that is in the background
	 * (docs/experiments/2026-10-11-first-bytes-timeout-in-chromium.md has by
	 * how much). The stream then holds its place for longer; it is not
	 * stopped early.
	 */
	const startFirstBytesTimer = (
		kind: PeerStreamKind,
		peerStream: number,
	): ReturnType<typeof setTimeout> =>
		setTimeout(() => {
			void input((session) => session.peerStreamTimedOut(kind, peerStream));
		}, firstBytesTimeoutMs);
	const peerStreamsOf = (kind: PeerStreamKind): Map<number, PeerStream> =>
		kind === "uni" ? peerUniStreams : peerBidiStreams;
	/** The stream has ended: its timer and what was kept of it go. */
	const forgetPeerStream = (kind: PeerStreamKind, peerStream: number): void => {
		const streams = peerStreamsOf(kind);
		clearTimeout(streams.get(peerStream)?.firstBytesTimer);
		streams.delete(peerStream);
	};
	let nextPeerStream = 0;
	/** Set by `clear`: nothing more is taken from the transport. */
	let cleared = false;

	/**
	 * The unidirectional streams of the peer that are read at once.
	 *
	 * WHY a limit on streams and none on Objects: the queue of a subscription
	 * makes a stream wait only after it has handed over the Objects of the
	 * chunk it read, so without this every new stream adds to the queue. With
	 * it, what the streams hold is bounded by their number, as in the Rust
	 * implementation (`max_uni_streams`), and the rest stays with the
	 * transport, where flow control holds the peer back.
	 *
	 * A stream holds its place until it has ended: a subgroup that stays open
	 * for a long time, and a stream of no subscription that the peer does not
	 * close, each keep one.
	 *
	 * WHY the control stream counts: it is one of the peer's unidirectional
	 * streams, and nothing tells it apart before its first bytes are read. A
	 * peer that keeps to the draft cannot take every place before it: data
	 * streams come for a subscription, and a subscription is made after
	 * SETUP, which the control stream carries.
	 *
	 * FIXME: in one shape of traffic, a subscription nobody reads holds up
	 * the other requests of its session. Seen in Chromium 156 with the own
	 * relay (docs/experiments/2026-10-10-subscription-backlog-in-chromium.md):
	 * with a subscription that was not read and one Object of 100,000 bytes a
	 * stream, the relay's PUBLISH_DONE for it did not arrive, and after the
	 * relay had ended that subscription its SUBSCRIBE_OK for another track of
	 * the same session did not arrive either. With Objects of 57 bytes, and
	 * with 30 Objects of 100,000 bytes a stream, both arrived; a second
	 * request made while the first subscription was still served was not
	 * tried. Without this limit the second request was answered in the same
	 * shape of traffic, while the page held everything that was sent.
	 * The likely cause, not verified with a capture: the streams that are not
	 * taken stay in the browser and hold the flow-control credit of the
	 * connection, so that the sender cannot send on a request stream either.
	 * §11.4.2 names a deadlock of that kind for control messages and asks
	 * endpoints to "allocate connection flow control to the control streams
	 * before allocating it to any data streams"; a page has no say in how the
	 * browser's credit is given out. The Rust implementation has a related
	 * FIXME at `read_uni_streams` in crates/moqt-io/src/streams.rs. Whether
	 * the client should end such a subscription itself is in
	 * docs/open-questions.md.
	 */
	const uniSlots = createStreamSlots(maxPeerUniStreams);

	/**
	 * The bidirectional streams of the peer whose receiving side is read at
	 * once.
	 *
	 * WHY a limit: a request of the peer is held until its first message is
	 * whole, and a FETCH is decoded whole before it is refused, up to 64 KiB a
	 * stream (`core/peer-requests.ts`). Without the limit, how many streams
	 * hold that much is up to the transport: Chromium permits a peer at least
	 * 100 bidirectional streams at once and takes no lower wish from the page
	 * (docs/moqt-implementation-notes.md). With it, the number is this
	 * endpoint's own, as in the Rust implementation (`max_request_streams`),
	 * and the rest stays with the transport, where the peer waits.
	 *
	 * A stream holds its place until the peer has ended its direction, also
	 * after the request was answered. A SUBSCRIBE that was accepted holds it
	 * for as long as the subscription lasts, unless its subscriber finishes
	 * its direction before.
	 *
	 * A stream that does not send its first message whole is stopped after
	 * `firstBytesTimeoutMs` (`startFirstBytesTimer` above).
	 *
	 * FIXME: nothing ends a stream whose request was refused and whose peer
	 * does not end its direction: as many of them as the limit keep every
	 * later request of the peer from being read. (A subscription this
	 * endpoint ended with PUBLISH_DONE is not among them: the peer's
	 * direction is stopped after `peerStreamEndTimeoutMs`.) The Rust
	 * implementation has the same ("how long it stays open is the business
	 * of the two endpoints", crates/moqt-io/src/streams.rs).
	 *
	 * A subgroup stream that stays open after its header holds a place among
	 * the unidirectional streams in the same way, and is left to: a time
	 * would end a Group that is sent slowly, which is a way to publish.
	 */
	const bidiSlots = createStreamSlots(maxPeerBidiStreams);

	const write = (local: LocalStream, data: Uint8Array): Promise<void> => {
		trace("sent", local.name, data);
		return local.writer.write(data);
	};

	/**
	 * Reports a STOP_SENDING from the peer on a stream this endpoint opened.
	 *
	 * WHY `writer.closed`: it is the only place a STOP_SENDING shows without a
	 * write. In Chromium 156 it rejected with a WebTransportError
	 * ("Received STOP_SENDING") as soon as the frame arrived, while the read
	 * side of the same stream stayed open (checked on 2026-10-10 against a
	 * wtransport 0.7.2 server that only stopped the stream). A request stream
	 * carries nothing after the request for a long time, so a write would not
	 * find out.
	 */
	const watchStop = (
		stream: number,
		writer: WritableStreamDefaultWriter<Uint8Array>,
	): void => {
		writer.closed.catch((error: unknown) => {
			// The end of the session rejects `closed` too, and can do so before
			// `transport.closed` settles. Taken for a STOP_SENDING, it would turn
			// the peer's close into a stopped control stream.
			const stopped = transport.isStreamError(error);
			// A stop the browser did not pass the code of. Without this the
			// session would count the stream as open for good, and a publisher
			// could not send PUBLISH_DONE.
			const withoutCode = !stopped && transport.isStopWithoutCode(error);
			if (!stopped && !withoutCode) return;
			// A reset by this endpoint has removed the stream by then.
			if (localStreams.get(stream)?.writer !== writer) return;
			void input((session) =>
				stopped
					? session.localStreamStopped(stream, transport.streamErrorCode(error))
					: session.localStreamAborted(stream),
			);
		});
	};

	/** Forgets a local stream once neither direction is in use. */
	const releaseLocalStream = (stream: number): void => {
		const local = localStreams.get(stream);
		if (local?.sendClosed && local.reader === undefined) {
			localStreams.delete(stream);
		}
	};

	/** Reads one stream to its end and reports what arrives. */
	const pump = async (source: {
		name: string;
		reader: ReadableStreamDefaultReader<unknown>;
		onData(session: Session, data: Uint8Array, fin: boolean): SessionOutput[];
		/** `error` is what the read failed with. */
		onReset(session: Session, error: unknown): SessionOutput[];
		onEnd(): void;
	}): Promise<void> => {
		try {
			for (;;) {
				// Only the read is guarded: it fails when the peer reset the stream
				// or the session ended. Anything else that throws in this loop is a
				// fault of this endpoint and goes to `fatal` below.
				const chunk = await source.reader
					.read()
					.catch((error: unknown) => ({ failed: true, error }) as const);
				if ("failed" in chunk) {
					// When the session ended, the read fails as well. That is not a
					// reset by the peer, and `transport.closed` reports it.
					if (transport.isStreamError(chunk.error)) {
						await input((session) => source.onReset(session, chunk.error));
					}
					return;
				}
				if (chunk.done) {
					await input((session) => source.onData(session, EMPTY, true));
					return;
				}
				const data = chunk.value;
				if (!(data instanceof Uint8Array)) {
					throw new TypeError(
						`${source.name} produced a chunk that is not bytes`,
					);
				}
				trace("received", source.name, data);
				await input((session) => source.onData(session, data, false));
			}
		} catch (error) {
			fatal(toError(error));
		} finally {
			source.onEnd();
		}
	};

	/** Hands every stream the peer opens to `start`, until the transport ends. */
	const accept = async <T>(
		streams: ReadableStream<T>,
		start: (peerStream: number, stream: T) => void,
		/** Resolves when the next stream may be taken. */
		room: () => Promise<void> = () => Promise.resolve(),
	): Promise<void> => {
		const reader = streams.getReader();
		try {
			for (;;) {
				await room();
				if (cleared) return;
				// Only the read is guarded: it fails when the transport session
				// ends, which `transport.closed` reports to the connection.
				const next = await reader.read().catch(() => undefined);
				// The session may have ended while this waited for a stream.
				if (next === undefined || next.done || cleared) return;
				start(nextPeerStream, next.value);
				nextPeerStream += 1;
			}
		} catch (error) {
			fatal(toError(error));
		}
	};

	return {
		async perform(output) {
			switch (output.type) {
				case "open-uni-stream": {
					const writable = await transport.createUnidirectionalStream();
					const writer = writable.getWriter();
					const local: LocalStream = {
						name: streamName("local", output.stream),
						writer,
						reader: undefined,
						sendClosed: false,
					};
					localStreams.set(output.stream, local);
					watchStop(output.stream, writer);
					return { settled: write(local, output.data) };
				}
				case "open-bidi-stream": {
					const { stream } = output;
					const bidi = await transport.createBidirectionalStream();
					const writer = bidi.writable.getWriter();
					const reader = bidi.readable.getReader();
					const local: LocalStream = {
						name: streamName("local", stream),
						writer,
						reader,
						sendClosed: false,
					};
					localStreams.set(stream, local);
					void pump({
						name: streamName("local", stream),
						reader,
						onData: (session, data, fin) =>
							session.receiveRequestStreamData(stream, data, fin),
						onReset: (session, error) =>
							session.requestStreamReset(
								stream,
								transport.streamErrorCode(error),
							),
						onEnd() {
							const local = localStreams.get(stream);
							if (local !== undefined) local.reader = undefined;
							releaseLocalStream(stream);
						},
					});
					watchStop(stream, writer);
					return { settled: write(local, output.data) };
				}
				case "adopt-peer-stream": {
					const { peerStream, stream } = output;
					const writer = peerBidiWriters.get(peerStream);
					const reader = peerBidiStreams.get(peerStream)?.reader;
					// The session adopts a stream while it handles bytes of it,
					// and the stream is forgotten only after that.
					if (writer === undefined || reader === undefined) {
						throw new Error(`peer stream ${peerStream} is not open`);
					}
					peerBidiWriters.delete(peerStream);
					adoptedPeerStreams.set(peerStream, stream);
					localStreams.set(stream, {
						name: streamName("peer-bidi", peerStream),
						writer,
						reader,
						sendClosed: false,
					});
					// A subscriber ends its subscription by stopping this stream.
					watchStop(stream, writer);
					return undefined;
				}
				case "send": {
					const local = localStreams.get(output.stream);
					if (local === undefined) {
						throw new Error(`stream ${output.stream} is not open`);
					}
					return { settled: write(local, output.data) };
				}
				case "finish": {
					const local = localStreams.get(output.stream);
					if (local === undefined) return undefined;
					local.sendClosed = true;
					releaseLocalStream(output.stream);
					// A FIN fails when the peer already stopped or reset the stream.
					// Either way this endpoint is done sending on it.
					return { settled: local.writer.close().catch(() => {}) };
				}
				case "reset": {
					const local = localStreams.get(output.stream);
					if (local === undefined) return undefined;
					localStreams.delete(output.stream);
					const error = transport.streamError(
						output.code,
						"reset by the MOQT session",
					);
					// Both fail on a direction that is already closed or errored,
					// which is the state a reset is meant to reach.
					void local.writer.abort(error).catch(() => {});
					void local.reader?.cancel(error).catch(() => {});
					return undefined;
				}
				case "respond": {
					const writer = peerBidiWriters.get(output.peerStream);
					if (writer === undefined) return undefined;
					peerBidiWriters.delete(output.peerStream);
					trace(
						"sent",
						streamName("peer-bidi", output.peerStream),
						output.data,
					);
					// The answer rejects a request. If the peer already gave up on
					// the stream, there is nobody left to tell.
					void writer.write(output.data).catch(() => {});
					return { settled: writer.close().catch(() => {}) };
				}
				case "stop-peer-direction": {
					const error = transport.streamError(
						output.code,
						"stopped by the MOQT session",
					);
					// Fails when the peer's direction already ended, and then
					// nothing is left to stop.
					void localStreams
						.get(output.stream)
						?.reader?.cancel(error)
						.catch(() => {});
					return undefined;
				}
				case "peer-stream-identified":
					clearTimeout(
						peerStreamsOf(output.kind).get(output.peerStream)?.firstBytesTimer,
					);
					return undefined;
				case "abandon-peer-request": {
					const error = transport.streamError(
						output.code,
						"given up by the MOQT session",
					);
					// Each fails on a direction that has ended, and then nothing is
					// left to give up.
					void peerBidiStreams
						.get(output.peerStream)
						?.reader.cancel(error)
						.catch(() => {});
					const writer = peerBidiWriters.get(output.peerStream);
					peerBidiWriters.delete(output.peerStream);
					void writer?.abort(error).catch(() => {});
					return undefined;
				}
				case "stop-sending": {
					const error = transport.streamError(
						output.code,
						"stopped by the MOQT session",
					);
					// Fails when the stream already ended, and then nothing is left
					// to stop.
					void peerUniStreams
						.get(output.peerStream)
						?.reader.cancel(error)
						.catch(() => {});
					return undefined;
				}
			}
		},

		// FIXME: datagrams are not read. §11 says "An endpoint that receives an
		// unknown datagram type MUST close the session", and an Object sent in a
		// datagram is lost without a trace. Read `transport.datagrams` once
		// Object Datagrams (§11.3) are implemented.
		acceptPeerStreams() {
			void accept(
				transport.incomingUnidirectionalStreams,
				(peerStream, stream) => {
					const reader = stream.getReader();
					peerUniStreams.set(peerStream, {
						reader,
						firstBytesTimer: startFirstBytesTimer("uni", peerStream),
					});
					uniSlots.take();
					// Before any byte: the session stops only streams it knows.
					void input((session) => {
						session.peerStreamOpened("uni", peerStream);
						return [];
					});
					void pump({
						name: streamName("peer-uni", peerStream),
						reader,
						onData: (session, data, fin) =>
							session.receivePeerUniStreamData(peerStream, data, fin),
						onReset: (session, error) =>
							session.peerUniStreamReset(
								peerStream,
								transport.streamErrorCode(error),
							),
						onEnd() {
							forgetPeerStream("uni", peerStream);
							uniSlots.release();
						},
					});
				},
				uniSlots.free,
			);
			void accept(
				transport.incomingBidirectionalStreams,
				(peerStream, stream) => {
					const writer = stream.writable.getWriter();
					const reader = stream.readable.getReader();
					peerBidiWriters.set(peerStream, writer);
					peerBidiStreams.set(peerStream, {
						reader,
						firstBytesTimer: startFirstBytesTimer("bidi", peerStream),
					});
					bidiSlots.take();
					void input((session) => {
						session.peerStreamOpened("bidi", peerStream);
						return [];
					});
					void pump({
						name: streamName("peer-bidi", peerStream),
						reader,
						onData: (session, data, fin) =>
							session.receivePeerBidiStreamData(peerStream, data, fin),
						onReset: (session, error) =>
							session.peerBidiStreamReset(
								peerStream,
								transport.streamErrorCode(error),
							),
						onEnd() {
							forgetPeerStream("bidi", peerStream);
							bidiSlots.release();
							// A stream the session took over is ended by the session.
							const adopted = adoptedPeerStreams.get(peerStream);
							if (adopted !== undefined) {
								adoptedPeerStreams.delete(peerStream);
								const local = localStreams.get(adopted);
								if (local !== undefined) local.reader = undefined;
								releaseLocalStream(adopted);
							}
							// The request ended without an answer from this endpoint,
							// so the send side was never used. Closing it can only fail
							// if the peer already stopped it.
							if (peerBidiWriters.delete(peerStream)) {
								void writer.close().catch(() => {});
							}
						},
					});
				},
				bidiSlots.free,
			);
		},

		heldStreams: () =>
			localStreams.size +
			peerUniStreams.size +
			peerBidiStreams.size +
			peerBidiWriters.size +
			adoptedPeerStreams.size,

		clear() {
			localStreams.clear();
			for (const streams of [peerUniStreams, peerBidiStreams]) {
				for (const { firstBytesTimer } of streams.values()) {
					clearTimeout(firstBytesTimer);
				}
				streams.clear();
			}
			peerBidiWriters.clear();
			adoptedPeerStreams.clear();
			cleared = true;
			uniSlots.close();
			bidiSlots.close();
		},
	};
}
