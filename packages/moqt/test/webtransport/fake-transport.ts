import { adaptWebTransport } from "../../src/webtransport/connect.ts";
import type {
	MoqtBidirectionalStream,
	MoqtTransport,
} from "../../src/webtransport/transport.ts";

/**
 * `WebTransportError` as Chromium 156.0.8078.4 has it (checked in a page on
 * 2026-10-10 with spikes/wt-error-form/constructor.ts): built from one
 * dictionary, a TypeError for anything else in its place, the code clamped to
 * 0 to 255, and no way for a page to set `source`. The fake is as strict as
 * the browser on purpose. A call in the form of the specification, `(message,
 * options)`, passed a more lenient fake and failed in the browser.
 */
export class FakeWebTransportError extends Error {
	readonly streamErrorCode: number | null;
	#source: "stream" | "session" = "stream";

	constructor(init: unknown = {}) {
		if (typeof init !== "object" || init === null) {
			throw new TypeError(
				"Failed to construct 'WebTransportError': The provided value is not of type 'WebTransportErrorInit'.",
			);
		}
		const message = "message" in init ? String(init.message) : "";
		super(message);
		const code = "streamErrorCode" in init ? init.streamErrorCode : null;
		// `[Clamp]` turns NaN into 0.
		this.streamErrorCode =
			typeof code === "number"
				? Math.min(255, Math.max(0, Math.trunc(code) || 0))
				: null;
	}

	get source(): "stream" | "session" {
		return this.#source;
	}

	/** What the browser fails every stream with when the session ends. */
	static sessionEnded(): FakeWebTransportError {
		const error = new FakeWebTransportError({ message: "the session ended" });
		error.#source = "session";
		return error;
	}
}

/** What the peer resets a stream or stops its sender with. */
export function fakeStreamError(
	code: number,
	message: string,
): FakeWebTransportError {
	return new FakeWebTransportError({ message, streamErrorCode: code });
}

/**
 * What a writer fails with in Chromium 156 when the peer stops its stream
 * while a write is under way and the data pipe of the stream closes before
 * the STOP_SENDING is reported: a `DOMException`, not a `WebTransportError`,
 * with no code. `writer.closed` and every write of the stream are rejected
 * with it from then on. Seen for 56 of 720 stopped streams, and for none of
 * 1080 writers of a session that ended
 * (docs/experiments/2026-10-11-write-abort-in-chromium.md).
 */
export function fakeStopWithoutCode(): DOMException {
	return new DOMException(
		"The stream was aborted by the remote server",
		"NetworkError",
	);
}

/**
 * `reason` as Chromium 156 sent it (fixtures/webtransport-close): the longest
 * run of whole characters from the start that is at most 1024 bytes of UTF-8,
 * with U+FFFD for half of a surrogate pair and a leading U+FEFF kept.
 */
function sentReason(reason: string): string {
	let sent = "";
	let bytes = 0;
	// Iterating a string yields whole characters, and half a pair on its own.
	for (const character of reason) {
		const codePoint = character.codePointAt(0) ?? 0;
		const halfOfAPair = codePoint >= 0xd800 && codePoint <= 0xdfff;
		const length =
			codePoint < 0x80
				? 1
				: codePoint < 0x800
					? 2
					: codePoint < 0x10000
						? 3
						: 4;
		if (bytes + length > 1024) break;
		bytes += length;
		sent += halfOfAPair ? "\ufffd" : character;
	}
	return sent;
}

/** Items handed over in order to whoever asks next. */
function handover<T>(): { put(item: T): void; take(): Promise<T> } {
	const items: T[] = [];
	const takers: ((item: T) => void)[] = [];
	return {
		put(item) {
			const taker = takers.shift();
			if (taker === undefined) items.push(item);
			else taker(item);
		},
		take() {
			const item = items.shift();
			if (item !== undefined) return Promise.resolve(item);
			return new Promise((resolve) => takers.push(resolve));
		},
	};
}

