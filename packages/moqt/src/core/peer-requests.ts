import { concatBytes } from "./bytes.ts";
import {
	decodeControlMessage,
	decodeRequestStart,
	encodeControlMessage,
	FETCH_MESSAGE_TYPE,
	type FetchRange,
} from "./control-message.ts";
import { MoqtError, RequestErrorCode, SessionErrorCode } from "./error.ts";
import { endsBefore } from "./parameters.ts";
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
	/** The peer opened the stream; nothing of it has arrived yet. */
	open(peerStream: number): void;
	/**
	 * Gives up a stream that is known here, has not ended, and has not sent
	 * its first message whole, and says whether there was such a stream.
	 * What still arrives on it is dropped: the peer sent it before it saw
	 * that the stream was given up.
	 */
	giveUpUnidentified(peerStream: number): boolean;
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

/** The first message of the request is whole: the stream has said what it is. */
const identified = (peerStream: number): SessionOutput => ({
	type: "peer-stream-identified",
	kind: "bidi",
	peerStream,
});

/**
 * Request IDs remembered above the lowest one not seen yet, while requests
 * arrive out of order or leave a gap. Provisional.
 */
const REQUEST_IDS_AHEAD_LIMIT = 1024;

const NOT_SUPPORTED = {
	code: RequestErrorCode.NOT_SUPPORTED,
	reason: "requests from the peer are not implemented",
};

/**
 * What a FETCH of the peer is refused with, and why. No FETCH is served, so
 * every one is refused; the first two are what the session of the Rust
 * implementation answers by itself for the same FETCH, so that the same
 * bytes get the same code from both.
 */
function fetchRefusal(range: FetchRange): { code: bigint; reason: string } {
	switch (range.fetchType) {
		case "Standalone":
			// §10.12.3: "End Location MUST specify the same or a larger Location
			// than Start Location". WHY a REQUEST_ERROR and not the end of the
			// session, which the draft does not ask for either: the range is this
			// request's alone, and INVALID_RANGE is the code for it (§10.6.2).
			if (endsBefore(range.startLocation, range.endLocation)) {
				return {
					code: RequestErrorCode.INVALID_RANGE,
					reason: "the range ends before it starts",
				};
			}
			break;
		case "RelativeJoining":
		case "AbsoluteJoining":
			// §10.12.2: the Joining Request ID has to name "a subscription in
			// the same session in the Established or Pending (subscriber)
			// states", or "it MUST return a REQUEST_ERROR with error code
			// INVALID_JOINING_REQUEST_ID". Every SUBSCRIBE of the peer was
			// refused here, and this endpoint sends it no PUBLISH that is
			// accepted as a subscription of its own to join, so no ID names one.
			return {
				code: RequestErrorCode.INVALID_JOINING_REQUEST_ID,
				reason: "the Joining Request ID names no subscription",
			};
	}
	return NOT_SUPPORTED;
}

/**
 * Requests the peer opens. None is implemented, so each is answered with
 * REQUEST_ERROR after its Request ID is checked: NOT_SUPPORTED, or for a
 * FETCH, which is decoded first, what `fetchRefusal` has.
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
				const wasMissing = stream.missing;
				stream.missing = Math.max(0, stream.missing - data.length);
				if (fin && stream.missing > 0) throw endedInside();
				if (wasMissing > 0 && stream.missing === 0) {
					out.push(identified(peerStream));
				}
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
			// FIXME: serve a FETCH (§10.12). It is refused below, like a request
			// that is not decoded.
			//
			// WHY it is decoded whole first, where the others are refused on
			// their Request ID: a FETCH with a Fetch Type other than 1 to 3, or
			// with a parameter that is unknown or not for a FETCH, closes the
			// session (§10.12, §10.2.1), and the Rust implementation, which
			// decodes it, does. Refusing such a FETCH would be another answer to
			// the same bytes. WHY before the Request ID is checked: the Rust
			// implementation decodes first, so a FETCH that is wrong in both ways
			// ends the session with the same code in both.
			let refusal = NOT_SUPPORTED;
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
				if (whole.value.type === "FETCH") {
					refusal = fetchRefusal(whole.value.range);
				}
			}
			checkRequestId(requestId);
			// WHY only when the message is whole, where the request is refused
			// on its Request ID: a stream that sent the start of a request and
			// stops has not said what it is, as in the Rust implementation.
			if (missing === 0) out.push(identified(peerStream));
			// §3.3.2: reject without application processing by REQUEST_ERROR and FIN.
			out.push({
				type: "respond",
				peerStream,
				data: encodeControlMessage({
					type: "REQUEST_ERROR",
					errorCode: refusal.code,
					retryInterval: 0n,
					errorReason: refusal.reason,
				}),
			});
			out.push({
				type: "event",
				event: {
					type: "peer-request-rejected",
					messageType,
					requestId,
					errorCode: refusal.code,
				},
			});
			if (!fin) streams.set(peerStream, { answered: true, missing });
		},
		reset(peerStream) {
			streams.delete(peerStream);
		},
		open(peerStream) {
			if (streams.has(peerStream)) return;
			streams.set(peerStream, { answered: false, buffer: new Uint8Array(0) });
		},
		giveUpUnidentified(peerStream) {
			const stream = streams.get(peerStream);
			if (stream === undefined) return false;
			if (stream.answered && stream.missing === 0) return false;
			// WHY kept, in the state of a request that was answered and is
			// whole: the rest of the stream is then dropped, and its end is no
			// end inside a message. Forgotten here, what still arrives would be
			// read as the start of a request, and close the session. It goes
			// when the stream ends, like every other.
			streams.set(peerStream, { answered: true, missing: 0 });
			return true;
		},
		clear() {
			streams.clear();
			seenAhead.clear();
		},
	};
}
