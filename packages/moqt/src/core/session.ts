import { concatBytes, createByteReader, ShortBufferError } from "./bytes.ts";
import { encodeControlMessage, SETUP_MESSAGE_TYPE } from "./control-message.ts";
import {
	createPeerControlStream,
	SETUP_OPTION_AUTHORIZATION_TOKEN,
	SETUP_OPTION_MOQT_IMPLEMENTATION,
} from "./control-stream.ts";
import { classifyDatagram } from "./datagram.ts";
import { MoqtError, protocolViolation, StreamResetErrorCode } from "./error.ts";
import {
	FETCH_HEADER_STREAM_TYPE,
	type FetchObject,
	type RangeEnd,
} from "./fetch.ts";
import type { KeyValuePair } from "./key-value-pair.ts";
import { resolveLimits, type SessionLimits } from "./limits.ts";
import type { OutgoingObject, SubgroupOptions } from "./outgoing-subgroups.ts";
import type { Location, MessageParameters } from "./parameters.ts";
import { createPeerRequests } from "./peer-requests.ts";
import {
	createRequests,
	type FetchAnswer,
	type FetchRequest,
	type PublishNamespaceRequest,
	type PublishRequest,
	type Requests,
	type SubscribeAnswer,
	type SubscribeNamespaceRequest,
	type SubscribeRequest,
	type TrackStatusRequest,
} from "./requests.ts";
import {
	CONTROL_STREAM_SEND_ORDER,
	type PeerStreamKind,
	type SessionOutput,
	type SessionResult,
} from "./session-output.ts";
import { isSubgroupHeaderType } from "./subgroup.ts";

export interface SessionOptions {
	/** Value of the MOQT_IMPLEMENTATION Setup Option (§10.3.1.5). Not sent when undefined. */
	implementation?: string;
	/**
	 * Value of the AUTHORIZATION TOKEN Setup Option (§10.3.1.4), a Token
	 * structure as bytes. Not sent when undefined.
	 */
	authorizationToken?: Uint8Array;
	/** Limits to use in place of the defaults; the ones left out keep their default. */
	limits?: Partial<SessionLimits>;
}

/**
 * MOQT session state for the client end of a WebTransport session, without
 * I/O: every method takes what happened and returns what to do. The caller
 * performs the outputs in order.
 *
 * Methods that report input never throw for what the peer did: a broken rule
 * becomes a `close-session` output. Methods the application calls throw when
 * they are misused.
 *
 * Not implemented: the server role; requests opened by the peer other than
 * SUBSCRIBE, a Standalone FETCH and TRACK_STATUS (they are answered with
 * REQUEST_ERROR, a Joining FETCH with NOT_SUPPORTED);
 * delivering Object Datagrams (their type is checked, `receiveDatagram`),
 * SUBSCRIBE_TRACKS and timers. Of a FETCH this endpoint
 * sends, the fetch stream is handed on as bytes; `fetch.ts` decodes them for
 * whoever wants Objects.
 */