export interface PeerBidiStream {
	/** Bytes the client sent. */
	sent: ReadableStreamDefaultReader<Uint8Array>;
	/** Writes bytes to the client. */
	reply: WritableStreamDefaultWriter<Uint8Array>;
}

interface CloseInfo {
	closeCode: number;
	reason: string;
}

/**
 * How many datagrams the fake keeps that the client has not read. WHY a
 * bound that drops the oldest: the W3C WebTransport API cuts its incoming
 * queue so (Editor's Draft, read on 2026-10-24), so a client that lets
 * datagrams wait loses them; a fake that kept all of them would be more
 * lenient than the browser. The number is the fake's own; Chromium's was
 * not looked up.
 */
export const MAX_BUFFERED_DATAGRAMS = 8;

/**
 * The incoming datagrams of the fake: a readable with no queue of its own
 * (a high water mark of 0, as the API has), fed from a bounded one.
 */
function datagramSource(): {
	readable: ReadableStream<Uint8Array>;
	send(datagram: Uint8Array): void;
	end(error: Error): void;
} {
	const waiting: Uint8Array[] = [];
	let wanted: ((datagram: Uint8Array) => void) | undefined;
	let ended: Error | undefined;
	let fail: (error: Error) => void = () => {};
	const readable = new ReadableStream<Uint8Array>(
		{
			start(controller) {
				fail = (error) => controller.error(error);
			},
			pull(controller) {
				const datagram = waiting.shift();
				if (datagram !== undefined) {
					controller.enqueue(datagram);
					return undefined;
				}
				return new Promise<void>((resolve) => {
					wanted = (next) => {
						wanted = undefined;
						controller.enqueue(next);
						resolve();
					};
				});
			},
		},
		{ highWaterMark: 0 },
	);
	return {
		readable,
		send(datagram) {
			if (ended !== undefined) return;
			// A copy, as a datagram that crossed the network is.
			const copy = datagram.slice();
			if (wanted !== undefined) {
				wanted(copy);
				return;
			}
			waiting.push(copy);
			if (waiting.length > MAX_BUFFERED_DATAGRAMS) waiting.shift();
		},
		end(error) {
			ended = error;
			waiting.length = 0;
			fail(error);
		},
	};
}

/** The far end of a `MoqtTransport`, driven by the test. */
export interface FakePeer {
	transport: MoqtTransport;
	/** The next unidirectional stream the client opened. */
	clientUni(): Promise<ReadableStreamDefaultReader<Uint8Array>>;
	/** The next bidirectional stream the client opened. */
	clientBidi(): Promise<PeerBidiStream>;
	openUni(): WritableStreamDefaultWriter<Uint8Array>;
	openBidi(): PeerBidiStream;
	/**
	 * Sends a datagram to the client. Of the ones the client has not read,
	 * only the latest `MAX_BUFFERED_DATAGRAMS` are kept.
	 */
	sendDatagram(datagram: Uint8Array): void;
	/** Closes the transport session from the peer's side. */
	close(info: CloseInfo): void;
	/** Makes the transport session fail, as a lost connection does. */
	fail(error: Error): void;
	/** What the client closed the session with, once it did. */
	closeInfo(): CloseInfo | undefined;
	/** How many of the streams made by `openUni` the client has not taken from the transport. */
	untakenUniStreams(): number;
	/** The same for the streams made by `openBidi`. */
	untakenBidiStreams(): number;
	/** The `sendOrder` of each unidirectional stream the client asked for, in order. */
	uniSendOrders(): number[];
}

/**
 * A writable that takes a write at once while `stalls` returns false and
 * keeps it under way while it returns true, until the stream is aborted;
 * the write is then rejected with what the stream was aborted with.
 *
 * WHY `abort` of the writer is wrapped: by the Streams standard an abort
 * waits for the write that is under way, and bun 1.3.14 has no
 * `controller.signal` for a sink to hear of the abort. Chromium's stream
 * rejects the write at once.
 */
