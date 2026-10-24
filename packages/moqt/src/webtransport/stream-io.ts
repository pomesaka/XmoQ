import { StreamResetErrorCode } from "../core/error.ts";
import type { Session } from "../core/session.ts";
import type { PeerStreamKind, SessionOutput } from "../core/session-output.ts";
import { StreamNotOpenedError } from "./endings.ts";
import { createStreamSlots } from "./stream-slots.ts";
import type { MoqtBidirectionalStream, MoqtTransport } from "./transport.ts";

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
	/**
	 * How long a subgroup stream of the peer, once its Subgroup Header is
	 * whole, may wait for its next bytes while it is read. One that has not
	 * sent any by then is stopped (`watch-peer-stream-idle`).
	 */
	idleSubgroupStreamTimeoutMs: number;
	/**
	 * How long a bidirectional stream of this endpoint, a request, may wait
	 * to be opened: for the peer to allow one more stream. Then what was to
	 * be written on it fails with `StreamNotOpenedError`.
	 */
	openBidiTimeoutMs: number;
	/** Called with the bytes of every stream, and of every datagram the peer sends, named `datagram`. */
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
	 * Performs one output. **A stream that is being opened is not waited
	 * for**: what is to happen on it is kept for it, in order, and the
	 * backpressure that is returned settles when that was done. WHY: the
	 * transport opens a stream when the peer allows one more, and the
	 * caller performs its outputs one after the other; waiting here would
	 * hold back everything behind it, the answers of the peer that were
	 * read among them.
	 */
	perform(output: StreamOutput): Promise<Backpressure | undefined>;
	/** Starts accepting the streams the peer opens. */
	acceptPeerStreams(): void;
	/** Starts reading the datagrams of the peer and reporting each to the session. */
	readDatagrams(): void;
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
	/** Whether the stream has the idle time, from the moment its Subgroup Header is whole. */
	watchedIdle: boolean;
	/** Runs while a read of a watched stream waits; cleared when the read returns. */
	idleTimer: ReturnType<typeof setTimeout> | undefined;
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

/**
 * How long after a refusal a request stream is asked for again
 * (**provisional**): short against the seconds a request waits, and no
 * load: one call that the browser refuses without sending anything.
 */
const OPEN_AGAIN_AFTER_MS = 50;

