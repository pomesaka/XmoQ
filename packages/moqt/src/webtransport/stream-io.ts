import type { Session } from "../core/session.ts";
import type { SessionOutput } from "../core/session-output.ts";
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
	/** Forgets every stream. The transport session is closing and takes them down. */
	clear(): void;
}

interface LocalStream {
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
	const { transport, maxPeerUniStreams, trace, input, fatal } = options;
	const localStreams = new Map<number, LocalStream>();
	const peerUniReaders = new Map<
		number,
		ReadableStreamDefaultReader<unknown>
	>();
	const peerBidiWriters = new Map<
		number,
		WritableStreamDefaultWriter<Uint8Array>
	>();
	let nextPeerStream = 0;
	/** Set by `clear`: nothing more is taken from the transport. */
	let cleared = false;
	/** The loop that takes unidirectional streams, while it waits for one to end. */
	let wakeUniAccept: (() => void) | undefined;

	/**
	 * Resolves once fewer unidirectional streams of the peer are being read
	 * than the limit, or the streams were cleared.
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
	const uniStreamSlot = async (): Promise<void> => {
		while (!cleared && peerUniReaders.size >= maxPeerUniStreams) {
			await new Promise<void>((resolve) => {
				wakeUniAccept = resolve;
			});
		}
	};
	const uniStreamEnded = (peerStream: number): void => {
		peerUniReaders.delete(peerStream);
		wakeUniAccept?.();
		wakeUniAccept = undefined;
	};

	const write = (
		stream: number,
		writer: WritableStreamDefaultWriter<Uint8Array>,
		data: Uint8Array,
	): Promise<void> => {
		trace("sent", streamName("local", stream), data);
		return writer.write(data);
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
			if (!transport.isStreamError(error)) return;
			// A reset by this endpoint has removed the stream by then.
			if (localStreams.get(stream)?.writer !== writer) return;
			void input((session) =>
				session.localStreamStopped(stream, transport.streamErrorCode(error)),
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
				if (next === undefined || next.done) return;
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
					localStreams.set(output.stream, {
						writer,
						reader: undefined,
						sendClosed: false,
					});
					watchStop(output.stream, writer);
					return { settled: write(output.stream, writer, output.data) };
				}
				case "open-bidi-stream": {
					const { stream } = output;
					const bidi = await transport.createBidirectionalStream();
					const writer = bidi.writable.getWriter();
					const reader = bidi.readable.getReader();
					localStreams.set(stream, { writer, reader, sendClosed: false });
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
					return { settled: write(stream, writer, output.data) };
				}
				case "send": {
					const local = localStreams.get(output.stream);
					if (local === undefined) {
						throw new Error(`stream ${output.stream} is not open`);
					}
					return { settled: write(output.stream, local.writer, output.data) };
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
				case "stop-sending": {
					const error = transport.streamError(
						output.code,
						"stopped by the MOQT session",
					);
					// Fails when the stream already ended, and then nothing is left
					// to stop.
					void peerUniReaders
						.get(output.peerStream)
						?.cancel(error)
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
					peerUniReaders.set(peerStream, reader);
					void pump({
						name: streamName("peer-uni", peerStream),
						reader,
						onData: (session, data, fin) =>
							session.receivePeerUniStreamData(peerStream, data, fin),
						onReset: (session) => session.peerUniStreamReset(peerStream),
						onEnd: () => uniStreamEnded(peerStream),
					});
				},
				uniStreamSlot,
			);
			void accept(
				transport.incomingBidirectionalStreams,
				(peerStream, stream) => {
					const writer = stream.writable.getWriter();
					peerBidiWriters.set(peerStream, writer);
					void pump({
						name: streamName("peer-bidi", peerStream),
						reader: stream.readable.getReader(),
						onData: (session, data, fin) =>
							session.receivePeerBidiStreamData(peerStream, data, fin),
						onReset: (session) => session.peerBidiStreamReset(peerStream),
						onEnd() {
							// The request ended without an answer from this endpoint,
							// so the send side was never used. Closing it can only fail
							// if the peer already stopped it.
							if (peerBidiWriters.delete(peerStream)) {
								void writer.close().catch(() => {});
							}
						},
					});
				},
			);
		},

		clear() {
			localStreams.clear();
			peerUniReaders.clear();
			peerBidiWriters.clear();
			cleared = true;
			wakeUniAccept?.();
			wakeUniAccept = undefined;
		},
	};
}
