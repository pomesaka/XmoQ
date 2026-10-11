import { concatBytes } from "./bytes.ts";
import {
	decodeControlMessage,
	decodeRequestStart,
	encodeControlMessage,
	FETCH_MESSAGE_TYPE,
	type Fetch,
	type FetchRange,
	SUBSCRIBE_MESSAGE_TYPE,
	type Subscribe,
} from "./control-message.ts";
import { MoqtError, RequestErrorCode, SessionErrorCode } from "./error.ts";
import { endsBefore } from "./parameters.ts";
import type { PeerSubscribeOpened } from "./requests.ts";
import type { SessionOutput } from "./session-output.ts";

export interface PeerRequestsOptions {
	/**
	 * A SUBSCRIBE of the peer, whole, with its Request ID checked. Says
	 * whether it is refused, or taken over with its stream.
	 */
	subscribe(
		message: Subscribe,
		peerStream: number,
		out: SessionOutput[],
	): PeerSubscribeOpened;
	/**
	 * A Standalone FETCH of the peer, whole, with its Request ID checked and
	 * a range that does not end before it starts. Says whether it is
	 * refused, or taken over with its stream.
	 */
	fetch(
		message: Fetch,
		peerStream: number,
		out: SessionOutput[],
	): PeerSubscribeOpened;
	/**
	 * Whether the Request ID names a SUBSCRIBE of the peer that waits for its
	 * answer or was accepted, and is not over: the one kind of request of
	 * the peer that stays open here.
	 */
	isOpenSubscribe(requestId: bigint): boolean;
}

/** A stream that was taken over with its request: what it is named from here on, and the bytes that followed the request. */
export interface AdoptedPeerStream {
	stream: number;
	rest: Uint8Array;
}

export interface PeerRequests {
	/**
	 * §10.1: records a Request ID the peer used. Throws the `MoqtError` for a
	 * wrong parity or a duplicate. A gap in the IDs is not an error.
	 */
	checkRequestId(requestId: bigint): void;
	/**
	 * Bytes on a bidirectional stream the peer opened. Throws the `MoqtError`
	 * that ends the session. Returns something when the request was taken
	 * over with its stream: nothing of that stream is kept here then, and the
	 * caller passes on what arrives on it, starting with `rest` and with
	 * `fin`.
	 */
	receive(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): AdoptedPeerStream | undefined;
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
	/** How many streams something is kept of. */
	heldStreams(): number;
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
 * What a FETCH of the peer is refused with before anyone is asked, and why;
 * `undefined` for a Standalone FETCH that is handed on.
 *
 * INVALID_RANGE and INVALID_JOINING_REQUEST_ID are what the session of the
 * Rust implementation answers by itself for the same FETCH
 * (`fixtures/peer-fetch/`, `fixtures/joining-request-id/`). A Joining FETCH
 * that the Rust session hands to its caller is refused here with
 * NOT_SUPPORTED, which is what the Rust client, the caller that serves no
 * FETCH, answers it with: the same bytes get the same code from a client of
 * either implementation.
 */
function fetchRefusal(
	range: FetchRange,
	isOpenSubscribe: (requestId: bigint) => boolean,
	isYetToCome: (requestId: bigint) => boolean,
): { code: bigint; reason: string } | undefined {
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
			return undefined;
		case "RelativeJoining":
		case "AbsoluteJoining":
			// §10.12.2: the Joining Request ID has to name "a subscription in
			// the same session in the Established or Pending (subscriber)
			// states", or "it MUST return a REQUEST_ERROR with error code
			// INVALID_JOINING_REQUEST_ID". A SUBSCRIBE of the peer is such a
			// subscription until it is over; a PUBLISH of this endpoint is not
			// taken as one (the notes, known gaps), as in the Rust session.
			//
			// A Joining Request ID whose request is yet to come is no reason
			// for that code: the SUBSCRIBE travels on a stream of its own and
			// can be read after the FETCH. The Rust session hands such a FETCH
			// to its caller, and a relay holds it for the SUBSCRIBE.
			if (
				!isOpenSubscribe(range.joiningRequestId) &&
				!isYetToCome(range.joiningRequestId)
			) {
				return {
					code: RequestErrorCode.INVALID_JOINING_REQUEST_ID,
					reason: "the Joining Request ID names no subscription",
				};
			}
			// WHY a Joining FETCH is not handed on, where the Rust session
			// hands it to its caller: a relay turns a viewer's Joining FETCH
			// into a Standalone one before it asks a publisher, so a publisher
			// behind a relay gets none. The Rust client refuses it the same.
			// WHY at once for one whose SUBSCRIBE is yet to come, where a relay
			// waits for it: what would be answered after the wait is this.
			return {
				code: RequestErrorCode.NOT_SUPPORTED,
				reason: "a Joining FETCH is not served",
			};
	}
}

