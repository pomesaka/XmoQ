export interface MoqtBidirectionalStream {
	readonly readable: ReadableStream<unknown>;
	readonly writable: WritableStream<Uint8Array>;
}

/**
 * The part of a WebTransport session the connection uses. The browser's
 * `WebTransport` is adapted to it in `connect.ts`; tests supply their own.
 */
export interface MoqtTransport {
	readonly closed: Promise<{ closeCode?: number; reason?: string }>;
	readonly incomingUnidirectionalStreams: ReadableStream<
		ReadableStream<unknown>
	>;
	readonly incomingBidirectionalStreams: ReadableStream<MoqtBidirectionalStream>;
	/**
	 * The datagrams of the peer. `readable` is a `ReadableStream` of
	 * `Uint8Array` in the W3C WebTransport API; lib.dom.d.ts of TypeScript
	 * 6.0.3 declares it without a chunk type, so a chunk is checked when read.
	 */
	readonly datagrams: { readonly readable: ReadableStream<unknown> };
	createUnidirectionalStream(): Promise<WritableStream<Uint8Array>>;
	createBidirectionalStream(): Promise<MoqtBidirectionalStream>;
	close(info: { closeCode: number; reason: string }): void;
	/**
	 * The value to abort a writable or cancel a readable with, so that the
	 * stream is reset or its sender stopped with `code`.
	 */
	streamError(code: number, message: string): unknown;
	/**
	 * Whether a read or `writer.closed` failed with `error` because the peer
	 * reset or stopped that one stream. False when the whole session ended,
	 * which fails every stream too and is reported by `closed`.
	 */
	isStreamError(error: unknown): boolean;
	/**
	 * Whether a writer failed with `error` because the peer stopped that one
	 * stream and the transport did not pass the reason on: a stop whose code
	 * is not known. False for the end of the session. It is asked about a
	 * writer only: what a reader fails with was not looked at.
	 */
	isStopWithoutCode(error: unknown): boolean;
	/** Whether a stream failed with `error` because the whole session ended. */
	isSessionError(error: unknown): boolean;
	/**
	 * The error code the peer reset or stopped a stream with, from the value
	 * the stream failed with. Undefined when the value carries none.
	 */
	streamErrorCode(error: unknown): number | undefined;
}
