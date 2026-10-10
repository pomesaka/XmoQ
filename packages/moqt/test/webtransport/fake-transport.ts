import type {
	MoqtBidirectionalStream,
	MoqtTransport,
} from "../../src/webtransport/transport.ts";

/** What a stream is reset or stopped with in the fake transport. */
export class FakeStreamError extends Error {
	readonly code: number;

	constructor(code: number, message: string) {
		super(`stream error ${code}: ${message}`);
		this.code = code;
	}
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

/** The far end of a `MoqtTransport`, driven by the test. */
export interface FakePeer {
	transport: MoqtTransport;
	/** The next unidirectional stream the client opened. */
	clientUni(): Promise<ReadableStreamDefaultReader<Uint8Array>>;
	/** The next bidirectional stream the client opened. */
	clientBidi(): Promise<PeerBidiStream>;
	openUni(): WritableStreamDefaultWriter<Uint8Array>;
	openBidi(): PeerBidiStream;
	/** Closes the transport session from the peer's side. */
	close(info: CloseInfo): void;
	/** Makes the transport session fail, as a lost connection does. */
	fail(error: Error): void;
	/** What the client closed the session with, once it did. */
	closeInfo(): CloseInfo | undefined;
}

export function createFakePeer(): FakePeer {
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
	const endStreams = (): void => {
		const error = new Error("the session ended");
		for (const end of streamEnders) end(error);
	};
	let closeInfo: CloseInfo | undefined;
	let resolveClosed: (info: CloseInfo) => void = () => {};
	let rejectClosed: (error: Error) => void = () => {};
	const closed = new Promise<CloseInfo>((resolve, reject) => {
		resolveClosed = resolve;
		rejectClosed = reject;
	});

	const transport: MoqtTransport = {
		closed,
		incomingUnidirectionalStreams: incomingUni.readable,
		incomingBidirectionalStreams: incomingBidi.readable,
		createUnidirectionalStream() {
			const stream = pipe();
			clientUnis.put(stream.readable.getReader());
			return Promise.resolve(stream.writable);
		},
		createBidirectionalStream() {
			const fromClient = pipe();
			const toClient = pipe();
			clientBidis.put({
				sent: fromClient.readable.getReader(),
				reply: toClient.writable.getWriter(),
			});
			return Promise.resolve({
				readable: toClient.readable,
				writable: fromClient.writable,
			});
		},
		close(info) {
			closeInfo = info;
			resolveClosed(info);
		},
		streamError: (code, message) => new FakeStreamError(code, message),
		isStreamError: (error) => error instanceof FakeStreamError,
		streamErrorCode: (error) =>
			error instanceof FakeStreamError ? error.code : undefined,
	};

	return {
		transport,
		clientUni: clientUnis.take,
		clientBidi: clientBidis.take,
		openUni() {
			const stream = pipe();
			void incomingUniWriter.write(stream.readable);
			return stream.writable.getWriter();
		},
		openBidi() {
			const fromClient = pipe();
			const toClient = pipe();
			void incomingBidiWriter.write({
				readable: toClient.readable,
				writable: fromClient.writable,
			});
			return {
				sent: fromClient.readable.getReader(),
				reply: toClient.writable.getWriter(),
			};
		},
		close(info) {
			endStreams();
			setTimeout(() => resolveClosed(info), 0);
		},
		fail(error) {
			endStreams();
			setTimeout(() => rejectClosed(error), 0);
		},
		closeInfo: () => closeInfo,
	};
}