function stallingWritable(stalls: () => boolean): WritableStream<Uint8Array> {
	let rejectStalled: ((reason: unknown) => void) | undefined;
	const stream = new WritableStream<Uint8Array>({
		write: () =>
			stalls()
				? new Promise<void>((_resolve, reject) => {
						rejectStalled = reject;
					})
				: undefined,
	});
	const getWriter = stream.getWriter.bind(stream);
	stream.getWriter = () => {
		const writer = getWriter();
		const abort = writer.abort.bind(writer);
		writer.abort = (reason?: unknown) => {
			rejectStalled?.(reason);
			return abort(reason);
		};
		return writer;
	};
	return stream;
}

/**
 * `holdBidi`: `createBidirectionalStream` of the client does not resolve
 * before this has, as when the peer gives no credit for a stream. The
 * connection performs its outputs in order, so everything behind the
 * request that waits for the stream waits with it. With `holdBidiFrom`
 * the first so many streams are not held, as when the peer gave credit for
 * that many.
 *
 * `failUni`: `createUnidirectionalStream` of the client rejects with what
 * this returns, when it returns something. In Chromium that is a `DOMException` named
 * NetworkError as well ("Failed to create send stream", `web_transport.cc`),
 * the name a stopped stream has, for a stream that was never there.
 *
 * `failBidi`: the same for `createBidirectionalStream`. **This, and not
 * `holdBidi`, is what Chromium 156 does where the peer allows no further
 * stream**: it rejects at once, with a `DOMException` named NetworkError,
 * "Failed to create bidirectional stream." (seen against a peer that allows
 * none, in the interop run with `quiet peer:` in its name). `holdBidi` is
 * what the API describes.
 *
 * `stallWrites`: the unidirectional streams of the client take a write at
 * once while this returns false, and keep it under way while it returns
 * true, until the stream is aborted; the write is then rejected with what
 * the stream was aborted with. That is what Chromium 156 does to a write of
 * 8 MiB that waits for the peer's window when the page aborts the writer
 * (spikes/wt-write-abort/out/own-reset-20.txt). WHY not the pipe of the
 * other streams with a reader that does not read: in bun 1.3.14 `abort()`
 * of such a stream waits for the write that is under way and settles
 * nothing. The peer's side of these streams reads nothing.
 */
