import { concatBytes } from "./bytes.ts";
import {
	decodeControlMessage,
	decodeRequestStart,
	encodeControlMessage,
	FETCH_MESSAGE_TYPE,
} from "./control-message.ts";
import { MoqtError, RequestErrorCode, SessionErrorCode } from "./error.ts";
import type { SessionOutput } from "./session-output.ts";

export interface PeerRequests {
	/**
	 * §10.1: records a Request ID the peer used. Throws the `MoqtError` for a
	 * wrong parity or a duplicate. A gap in the IDs is not an error.
	 */
	checkRequestId(requestId: bigint): void;
	/** Bytes on a bidirectional stream the peer opened. Throws the `MoqtError` that ends the session. */
	receive(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): void;
	reset(peerStream: number): void;
	clear(): void;
}

type PeerRequestStream =
	/** Message Type and Request ID have not fully arrived. */
	| { answered: false; buffer: Uint8Array }
	/**
	 * The request was answered; the rest of it is dropped. `missing` is how
	 * many bytes of its first message have not arrived yet.
	 */
	| { answered: true; missing: number };

/**
 * Request IDs remembered above the lowest one not seen yet, while requests
 * arrive out of order or leave a gap. Provisional.
 */
const REQUEST_IDS_AHEAD_LIMIT = 1024;

/**
 * Requests the peer opens. None is implemented, so each is answered with
 * REQUEST_ERROR NOT_SUPPORTED after its Request ID is checked; a FETCH is
 * decoded first.
 */
export function createPeerRequests(): PeerRequests {
	const streams = new Map<number, PeerRequestStream>();
	// §10.1: the server generates odd Request IDs starting at 1. This endpoint
	// is always the client.
	let lowestUnseen = 1n;
	/** Request IDs seen above `lowestUnseen`, while lower ones are still to come. */
	const seenAhead = new Set<bigint>();

	/**
	 * §10.1: wrong parity or a duplicate closes the session with
	 * INVALID_REQUEST_ID. The draft asks nothing else of the receiver, so a gap
	 * in the IDs is never an error: a request the peer reset before its bytes
	 * arrived leaves one for good, and a request that is only late fills it.
	 */
	const checkRequestId = (requestId: bigint): void => {
		if (requestId % 2n !== 1n) {
			throw new MoqtError(
				SessionErrorCode.INVALID_REQUEST_ID,
				`Request ID ${requestId} from the server is even`,
			);
		}
		if (requestId < lowestUnseen || seenAhead.has(requestId)) {
			throw new MoqtError(
				SessionErrorCode.INVALID_REQUEST_ID,
				`duplicate Request ID ${requestId}`,
			);
		}
		seenAhead.add(requestId);
		while (seenAhead.delete(lowestUnseen)) lowestUnseen += 2n;
		if (seenAhead.size <= REQUEST_IDS_AHEAD_LIMIT) return;
		// Too many IDs are remembered above a gap. Each new ID now pushes out
		// the lowest remembered one, and a duplicate of an ID that old is no
		// longer detected.
		// WHY NOT give up the gap and count everything below as seen: a request
		// whose ID lies in the gap can still arrive, and it would then be
		// reported as a duplicate although the peer did nothing wrong.
		let lowestSeen = requestId;
		for (const seen of seenAhead) if (seen < lowestSeen) lowestSeen = seen;
		seenAhead.delete(lowestSeen);
	};

	return {
		checkRequestId,
		receive(peerStream, data, fin, out) {
			const stream = streams.get(peerStream) ?? {
				answered: false,
				buffer: new Uint8Array(0),
			};
			if (fin) streams.delete(peerStream);
			else streams.set(peerStream, stream);
			// A stream that ends inside its first message is treated like a
			// Message Length that its payload does not match, a
			// PROTOCOL_VIOLATION (§10), as the Rust implementation does
			// (`MessageDecoder::finish`). One that ends without a byte asked for
			// nothing.
			const endedInside = (): MoqtError =>
				new MoqtError(
					SessionErrorCode.PROTOCOL_VIOLATION,
					"a request stream ended inside its first message",
				);
			if (stream.answered) {
				stream.missing = Math.max(0, stream.missing - data.length);
				if (fin && stream.missing > 0) throw endedInside();
				return;
			}

			const buffer = concatBytes([stream.buffer, data]);
			const decoded = decodeRequestStart(buffer);
			if (decoded.status === "error") throw decoded.error;
			if (decoded.status === "incomplete") {
				if (fin && buffer.length > 0) throw endedInside();
				stream.buffer = buffer;
				return;
			}
			const { messageType, requestId, messageLength } = decoded.value;
			const missing = Math.max(0, messageLength - buffer.length);
			// Before the request is looked at: a message that will never be
			// whole is not answered.
			if (fin && missing > 0) throw endedInside();
			// FIXME: serve a FETCH (§10.12). It is refused with NOT_SUPPORTED
			// below, like a request that is not decoded.
			//
			// WHY it is decoded whole first, where the others are refused on
			// their Request ID: a FETCH with a Fetch Type other than 1 to 3, or
			// with a parameter that is unknown or not for a FETCH, closes the
			// session (§10.12, §10.2.1), and the Rust implementation, which
			// decodes it, does. Refusing such a FETCH would be another answer to
			// the same bytes. WHY before the Request ID is checked: the Rust
			// implementation decodes first, so a FETCH that is wrong in both ways
			// ends the session with the same code in both.
			if (messageType === FETCH_MESSAGE_TYPE) {
				const whole = decodeControlMessage(buffer);
				if (whole.status === "error") throw whole.error;
				if (whole.status === "incomplete") {
					// At most 9 + 2 + 65535 bytes of one stream: Message Length is
					// 16 bits.
					//
					// How many streams hold that much at once is bounded where the
					// streams are read: `maxPeerBidiStreams` of the connection, 64
					// by default (`webtransport/stream-io.ts`). A session that is
					// driven without that layer has no such bound.
					stream.buffer = buffer;
					return;
				}
			}
			checkRequestId(requestId);
			// §3.3.2: reject without application processing by REQUEST_ERROR and FIN.
			out.push({
				type: "respond",
				peerStream,
				data: encodeControlMessage({
					type: "REQUEST_ERROR",
					errorCode: RequestErrorCode.NOT_SUPPORTED,
					retryInterval: 0n,
					errorReason: "requests from the peer are not implemented",
				}),
			});
			out.push({
				type: "event",
				event: { type: "peer-request-rejected", messageType, requestId },
			});
			if (!fin) streams.set(peerStream, { answered: true, missing });
		},
		reset(peerStream) {
			streams.delete(peerStream);
		},
		clear() {
			streams.clear();
			seenAhead.clear();
		},
	};
}
