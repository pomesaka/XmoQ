import { concatBytes, createByteReader, ShortBufferError } from "./bytes.ts";
import { encodeControlMessage, SETUP_MESSAGE_TYPE } from "./control-message.ts";
import {
	createPeerControlStream,
	SETUP_OPTION_MOQT_IMPLEMENTATION,
} from "./control-stream.ts";
import { MoqtError, NotImplementedError, protocolViolation } from "./error.ts";
import type { KeyValuePair } from "./key-value-pair.ts";
import { resolveLimits, type SessionLimits } from "./limits.ts";
import type { OutgoingObject, SubgroupOptions } from "./outgoing-subgroups.ts";
import { createPeerRequests } from "./peer-requests.ts";
import {
	createRequests,
	type PublishNamespaceRequest,
	type PublishRequest,
	type SubscribeRequest,
} from "./requests.ts";
import type { SessionOutput, SessionResult } from "./session-output.ts";
import { isSubgroupHeaderType } from "./subgroup.ts";

export interface SessionOptions {
	/** Value of the MOQT_IMPLEMENTATION Setup Option (§10.3.1.5). Not sent when undefined. */
	implementation?: string;
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
 * Not implemented: the server role, requests opened by the peer (they are
 * answered with REQUEST_ERROR NOT_SUPPORTED), REQUEST_UPDATE, GOAWAY, FETCH,
 * datagrams, namespace subscription and timers.
 */
export interface Session {
	readonly closed: boolean;
	/** Opens the control stream with SETUP (§3.3). */
	start(): SessionOutput[];
	/** Each request method returns the Request ID it used. */
	publishNamespace(request: PublishNamespaceRequest): SessionResult<bigint>;
	publish(request: PublishRequest): SessionResult<bigint>;
	subscribe(request: SubscribeRequest): SessionResult<bigint>;
	/** Cancels a request this endpoint made (§3.3.2). */
	cancelRequest(requestId: bigint): SessionOutput[];
	/**
	 * Ends a subscription made with PUBLISH: sends PUBLISH_DONE and a FIN
	 * (§10.11). Every subgroup stream of the subscription has to be closed first.
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
	/**
	 * The wait for the subscriber to end its direction of the request stream
	 * after `publishDone` is over. The timer belongs to the caller. Stops
	 * that direction (CANCELLED); does nothing when it has ended.
	 */
	expirePublishDonePeerEnd(requestId: bigint): SessionOutput[];
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
	/**
	 * The peer sent STOP_SENDING for a stream this endpoint opened. `code` is
	 * its error code when the transport reports one.
	 */
	localStreamStopped(stream: number, code?: number): SessionOutput[];
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
	/** The peer reset a unidirectional stream it opened. */
	peerUniStreamReset(peerStream: number): SessionOutput[];
	/** Bytes on a bidirectional stream the peer opened. */
	receivePeerBidiStreamData(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
	): SessionOutput[];
	/** The peer reset a bidirectional stream it opened. */
	peerBidiStreamReset(peerStream: number): SessionOutput[];
	/** The transport session ended. Drops all state; no further output is produced. */
	transportClosed(): void;
}

type PeerUniStream =
	/** The stream type varint has not fully arrived. */
	| { kind: "unknown"; buffer: Uint8Array }
	| { kind: "control" }
	| { kind: "subgroup" }
	/** Padding: bytes are dropped (§11.5.1). */
	| { kind: "discard" };

/** Stream types of Table 3 (§3.4) that are neither SETUP nor SUBGROUP_HEADER. */
const STREAM_TYPE_FETCH_HEADER = 0x05n;
const STREAM_TYPE_PADDING = 0x132b3e28n;

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
	const peerRequests = createPeerRequests();
	const requests = createRequests({
		limits,
		allocateStream,
		checkPeerRequestId: peerRequests.checkRequestId,
	});
	const controlStream = createPeerControlStream();
	const peerUniStreams = new Map<number, PeerUniStream>();
	let peerControlStream: number | undefined;
	/** The stream this endpoint sent its SETUP on. */
	let controlStreamNumber: number | undefined;