export function createFakePeer(
	options: {
		holdBidi?: Promise<void>;
		holdBidiFrom?: number;
		failUni?: () => unknown;
		failBidi?: () => unknown;
		stallWrites?: () => boolean;
	} = {},
): FakePeer {
	let bidisAsked = 0;
	const uniSendOrders: number[] = [];
	const clientUnis = handover<ReadableStreamDefaultReader<Uint8Array>>();
	const clientBidis = handover<PeerBidiStream>();
	const incomingUni = new TransformStream<
		ReadableStream<unknown>,
		ReadableStream<unknown>
	>({}, undefined, { highWaterMark: 1024 });
	const incomingBidi = new TransformStream<
		MoqtBidirectionalStream,
		MoqtBidirectionalStream
	>({}, undefined, { highWaterMark: 1024 });
	const incomingUniWriter = incomingUni.writable.getWriter();
	const openedUnis: ReadableStream<Uint8Array>[] = [];
	const openedBidis: ReadableStream<Uint8Array>[] = [];
	const incomingBidiWriter = incomingBidi.writable.getWriter();
	/** Ends every stream made so far, as the end of the session does. */
	const streamEnders: ((error: Error) => void)[] = [];
	/** A stream pair whose writes complete without waiting for the reader. */
	const pipe = (): TransformStream<Uint8Array, Uint8Array> =>
		new TransformStream<Uint8Array, Uint8Array>(
			{
				start(controller) {
					streamEnders.push((error) => controller.error(error));
				},
			},
			undefined,
			{ highWaterMark: 1024 },
		);
	// WHY the streams fail a task before `closed` settles: that is the order in
	// which the connection can see the end of a session, and the one it must
	// not take for resets and STOP_SENDINGs of the peer.
	const datagrams = datagramSource();
	const endStreams = (): void => {
		const error = FakeWebTransportError.sessionEnded();
		for (const end of streamEnders) end(error);
		datagrams.end(error);
	};
	let closeInfo: CloseInfo | undefined;
	let resolveClosed: (info: CloseInfo) => void = () => {};
	let rejectClosed: (error: Error) => void = () => {};
	const closed = new Promise<CloseInfo>((resolve, reject) => {
		resolveClosed = resolve;
		rejectClosed = reject;
	});

	// WHY through the adapter of the browser: what it does to errors is then
	// part of every test of the connection.
	const transport = adaptWebTransport(
		{
			closed,
			incomingUnidirectionalStreams: incomingUni.readable,
			incomingBidirectionalStreams: incomingBidi.readable,
			datagrams: { readable: datagrams.readable },
			createUnidirectionalStream({ sendOrder }) {
				uniSendOrders.push(sendOrder);
				const failure = options.failUni?.();
				if (failure !== undefined) return Promise.reject(failure);
				if (options.stallWrites !== undefined) {
					clientUnis.put(new ReadableStream<Uint8Array>().getReader());
					return Promise.resolve(stallingWritable(options.stallWrites));
				}
				const stream = pipe();
				clientUnis.put(stream.readable.getReader());
				return Promise.resolve(stream.writable);
			},
			async createBidirectionalStream() {
				const failure = options.failBidi?.();
				if (failure !== undefined) throw failure;
				const nth = bidisAsked;
				bidisAsked += 1;
				if (nth >= (options.holdBidiFrom ?? 0)) await options.holdBidi;
				const fromClient = pipe();
				const toClient = pipe();
				clientBidis.put({
					sent: fromClient.readable.getReader(),
					reply: toClient.writable.getWriter(),
				});
				return {
					readable: toClient.readable,
					writable: fromClient.writable,
				};
			},
			// As Chromium 156 (spikes/wt-close-info): the code is wrapped to 32
			// bits, where NaN is 0, and the peer gets the reason cut to 1024
			// bytes of UTF-8, while `closed` resolves with the whole reason.
			close(info) {
				const closeCode = info.closeCode >>> 0;
				closeInfo = { closeCode, reason: sentReason(info.reason) };
				resolveClosed({ closeCode, reason: info.reason });
			},
		},
		FakeWebTransportError,
	);

	return {
		transport,
		clientUni: clientUnis.take,
		clientBidi: clientBidis.take,
		openUni() {
			const stream = pipe();
			openedUnis.push(stream.readable);
			void incomingUniWriter.write(stream.readable);
			return stream.writable.getWriter();
		},
		openBidi() {
			const fromClient = pipe();
			const toClient = pipe();
			openedBidis.push(toClient.readable);
			void incomingBidiWriter.write({
				readable: toClient.readable,
				writable: fromClient.writable,
			});
			return {
				sent: fromClient.readable.getReader(),
				reply: toClient.writable.getWriter(),
			};
		},
		sendDatagram: datagrams.send,
		close(info) {
			endStreams();
			setTimeout(() => resolveClosed(info), 0);
		},
		fail(error) {
			endStreams();
			setTimeout(() => rejectClosed(error), 0);
		},
		closeInfo: () => closeInfo,
		uniSendOrders: () => [...uniSendOrders],
		// The client locks a stream when it starts to read it.
		untakenUniStreams: () =>
			openedUnis.filter((readable) => !readable.locked).length,
		untakenBidiStreams: () =>
			openedBidis.filter((readable) => !readable.locked).length,
	};
}