export interface Session {
	readonly closed: boolean;
	/** Opens the control stream with SETUP (§3.3). */
	start(): SessionOutput[];
	/** Each request method returns the Request ID it used. */
	publishNamespace(request: PublishNamespaceRequest): SessionResult<bigint>;
	publish(request: PublishRequest): SessionResult<bigint>;
	subscribe(request: SubscribeRequest): SessionResult<bigint>;
	/** Sends a FETCH (§10.12). `Requests.fetch` has when the request is over. */
	fetch(request: FetchRequest): SessionResult<bigint>;
	/** Sends a TRACK_STATUS (§10.14). `Requests.trackStatus` has its answer. */
	trackStatus(request: TrackStatusRequest): SessionResult<bigint>;
	/** Sends a SUBSCRIBE_NAMESPACE (§10.18). `Requests.subscribeNamespace` has what comes of it. */
	subscribeNamespace(request: SubscribeNamespaceRequest): SessionResult<bigint>;
	/** Cancels a request this endpoint made (§3.3.2). */
	cancelRequest(requestId: bigint): SessionOutput[];
	/** `Requests.updateSubscription`: a REQUEST_UPDATE for an Established SUBSCRIBE of this endpoint (§10.9). */
	updateSubscription(
		requestId: bigint,
		parameters: MessageParameters,
	): SessionOutput[];
	/**
	 * Accepts a SUBSCRIBE of the peer that the event `peer-subscribe`
	 * reported: sends SUBSCRIBE_OK (§10.8). Returns the Track Alias, which
	 * the session chooses. The subscription is then used like one made with
	 * PUBLISH, under the peer's Request ID: `openSubgroup`, `publishDone`,
	 * and the events of its end.
	 *
	 * Throws an `InactiveError` for a request that does not wait for its
	 * answer, and a `RangeError` for a parameter that is none of SUBSCRIBE_OK.
	 */
	acceptSubscribe(
		requestId: bigint,
		answer: SubscribeAnswer,
	): SessionResult<bigint>;
	/**
	 * Answers a TRACK_STATUS of the peer that the event `peer-track-status`
	 * reported: REQUEST_OK and a FIN (§10.14). Throws an `InactiveError` for
	 * a request that does not wait for its answer, and a `RangeError` for a
	 * parameter other than LARGEST_OBJECT (§10.2.11).
	 */
	acceptTrackStatus(
		requestId: bigint,
		answer: SubscribeAnswer,
	): SessionOutput[];
	/**
	 * Refuses a request of the peer that waits for its answer: REQUEST_ERROR
	 * and a FIN (§3.3.2). Throws an `InactiveError` for a request that does
	 * not.
	 */
	rejectPeerRequest(
		requestId: bigint,
		errorCode: bigint,
		errorReason: string,
	): SessionOutput[];
	/**
	 * Accepts a Standalone FETCH of the peer that the event `peer-fetch`
	 * reported: sends FETCH_OK, a REQUEST_OK for each REQUEST_UPDATE that
	 * came before, and opens the fetch stream with its FETCH_HEADER. The
	 * Objects are then written with `writeFetchObject` and
	 * `writeFetchRangeEnd`, and `finishFetch` ends the FETCH.
	 *
	 * Throws an `InactiveError` for a request that does not wait for its
	 * answer, and a `RangeError`, with the FETCH still waiting, for an End
	 * Location before the Start Location of the FETCH (§10.13).
	 */
	acceptFetch(requestId: bigint, answer: FetchAnswer): SessionOutput[];
	/**
	 * Writes the next Object to the fetch stream of an accepted FETCH. Throws
	 * a `RangeError`, with nothing written and the stream as it was, for an
	 * Object that does not follow the one before it by the Group Order of
	 * the FETCH.
	 */
	writeFetchObject(requestId: bigint, object: FetchObject): SessionOutput[];
	/** Writes an End of Range indicator to that stream (§11.4.4.2). */
	writeFetchRangeEnd(
		requestId: bigint,
		kind: RangeEnd,
		location: Location,
	): SessionOutput[];
	/**
	 * Finishes the fetch stream and the request stream: the FETCH is served.
	 * How the peer then ends its direction of the request stream is reported
	 * as after PUBLISH_DONE (`peer-direction-ended`), and
	 * `expirePeerEnd` ends the wait for it.
	 */
	finishFetch(requestId: bigint): SessionOutput[];
	/** Gives an accepted FETCH up: resets both of its streams with `code` (§5.2). */
	resetFetch(requestId: bigint, code: number): SessionOutput[];
	/**
	 * Ends a subscription this endpoint is the publisher of: sends
	 * PUBLISH_DONE and a FIN (§10.11). Every subgroup stream of the
	 * subscription has to be closed first.
	 */
	publishDone(
		requestId: bigint,
		statusCode: bigint,
		errorReason: string,
	): SessionOutput[];
	/**
	 * The wait for late streams after PUBLISH_DONE is over (§10.11). The timer
	 * belongs to the caller; the session has none.
	 */
	expireSubscription(requestId: bigint): SessionOutput[];
	/** The wait for the streams of a subscription whose REQUEST_UPDATE was refused ran out (`update-refused`). */
	expireUpdateRefusal(requestId: bigint): SessionOutput[];
	/**
	 * The wait for the peer to end its direction of a request stream is
	 * over, on which this endpoint has finished its own: after `publishDone`,
	 * after `finishFetch`, and after a request of the peer was refused, by
	 * `rejectPeerRequest` or by the session itself (`peer-request-rejected`).
	 * The timer belongs to the caller. Stops that direction (CANCELLED), and
	 * a request of the peer that was kept until its stream is gone is kept
	 * no longer; does nothing when the direction has ended
	 * (`peer-direction-ended`).
	 */
	expirePeerEnd(requestId: bigint): SessionOutput[];
	/**
	 * The wait for the subscriber of a FETCH that was served with
	 * `finishFetch` to end its direction of the request stream first is
	 * over: this endpoint finishes its own. The timer belongs to the caller,
	 * and the wait of `expirePeerEnd` begins after it. Does nothing when the
	 * direction is finished.
	 */
	finishServedFetchSend(requestId: bigint): SessionOutput[];
	/** Opens a subgroup stream with its SUBGROUP_HEADER (§11.4.2). Returns the stream number. */
	openSubgroup(
		requestId: bigint,
		options: SubgroupOptions,
	): SessionResult<number>;
	writeObject(stream: number, object: OutgoingObject): SessionOutput[];
	/** Closes a subgroup stream with a FIN: every Object of the subgroup was written (§11.4.3). */
	closeSubgroup(stream: number): SessionOutput[];
	/** Resets a subgroup stream that will not carry every Object of the subgroup (§11.4.3). */
	resetSubgroup(stream: number, code: number): SessionOutput[];
	/** A subgroup stream could not be opened by the transport: it is forgotten, and not counted among the streams of its subscription. */
	subgroupNotOpened(stream: number): SessionOutput[];
	/**
	 * The peer sent STOP_SENDING for a stream this endpoint opened. `code` is
	 * its error code when the transport reports one.
	 */
	localStreamStopped(stream: number, code?: number): SessionOutput[];
	/**
	 * The transport ended this endpoint's sending on a stream it opened and
	 * did not say why, in a way that is read as a STOP_SENDING of the peer
	 * with an unknown code. Like `localStreamStopped` without a code, but for
	 * the control stream, where it does nothing and closes nothing: that
	 * reading is not certain, and it must not make a violation of the peer.
	 */
	localStreamAborted(stream: number): SessionOutput[];
	/** Bytes the peer sent on a request stream this endpoint opened. */
	receiveRequestStreamData(
		stream: number,
		data: Uint8Array,
		fin: boolean,
	): SessionOutput[];
	/**
	 * The peer reset its side of a request stream, or sent STOP_SENDING for it.
	 * `code` is the error code of the reset when the transport reports one.
	 */
	requestStreamReset(stream: number, code?: number): SessionOutput[];
	/** Bytes on a unidirectional stream the peer opened. */
	receivePeerUniStreamData(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
	): SessionOutput[];
	/**
	 * The peer reset a unidirectional stream it opened. `code` is the error
	 * code of the reset, when the transport reported one.
	 */
	peerUniStreamReset(peerStream: number, code?: number): SessionOutput[];
	/** Bytes on a bidirectional stream the peer opened. */
	receivePeerBidiStreamData(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
	): SessionOutput[];
	/**
	 * The peer reset a bidirectional stream it opened. `code` is the error
	 * code of the reset, when the transport reported one.
	 */
	peerBidiStreamReset(peerStream: number, code?: number): SessionOutput[];
	/**
	 * A datagram of the peer (§11). One whose type is not allowed closes the
	 * session (`classifyDatagram`); any other is dropped, since Object
	 * Datagrams are not delivered.
	 */
	receiveDatagram(datagram: Uint8Array): SessionOutput[];
	/**
	 * The peer opened a stream, and nothing of it was reported yet. Without
	 * this the session does not know a stream that has sent no byte, and
	 * `peerStreamTimedOut` does nothing for a stream it does not know.
	 *
	 * Once for a stream, before any of its bytes and before its end. Called
	 * for a stream that has ended, it leaves an entry that nothing removes
	 * before the session ends.
	 */
	peerStreamOpened(kind: PeerStreamKind, peerStream: number): void;
	/**
	 * The time a stream of the peer has to say what it is ran out. If it has
	 * not said it, the outputs stop it, and what the session held of it is
	 * dropped, as for a stream that is reset before it said what it is
	 * (§11.4.1); the session stays open.
	 *
	 * Does nothing for a stream that has said it, which the caller may not
	 * have learned yet; for a stream that has ended, or was stopped by an
	 * earlier call; and for a stream the session was never told of. So a
	 * caller that reports the time late, or twice, does no harm.
	 */
	peerStreamTimedOut(kind: PeerStreamKind, peerStream: number): SessionOutput[];
	/**
	 * A subgroup stream of the peer, watched since its Subgroup Header was
	 * whole (`watch-peer-stream-idle`), has sent no byte for the idle time.
	 * The outputs stop it with INTERNAL_ERROR and drop what the session held
	 * of it; the session and the subscription stay. Does nothing for a stream
	 * that is no subgroup stream, has ended, or was stopped before.
	 */
	peerUniStreamIdle(peerStream: number): SessionOutput[];
	/**
	 * How many streams the session keeps something of: request streams of
	 * either side, and the peer's streams by the numbers the I/O layer gave
	 * them. Nothing acts on it. It is here so that a test can see that what
	 * is kept for a stream goes when the stream does: a place that forgets
	 * to drop it shows in nothing else, and grows with every request.
	 */
	heldStreams(): number;
	/** The transport session ended. Drops all state; no further output is produced. */
	transportClosed(): void;
}