	const classifyPeerUniStream = (
		peerStream: number,
		buffer: Uint8Array,
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
			return { kind: "control" };
		}
		if (isSubgroupHeaderType(streamType)) {
			requests.incoming.open(peerStream);
			return { kind: "subgroup" };
		}
		if (streamType === STREAM_TYPE_PADDING) return { kind: "discard" };
		if (streamType === STREAM_TYPE_FETCH_HEADER) {
			// Table 3 lists it, so the peer is within the draft. The draft has no
			// rule for a fetch stream without a FETCH, which is the only case here.
			throw new NotImplementedError("FETCH_HEADER stream");
		}
		// §3.4: "An endpoint that receives an unknown stream type MUST close the
		// session." The draft names no code; PROTOCOL_VIOLATION is used.
		throw protocolViolation(`unknown stream type 0x${streamType.toString(16)}`);
	};

	/** Clears everything. Used when the session ends, whoever ended it. */
	const shutdown = (): void => {
		closed = true;
		requests.clear();
		peerRequests.clear();
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

	return {
		get closed() {
			return closed;
		},

		start() {
			if (started) throw new Error("the session has already started");
			started = true;
			const setupOptions: KeyValuePair[] = [];
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
				},
			];
		},

		// §3.3: requests may be sent before the peer's SETUP arrives, so none of
		// these waits for it.
		publishNamespace: (request) =>
			whileOpen(() => requests.publishNamespace(request)),
		publish: (request) => whileOpen(() => requests.publish(request)),
		subscribe: (request) => whileOpen(() => requests.subscribe(request)),
		publishDone: (requestId, statusCode, errorReason) =>
			whileOpen(() => requests.publishDone(requestId, statusCode, errorReason)),
		openSubgroup: (requestId, subgroupOptions) =>
			whileOpen(() => requests.openSubgroup(requestId, subgroupOptions)),
		writeObject: (stream, object) =>
			whileOpen(() => requests.outgoing.write(stream, object)),
		closeSubgroup: (stream) => whileOpen(() => requests.outgoing.close(stream)),
		resetSubgroup: (stream, code) =>
			whileOpen(() => requests.outgoing.reset(stream, code)),

		// Cancelling and expiring race with the end of the session, so after the
		// end they do nothing instead of throwing.
		cancelRequest: (requestId) =>
			closed ? [] : requests.cancelRequest(requestId),
		expireSubscription: (requestId) =>
			guard((out) => requests.subscriptionExpired(requestId, out)),
		expirePublishDonePeerEnd: (requestId) =>
			guard((out) => requests.publishDonePeerEndExpired(requestId, out)),

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
				requests.streamStopped(stream, code, out);
				// §11.4.3: "A publisher that receives a STOP_SENDING on a Subgroup
				// stream ... SHOULD respond with a reset."
				requests.outgoing.stopped(stream, out);
			}),

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
					stream = classifyPeerUniStream(peerStream, input);
				}
				if (fin) peerUniStreams.delete(peerStream);
				else peerUniStreams.set(peerStream, stream);

				switch (stream.kind) {
					case "unknown":
					// The draft has no rule for a stream that ends inside its type;
					// it carried nothing, so it is dropped.
					case "discard":
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

		peerUniStreamReset(peerStream) {
			return guard((out) => {
				// §3.3: a control stream must stay open for the whole session.
				if (peerStream === peerControlStream) {
					throw protocolViolation("the peer reset its control stream");
				}
				peerUniStreams.delete(peerStream);
				requests.incoming.reset(peerStream, out);
			});
		},

		receivePeerBidiStreamData: (peerStream, data, fin) =>
			guard((out) => peerRequests.receive(peerStream, data, fin, out)),
		peerBidiStreamReset: (peerStream) =>
			guard(() => peerRequests.reset(peerStream)),

		transportClosed: shutdown,
	};
}