/** Why the session itself refuses a SUBSCRIBE of the peer. */
const SUBSCRIBE_REFUSALS = new Map<bigint, string>([
	[
		RequestErrorCode.DUPLICATE_SUBSCRIPTION,
		"this endpoint is already the publisher of a subscription to the Track",
	],
	// FIXME: give a Retry Interval. §10.6.2 has the sender of EXCESSIVE_LOAD
	// say when to retry, and 0, which every refusal here carries, says "do
	// not retry". The Rust session has the same.
	[RequestErrorCode.EXCESSIVE_LOAD, "too many requests of the peer are open"],
]);

/**
 * Requests the peer opens, until it is known what becomes of them. A
 * SUBSCRIBE is handed on (`PeerRequestsOptions.subscribe`). Every other
 * request is answered here with REQUEST_ERROR after its Request ID is
 * checked: NOT_SUPPORTED, or for a FETCH, which is decoded first, what
 * `fetchRefusal` has.
 */
export function createPeerRequests(options: PeerRequestsOptions): PeerRequests {
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
		// WHY the open requests are asked as well: the ID of one can have
		// been pushed out of the memory below, and taking it for new would
		// put a second request in the place of the first, which could then
		// not be ended. The Rust session looks at its open requests in the
		// same way (`take_id`).
		if (
			requestId < lowestUnseen ||
			seenAhead.has(requestId) ||
			options.isOpenSubscribe(requestId)
		) {
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

	/**
	 * Whether the request with this ID is yet to come, for a request with
	 * the ID `before` that names it: one of the peer's IDs, smaller than
	 * that of the request that names it (§10.1: "Each endpoint increments its
	 * Request ID by 2 for each new request", so a request that names another
	 * was made after it), and not used. The rule of both implementations
	 * (`fixtures/joining-request-id/`).
	 *
	 * An ID that was pushed out of what is remembered reads as yet to come.
	 */
	const isYetToCome = (requestId: bigint, before: bigint): boolean =>
		requestId % 2n === 1n &&
		requestId < before &&
		requestId >= lowestUnseen &&
		!seenAhead.has(requestId);

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
				return undefined;
			}

			const buffer = concatBytes([stream.buffer, data]);
			const decoded = decodeRequestStart(buffer);
			if (decoded.status === "error") throw decoded.error;
			if (decoded.status === "incomplete") {
				if (fin && buffer.length > 0) throw endedInside();
				stream.buffer = buffer;
				return undefined;
			}
			const { messageType, requestId, messageLength } = decoded.value;
			const missing = Math.max(0, messageLength - buffer.length);
			// Before the request is looked at: a message that will never be
			// whole is not answered.
			if (fin && missing > 0) throw endedInside();
			// FIXME: serve a FETCH (§10.12). It is refused below, like a request
			// that is not decoded.
			//
			// WHY a FETCH and a SUBSCRIBE are decoded whole first, where the
			// others are refused on their Request ID: a FETCH with a Fetch Type
			// other than 1 to 3, and either with a parameter that is unknown or
			// not for its message, closes the session (§10.12, §10.2.1), and
			// the Rust implementation, which decodes them, does. Refusing such a
			// request would be another answer to the same bytes. WHY before the
			// Request ID is checked: the Rust implementation decodes first, so
			// a request that is wrong in both ways ends the session with the
			// same code in both.
			let refusal = NOT_SUPPORTED;
			let handedOn:
				| { message: Subscribe | Fetch; bytesRead: number }
				| undefined;
			if (
				messageType === FETCH_MESSAGE_TYPE ||
				messageType === SUBSCRIBE_MESSAGE_TYPE
			) {
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
					return undefined;
				}
				// WHY the Request ID of a FETCH is used before the one it names is
				// looked at: using it can push the lowest ID out of what is
				// remembered, and the Rust session, which takes the ID first
				// (`on_peer_request_opened`), then reads that ID as yet to come.
				checkRequestId(requestId);
				if (whole.value.type === "FETCH") {
					const refused = fetchRefusal(
						whole.value.range,
						options.isOpenSubscribe,
						(joined) => isYetToCome(joined, requestId),
					);
					if (refused !== undefined) refusal = refused;
					else handedOn = { message: whole.value, bytesRead: whole.bytesRead };
				}
				if (whole.value.type === "SUBSCRIBE") {
					handedOn = { message: whole.value, bytesRead: whole.bytesRead };
				}
			} else {
				checkRequestId(requestId);
			}
			// WHY only when the message is whole, where the request is refused
			// on its Request ID: a stream that sent the start of a request and
			// stops has not said what it is, as in the Rust implementation.
			if (missing === 0) out.push(identified(peerStream));
			if (handedOn !== undefined) {
				const opened =
					handedOn.message.type === "FETCH"
						? options.fetch(handedOn.message, peerStream, out)
						: options.subscribe(handedOn.message, peerStream, out);
				if ("stream" in opened) {
					streams.delete(peerStream);
					return {
						stream: opened.stream,
						rest: buffer.subarray(handedOn.bytesRead),
					};
				}
				refusal = {
					code: opened.refused,
					reason: SUBSCRIBE_REFUSALS.get(opened.refused) ?? "",
				};
			}
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
			return undefined;
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
		heldStreams: () => streams.size,
		clear() {
			streams.clear();
			seenAhead.clear();
		},
	};
}