type PeerUniStream =
	/** The stream type varint has not fully arrived. */
	| { kind: "unknown"; buffer: Uint8Array }
	| { kind: "control" }
	| { kind: "subgroup" }
	/** A fetch stream whose Request ID has not fully arrived (§11.4.4). */
	| { kind: "fetch-header"; buffer: Uint8Array }
	/** The fetch stream of the open FETCH `requestId`, after its FETCH_HEADER. */
	| { kind: "fetch"; requestId: bigint }
	/**
	 * Bytes are dropped: padding (§11.5.1), a fetch stream of a request that
	 * is over, and a stream that was stopped for not saying what it is.
	 */
	| { kind: "discard" };

/**
 * Stream types of Table 3 (§3.4) that are neither SETUP nor SUBGROUP_HEADER.
 * FETCH_HEADER is with its codec, in `fetch.ts`.
 */
const STREAM_TYPE_PADDING = 0x132b3e28n;

const EMPTY = new Uint8Array(0);

export function createSession(options: SessionOptions = {}): Session {
	const limits = resolveLimits(options.limits);
	let started = false;
	let closed = false;
	let nextStream = 0;
	const allocateStream = (): number => {
		const stream = nextStream;
		nextStream += 1;
		return stream;
	};
	/**
	 * Bidirectional streams of the peer whose request this endpoint took
	 * over, by the number the I/O layer gave them, with the number the
	 * session gave them (`adopt-peer-stream`).
	 */
	const adoptedPeerStreams = new Map<number, number>();
	const peerRequests = createPeerRequests({
		subscribe: (message, peerStream, out) =>
			requests.peerSubscribeOpened(message, peerStream, out),
		fetch: (message, peerStream, out) =>
			requests.peerFetchOpened(message, peerStream, out),
		trackStatus: (message, peerStream, out) =>
			requests.peerTrackStatusOpened(message, peerStream, out),
		isOpenSubscribe: (requestId) => requests.isOpenPeerSubscribe(requestId),
		holdsRequestId: (requestId) => requests.holdsPeerRequestId(requestId),
		heldRequests: () => requests.heldPeerRequests(),
		maxPeerRequests: limits.maxPeerRequests,
	});
	const requests: Requests = createRequests({
		limits,
		allocateStream,
		checkPeerRequestId: peerRequests.checkRequestId,
		refusedPeerRequests: () => peerRequests.keptRequests(),
	});
	const controlStream = createPeerControlStream();
	const peerUniStreams = new Map<number, PeerUniStream>();
	let peerControlStream: number | undefined;
	/** The stream this endpoint sent its SETUP on. */
	let controlStreamNumber: number | undefined;

	const identifiedUni = (peerStream: number): SessionOutput => ({
		type: "peer-stream-identified",
		kind: "uni",
		peerStream,
	});

	const classifyPeerUniStream = (
		peerStream: number,
		buffer: Uint8Array,
		out: SessionOutput[],
	): PeerUniStream => {
		let streamType: bigint;
		try {
			streamType = createByteReader(buffer).varint();
		} catch (error) {
			if (!(error instanceof ShortBufferError)) throw error;
			return { kind: "unknown", buffer };
		}

		if (streamType === SETUP_MESSAGE_TYPE) {
			if (peerControlStream !== undefined) {
				throw protocolViolation("the peer opened a second control stream");
			}
			peerControlStream = peerStream;
			out.push(identifiedUni(peerStream));
			return { kind: "control" };
		}
		if (isSubgroupHeaderType(streamType)) {
			requests.incoming.open(peerStream);
			return { kind: "subgroup" };
		}
		if (streamType === STREAM_TYPE_PADDING) {
			out.push(identifiedUni(peerStream));
			return { kind: "discard" };
		}
		if (streamType === FETCH_HEADER_STREAM_TYPE) {
			return { kind: "fetch-header", buffer };
		}
		// §3.4: "An endpoint that receives an unknown stream type MUST close the
		// session." The draft names no code; PROTOCOL_VIOLATION is used.
		throw protocolViolation(`unknown stream type 0x${streamType.toString(16)}`);
	};

	/**
	 * Reads the FETCH_HEADER of a fetch stream once it is whole, and says
	 * what the stream is from then on. `buffer` starts with the Stream Type.
	 * Returns the bytes after the header as well.
	 */
	const openFetchStream = (
		peerStream: number,
		buffer: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): { stream: PeerUniStream; rest: Uint8Array } => {
		const reader = createByteReader(buffer);
		let requestId: bigint;
		try {
			reader.varint();
			requestId = reader.varint();
		} catch (error) {
			if (!(error instanceof ShortBufferError)) throw error;
			// §10.12.3 has the FETCH_HEADER on every fetch stream, also one
			// without Objects, so a stream that ends inside it is not one the
			// draft allows. A reset is another matter (§11.4.1) and does not
			// come here.
			//
			// A stream that sends its type and stops is stopped by the I/O
			// layer when its time to say what it is runs out
			// (`peerStreamTimedOut`).
			if (fin)
				throw protocolViolation("fetch stream ended in its FETCH_HEADER");
			return { stream: { kind: "fetch-header", buffer }, rest: EMPTY };
		}
		const rest = buffer.subarray(reader.offset);
		const opened = requests.fetchStreamOpened(requestId, peerStream, out);
		out.push(identifiedUni(peerStream));
		// A stream that was stopped: what still arrives on it is dropped.
		if (opened === "ended") return { stream: { kind: "discard" }, rest: EMPTY };
		return { stream: { kind: "fetch", requestId }, rest };
	};

	/** The peer stopped a stream of this endpoint that is not the control stream. */
	const localStreamStopped = (
		stream: number,
		code: number | undefined,
		out: SessionOutput[],
	): void => {
		requests.streamStopped(stream, code, out);
		// §11.4.3: "A publisher that receives a STOP_SENDING on a Subgroup
		// stream ... SHOULD respond with a reset."
		requests.outgoing.stopped(stream, out);
	};

	/** Clears everything. Used when the session ends, whoever ended it. */
	const shutdown = (): void => {
		closed = true;
		requests.clear();
		peerRequests.clear();
		adoptedPeerStreams.clear();
		peerUniStreams.clear();
		peerControlStream = undefined;
	};

	/**
	 * Runs an input handler. A `MoqtError` ends the session: the outputs
	 * produced before it are kept and followed by `close-session`.
	 */
	const guard = (handler: (out: SessionOutput[]) => void): SessionOutput[] => {
		if (closed) return [];
		const out: SessionOutput[] = [];
		try {
			handler(out);
		} catch (error) {
			if (!(error instanceof MoqtError)) throw error;
			shutdown();
			out.push({
				type: "close-session",
				code: error.code,
				reason: error.message,
			});
		}
		return out;
	};

	/** Runs a call the application makes; such a call is a mistake before `start()` and after the end. */
	const whileOpen = <T>(action: () => T): T => {
		if (!started) throw new Error("the session has not started");
		if (closed) throw new Error("the session is closed");
		return action();
	};
	/** As `whileOpen`, for a call that opens a request (§10.4). */
	const opening = <T>(action: () => T): T =>
		whileOpen(() => {
			if (controlStream.peerGoingAway) throw new Error("the peer sent GOAWAY");
			return action();
		});

	return {
		get closed() {
			return closed;
		},

		start() {
			if (started) throw new Error("the session has already started");
			started = true;
			const setupOptions: KeyValuePair[] = [];
			// Before the other: the options go in the order of their types.
			if (options.authorizationToken !== undefined) {
				setupOptions.push({
					type: SETUP_OPTION_AUTHORIZATION_TOKEN,
					value: options.authorizationToken,
				});
			}
			if (options.implementation !== undefined) {
				setupOptions.push({
					type: SETUP_OPTION_MOQT_IMPLEMENTATION,
					value: new TextEncoder().encode(options.implementation),
				});
			}
			controlStreamNumber = allocateStream();
			return [
				{
					type: "open-uni-stream",
					stream: controlStreamNumber,
					data: encodeControlMessage({ type: "SETUP", setupOptions }),
					sendOrder: CONTROL_STREAM_SEND_ORDER,
				},
			];
		},

		// §3.3: requests may be sent before the peer's SETUP arrives, so none of
		// these waits for it.
		publishNamespace: (request) =>
			opening(() => requests.publishNamespace(request)),
		publish: (request) => opening(() => requests.publish(request)),
		subscribe: (request) => opening(() => requests.subscribe(request)),
		fetch: (request) => opening(() => requests.fetch(request)),
		trackStatus: (request) => opening(() => requests.trackStatus(request)),
		subscribeNamespace: (request) =>
			opening(() => requests.subscribeNamespace(request)),
		acceptSubscribe: (requestId, answer) =>
			whileOpen(() => requests.acceptSubscribe(requestId, answer)),
		acceptTrackStatus: (requestId, answer) =>
			whileOpen(() => requests.acceptTrackStatus(requestId, answer)),
		rejectPeerRequest: (requestId, errorCode, errorReason) =>
			whileOpen(() =>
				requests.rejectPeerRequest(requestId, errorCode, errorReason),
			),
		acceptFetch: (requestId, answer) =>
			whileOpen(() => requests.acceptFetch(requestId, answer)),
		writeFetchObject: (requestId, object) =>
			whileOpen(() => requests.writeFetchObject(requestId, object)),
		writeFetchRangeEnd: (requestId, kind, location) =>
			whileOpen(() => requests.writeFetchRangeEnd(requestId, kind, location)),
		finishFetch: (requestId) =>
			whileOpen(() => requests.finishFetch(requestId)),
		resetFetch: (requestId, code) =>
			whileOpen(() => requests.resetFetch(requestId, code)),
		publishDone: (requestId, statusCode, errorReason) =>
			whileOpen(() => requests.publishDone(requestId, statusCode, errorReason)),
		openSubgroup: (requestId, subgroupOptions) =>
			whileOpen(() => requests.openSubgroup(requestId, subgroupOptions)),
		writeObject: (stream, object) =>
			whileOpen(() => requests.outgoing.write(stream, object)),
		closeSubgroup: (stream) => whileOpen(() => requests.outgoing.close(stream)),
		resetSubgroup: (stream, code) =>
			whileOpen(() => requests.outgoing.reset(stream, code)),
		subgroupNotOpened: (stream) => {
			const out: SessionOutput[] = [];
			if (!closed) requests.outgoing.notOpened(stream, out);
			return out;
		},

		// Cancelling and expiring race with the end of the session, so after the
		// end they do nothing instead of throwing.
		cancelRequest: (requestId) =>
			closed ? [] : requests.cancelRequest(requestId),
		updateSubscription: (requestId, parameters) =>
			whileOpen(() => requests.updateSubscription(requestId, parameters)),
		expireSubscription: (requestId) =>
			guard((out) => requests.subscriptionExpired(requestId, out)),
		expireUpdateRefusal: (requestId) =>
			guard((out) => requests.updateRefusalExpired(requestId, out)),
		finishServedFetchSend: (requestId) =>
			guard((out) => requests.finishServedFetchSend(requestId, out)),
		expirePeerEnd: (requestId) =>
			guard((out) => {
				requests.peerEndExpired(requestId, out);
				peerRequests.expire(requestId, out);
			}),

		receiveRequestStreamData: (stream, data, fin) =>
			guard((out) => requests.receiveStreamData(stream, data, fin, out)),
		requestStreamReset: (stream, code) =>
			guard((out) => requests.streamReset(stream, code, out)),
		localStreamStopped: (stream, code) =>
			guard((out) => {
				// §3.3: a control stream must stay open for the whole session.
				if (stream === controlStreamNumber) {
					throw protocolViolation(
						"the peer stopped this endpoint's control stream",
					);
				}
				localStreamStopped(stream, code, out);
			}),
		// WHY nothing here names the control stream, where
		// `localStreamStopped` above makes a violation of its stop: this
		// reading of the transport's error is not certain, and it must close
		// no session. It cannot: what follows only looks the stream up among
		// the requests and the subgroup streams, where the control stream is
		// not, so for that stream nothing happens. A test holds that
		// (`a stop without a code on the control stream`), and fails when the
		// control stream is ever given something a stop acts on.
		localStreamAborted: (stream) =>
			guard((out) => localStreamStopped(stream, undefined, out)),

		// WHY Objects and responses are processed before the peer's SETUP: §3.3
		// says data that arrives before the control streams SHOULD be buffered
		// until setup is complete. Setup changes nothing for this endpoint: it
		// negotiates no extension and ignores or rejects every Setup Option, so
		// waiting would only add a buffer to bound.

		receivePeerUniStreamData(peerStream, data, fin) {
			return guard((out) => {
				let stream = peerUniStreams.get(peerStream) ?? {
					kind: "unknown",
					buffer: new Uint8Array(0),
				};
				let input = data;
				if (stream.kind === "unknown") {
					input = concatBytes([stream.buffer, data]);
					stream = classifyPeerUniStream(peerStream, input, out);
				} else if (stream.kind === "fetch-header") {
					input = concatBytes([stream.buffer, data]);
				}
				if (stream.kind === "fetch-header") {
					({ stream, rest: input } = openFetchStream(
						peerStream,
						input,
						fin,
						out,
					));
				}
				if (fin) peerUniStreams.delete(peerStream);
				else peerUniStreams.set(peerStream, stream);

				switch (stream.kind) {
					case "unknown":
						// The draft has no rule for a stream that ends inside its
						// type. It is read like a stream that ends inside an Object
						// (§11.4), as the Rust implementation does, and like a
						// request stream that ends inside its first message. A stream
						// that ends without a byte carried nothing.
						if (fin && input.length > 0) {
							throw protocolViolation("stream ended inside its type");
						}
						break;
					case "fetch-header":
					case "discard":
						break;
					case "fetch":
						requests.fetchStreamData(
							stream.requestId,
							peerStream,
							input,
							fin,
							out,
						);
						break;
					case "control":
						controlStream.receive(input, fin, out);
						break;
					case "subgroup":
						requests.incoming.receive(peerStream, input, fin, out);
						break;
				}
			});
		},

		peerUniStreamReset(peerStream, code) {
			return guard((out) => {
				// §3.3: a control stream must stay open for the whole session.
				if (peerStream === peerControlStream) {
					throw protocolViolation("the peer reset its control stream");
				}
				const stream = peerUniStreams.get(peerStream);
				peerUniStreams.delete(peerStream);
				// A fetch stream that is reset inside its FETCH_HEADER carried
				// nothing, like a stream reset before its type (§11.4.1).
				if (stream?.kind === "fetch") {
					requests.fetchStreamReset(stream.requestId, peerStream, code, out);
					return;
				}
				requests.incoming.reset(peerStream, out);
			});
		},

		receiveDatagram: (datagram) =>
			guard(() => {
				const datagramKind = classifyDatagram(datagram);
				if (datagramKind.kind === "fault") {
					throw protocolViolation(datagramKind.reason);
				}
			}),

		receivePeerBidiStreamData: (peerStream, data, fin) =>
			guard((out) => {
				let stream = adoptedPeerStreams.get(peerStream);
				let rest = data;
				if (stream === undefined) {
					const adopted = peerRequests.receive(peerStream, data, fin, out);
					if (adopted === undefined) return;
					({ stream, rest } = adopted);
					adoptedPeerStreams.set(peerStream, stream);
				}
				if (fin) adoptedPeerStreams.delete(peerStream);
				requests.receiveStreamData(stream, rest, fin, out);
			}),
		peerBidiStreamReset: (peerStream, code) =>
			guard((out) => {
				const stream = adoptedPeerStreams.get(peerStream);
				if (stream === undefined) {
					peerRequests.reset(peerStream, out);
					return;
				}
				adoptedPeerStreams.delete(peerStream);
				requests.streamReset(stream, code, out);
			}),

		peerStreamOpened(kind, peerStream) {
			if (closed) return;
			if (kind === "bidi") {
				peerRequests.open(peerStream);
			} else if (!peerUniStreams.has(peerStream)) {
				peerUniStreams.set(peerStream, {
					kind: "unknown",
					buffer: new Uint8Array(0),
				});
			}
		},

		peerStreamTimedOut(kind, peerStream) {
			return guard((out) => {
				if (kind === "bidi") {
					if (!peerRequests.giveUpUnidentified(peerStream)) return;
					// A stream that never sent its request is cancelled (§3.3.2).
					out.push({
						type: "abandon-peer-request",
						peerStream,
						code: StreamResetErrorCode.CANCELLED,
					});
				} else {
					const stream = peerUniStreams.get(peerStream);
					// The stream has not said what it is while its type, or the
					// header its type comes with, is not whole. For a subgroup
					// stream that is asked where its header is read, and what was
					// held of it is dropped with the answer, as for one reset
					// before its header (§11.4.1, §10.11).
					const silent =
						stream?.kind === "unknown" ||
						stream?.kind === "fetch-header" ||
						(stream?.kind === "subgroup" &&
							requests.incoming.dropBeforeHeader(peerStream));
					if (!silent) return;
					// WHY the stream is kept, as one whose bytes are dropped: what
					// the peer sent before it saw the stop can still arrive, and is
					// no fault of the peer. Forgotten here, those bytes would be
					// read as a stream from its type on, and close the session. It
					// goes when the stream ends, like every other.
					peerUniStreams.set(peerStream, { kind: "discard" });
					// No code of §3.3.3 is for this; the Rust implementation sends
					// the same.
					out.push({
						type: "stop-sending",
						peerStream,
						code: StreamResetErrorCode.INTERNAL_ERROR,
					});
				}
				out.push({
					type: "event",
					event: { type: "peer-stream-timed-out", kind, peerStream },
				});
			});
		},

		peerUniStreamIdle: (peerStream) =>
			guard((out) => {
				if (peerUniStreams.get(peerStream)?.kind !== "subgroup") return;
				requests.incoming.idle(peerStream, out);
			}),

		heldStreams: () =>
			requests.heldStreams() +
			peerRequests.heldStreams() +
			adoptedPeerStreams.size +
			peerUniStreams.size,

		transportClosed: shutdown,
	};
}