/** A stream this endpoint is opening, and what waits to be done on it. */
interface OpeningStream {
	/** Settles when everything asked of the stream so far was done. */
	last: Promise<void>;
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
		idleSubgroupStreamTimeoutMs,
		openBidiTimeoutMs,
		trace,
		input,
		fatal,
	} = options;
	const localStreams = new Map<number, LocalStream>();
	/** The timers of the request streams that are being opened. */
	const openTimers = new Set<ReturnType<typeof setTimeout> | undefined>();

	/**
	 * Whether a stream could not be made because the peer allows no further
	 * one for now. Chromium 156 rejects with a `DOMException` named
	 * NetworkError that is no `WebTransportError` ("Failed to create send
	 * stream.", "Failed to create bidirectional stream."; seen on
	 * 2026-10-12 against a peer that allows none). WHY by the name and not
	 * the text: the text is the browser's to change.
	 */
	const refusedForNow = (error: unknown): boolean =>
		error instanceof Error &&
		error.name === "NetworkError" &&
		!transport.isStreamError(error) &&
		!transport.isSessionError(error);

	/** The streams of this endpoint that are not open yet, or that still have something kept for them. */
	const opening = new Map<number, OpeningStream>();

	/**
	 * Has `act` done on a local stream when what was asked of it before is
	 * done; at once where nothing is. The order of what happens on one
	 * stream is the order it was asked in.
	 */
	const inTurn = (
		stream: number,
		act: () => Promise<void> | undefined,
	): Backpressure | undefined => {
		const waits = opening.get(stream);
		if (waits === undefined) {
			const settled = act();
			return settled === undefined ? undefined : { settled };
		}
		const last = waits.last.then(() => act());
		waits.last = last;
		// The stream is no longer one that waits when nothing was asked of it
		// since.
		const done = (): void => {
			if (opening.get(stream)?.last === last) opening.delete(stream);
		};
		last.then(done, done);
		return { settled: last };
	};

	/** Opens a stream of this endpoint without holding the caller: `open` registers it when the transport has made it. */
	const openInTurn = (
		stream: number,
		open: () => Promise<void>,
		data: Uint8Array,
	): Backpressure | undefined => {
		opening.set(stream, { last: open() });
		return inTurn(stream, () => {
			const local = localStreams.get(stream);
			// Reset meanwhile, by a request that was given up.
			return local === undefined ? undefined : write(local, data);
		});
	};
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
	/**
	 * Starts the time a subgroup stream has to send its next bytes, for one
	 * read. It runs from when the read is made, which is after the bytes
	 * before were taken by the session, so it counts from the stream's last
	 * bytes, or from its Subgroup Header.
	 *
	 * WHY from the last bytes and not from when the stream was taken: a
	 * stream that sends slowly but steadily, a Group published over minutes,
	 * is a way to publish, and keeps its place. What is stopped is one the
	 * peer keeps open with nothing on it, which holds one of the places for
	 * the streams read at once, and what the session held of it, for as long
	 * as the session lasts. WHY only while a read waits: a stream that
	 * waits because this endpoint does not read it, a subscription whose
	 * queue is full, has not been silent; its bytes are in the transport.
	 *
	 * The draft has no such time (§11.4 and §11.4.3 say nothing of a stream
	 * left open without bytes; §13.6 has "Implementations are advised to use
	 * timeouts to prevent resource exhaustion attacks by a peer that does
	 * not send expected data within an expected time"). A page in the
	 * background runs the timer late, never early, as for the first bytes.
	 */
	const startIdleTimer = (peerStream: number): ReturnType<typeof setTimeout> =>
		setTimeout(() => {
			void input((session) => session.peerUniStreamIdle(peerStream));
		}, idleSubgroupStreamTimeoutMs);
	const peerStreamsOf = (kind: PeerStreamKind): Map<number, PeerStream> =>
		kind === "uni" ? peerUniStreams : peerBidiStreams;
	/** The stream has ended: its timer and what was kept of it go. */
	const forgetPeerStream = (kind: PeerStreamKind, peerStream: number): void => {
		const streams = peerStreamsOf(kind);
		clearTimeout(streams.get(peerStream)?.firstBytesTimer);
		clearTimeout(streams.get(peerStream)?.idleTimer);
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
	 * for a long time and goes on sending, and a stream of no subscription
	 * that the peer does not close, each keep one. A subgroup stream that
	 * sends nothing is stopped after `idleSubgroupStreamTimeoutMs`.
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
	 * A stream on which this endpoint has finished its direction, for a
	 * request it refused, a FETCH it served or a subscription it ended with
	 * PUBLISH_DONE, has the peer's direction stopped after
	 * `peerStreamEndTimeoutMs` (`awaitPeerEndAfterRefusal` in connection.ts):
	 * as many of them as the limit would keep every later request of the
	 * peer from being read.
	 *
	 * A subgroup stream that stays open after its header holds a place among
	 * the unidirectional streams in the same way for as long as it sends: a
	 * Group that is sent slowly is a way to publish. One that sends nothing
	 * for `idleSubgroupStreamTimeoutMs` is stopped (`startIdleTimer` below).
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
		/** Called before each read, and with the read's end, whatever it brought. */
		whileReading?: { start(): void; stop(): void };
	}): Promise<void> => {
		try {
			for (;;) {
				source.whileReading?.start();
				// Only the read is guarded: it fails when the peer reset the stream
				// or the session ended. Anything else that throws in this loop is a
				// fault of this endpoint and goes to `fatal` below.
				const chunk = await source.reader
					.read()
					.catch((error: unknown) => ({ failed: true, error }) as const);
				source.whileReading?.stop();
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
					const { stream } = output;
					return openInTurn(
						stream,
						async () => {
							// WHY a stream the peer allows no room for is not
							// waited for, as a request's is: what goes on it is a
							// Subgroup of a track, and the caller is told that it
							// was not sent. Any other failure to make a stream is
							// the transport's, and the session's end.
							const writable = await transport
								.createUnidirectionalStream()
								.catch((error: unknown) => {
									if (refusedForNow(error)) throw new StreamNotOpenedError();
									fatal(toError(error));
									throw error;
								});
							if (cleared) return;
							const writer = writable.getWriter();
							localStreams.set(stream, {
								name: streamName("local", stream),
								writer,
								reader: undefined,
								sendClosed: false,
							});
							watchStop(stream, writer);
						},
						output.data,
					);
				}
				case "open-bidi-stream": {
					const { stream } = output;
					return openInTurn(
						stream,
						async () => {
							let late = false;
							let timer: ReturnType<typeof setTimeout> | undefined;
							const inTime = new Promise<undefined>((resolve) => {
								timer = setTimeout(() => {
									late = true;
									resolve(undefined);
								}, openBidiTimeoutMs);
							});
							// WHY it is asked for again and again: the WebTransport
							// API has a stream made when the peer allows one more,
							// and Chromium 156 rejects at once where the peer
							// allows none ("Failed to create bidirectional
							// stream", seen against a peer that allows no stream:
							// docs/experiments/2026-10-12-a-request-that-waits-for-a-stream.md).
							// Asked again, the request waits here as the API has
							// it wait there.
							// WHY not the end of the session, which it was: a peer
							// that has no room for a request for a moment has not
							// ended anything. A session that is gone is reported
							// by the transport, and ends this by `cleared`.
							const create = async (): Promise<MoqtBidirectionalStream> => {
								for (;;) {
									try {
										return await transport.createBidirectionalStream();
									} catch (error) {
										if (late || cleared || !refusedForNow(error)) throw error;
										await new Promise((resolve) =>
											setTimeout(resolve, OPEN_AGAIN_AFTER_MS),
										);
										if (late || cleared) throw error;
									}
								}
							};
							const creating = create();
							// WHY the stream is given up when it comes late, and
							// not used: the request it was for has failed by then.
							// A stream the transport made cannot be unmade, so it
							// is reset and stopped with nothing written on it.
							creating.then(
								(bidi) => {
									if (!late) return;
									const error = transport.streamError(
										StreamResetErrorCode.CANCELLED,
										"opened too late for its request",
									);
									void bidi.writable.abort(error).catch(() => {});
									void bidi.readable.cancel(error).catch(() => {});
								},
								() => {},
							);
							openTimers.add(timer);
							const bidi = await Promise.race([
								// Not one the peer refused for now: the transport's
								// failure, and the session's end.
								creating.catch((error: unknown) => {
									if (!late && !cleared) fatal(toError(error));
									throw error;
								}),
								inTime,
							]);
							openTimers.delete(timer);
							clearTimeout(timer);
							if (bidi === undefined) throw new StreamNotOpenedError();
							if (cleared) return;
							const writer = bidi.writable.getWriter();
							const reader = bidi.readable.getReader();
							localStreams.set(stream, {
								name: streamName("local", stream),
								writer,
								reader,
								sendClosed: false,
							});
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
						},
						output.data,
					);
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
					const { stream, data } = output;
					if (!opening.has(stream) && !localStreams.has(stream)) {
						throw new Error(`stream ${stream} is not open`);
					}
					return inTurn(stream, () => {
						const local = localStreams.get(stream);
						// Reset before its turn came.
						return local === undefined ? undefined : write(local, data);
					});
				}
				case "finish": {
					const { stream } = output;
					return inTurn(stream, () => {
						const local = localStreams.get(stream);
						if (local === undefined) return undefined;
						local.sendClosed = true;
						releaseLocalStream(stream);
						// A FIN fails when the peer already stopped or reset the
						// stream. Either way this endpoint is done sending on it.
						return local.writer.close().catch(() => {});
					});
				}
				case "reset": {
					const { stream, code } = output;
					const reset = (): undefined => {
						const local = localStreams.get(stream);
						if (local === undefined) return undefined;
						localStreams.delete(stream);
						const error = transport.streamError(
							code,
							"reset by the MOQT session",
						);
						// Both fail on a direction that is already closed or
						// errored, which is the state a reset is meant to reach.
						void local.writer.abort(error).catch(() => {});
						void local.reader?.cancel(error).catch(() => {});
						return undefined;
					};
					// WHY at once where the stream is open, also while a write on
					// it waits: a write waits for the peer's window, and a
					// stream that is given up must not wait for a peer that is
					// slow. What was kept for the stream finds it gone. Where it
					// is not open yet, it is reset when it is, with nothing
					// written. Nobody waits for that: a reset says the caller is
					// done with the stream, whatever became of it.
					if (localStreams.has(stream)) return reset();
					inTurn(stream, reset)?.settled.catch(() => {});
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
					const { stream } = output;
					// Fails when the peer's direction already ended, and then
					// nothing is left to stop.
					const stop = (): undefined => {
						void localStreams
							.get(stream)
							?.reader?.cancel(error)
							.catch(() => {});
						return undefined;
					};
					// At once where the stream is open, as a reset.
					if (localStreams.has(stream)) return stop();
					inTurn(stream, stop)?.settled.catch(() => {});
					return undefined;
				}
				case "watch-peer-stream-idle": {
					const peer = peerUniStreams.get(output.peerStream);
					if (peer !== undefined) peer.watchedIdle = true;
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

		/**
		 * WHY datagrams are read although no Object Datagram is delivered: §11
		 * has "An endpoint that receives an unknown datagram type MUST close
		 * the session", which only a reader can do. The session drops the
		 * others (`Session.receiveDatagram`).
		 *
		 * WHY one datagram at a time, the next read only after the session has
		 * taken the one before: nothing is kept here, so what waits is held by
		 * the transport and not by this endpoint. The W3C WebTransport API
		 * bounds that (Editor's Draft, read on 2026-10-24: the incoming queue
		 * is cut to its [[IncomingMaxBufferedDatagrams]] from the oldest end,
		 * which drops datagrams rather than holding the peer back, and a
		 * datagram older than `incomingMaxAge` is dropped; what Chromium does
		 * was not measured). WHY NOT a read that hands each datagram on
		 * without waiting: every one would add a call to the queue of the
		 * session's outputs, with no bound but how fast the peer sends.
		 */
		readDatagrams() {
			const read = async (): Promise<void> => {
				const reader = transport.datagrams.readable.getReader();
				for (;;) {
					// Only the read is guarded: it fails when the transport session
					// ends, which `transport.closed` reports to the connection.
					const next = await reader.read().catch(() => undefined);
					if (next === undefined || next.done || cleared) return;
					const datagram = next.value;
					if (!(datagram instanceof Uint8Array)) {
						throw new TypeError("a datagram that is not bytes was read");
					}
					trace("received", "datagram", datagram);
					await input((session) => session.receiveDatagram(datagram));
				}
			};
			read().catch((error: unknown) => fatal(toError(error)));
		},

		acceptPeerStreams() {
			void accept(
				transport.incomingUnidirectionalStreams,
				(peerStream, stream) => {
					const reader = stream.getReader();
					const peer: PeerStream = {
						reader,
						firstBytesTimer: startFirstBytesTimer("uni", peerStream),
						watchedIdle: false,
						idleTimer: undefined,
					};
					peerUniStreams.set(peerStream, peer);
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
						whileReading: {
							start() {
								if (peer.watchedIdle) {
									peer.idleTimer = startIdleTimer(peerStream);
								}
							},
							stop() {
								clearTimeout(peer.idleTimer);
								peer.idleTimer = undefined;
							},
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
						watchedIdle: false,
						idleTimer: undefined,
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
			opening.clear();
			for (const timer of openTimers) clearTimeout(timer);
			openTimers.clear();
			for (const streams of [peerUniStreams, peerBidiStreams]) {
				for (const { firstBytesTimer, idleTimer } of streams.values()) {
					clearTimeout(firstBytesTimer);
					clearTimeout(idleTimer);
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
