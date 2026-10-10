import {
	type ControlMessage,
	drainControlMessages,
	encodeControlMessage,
	type FetchOk,
	type FetchRange,
	type PublishDone,
	type RequestError,
	type RequestOk,
	type RequestUpdate,
	SUBSCRIBE_MESSAGE_TYPE,
	type Subscribe,
	type SubscribeOk,
} from "./control-message.ts";
import {
	InactiveError,
	MoqtError,
	protocolViolation,
	RequestErrorCode,
	SessionErrorCode,
	StreamResetErrorCode,
} from "./error.ts";
import {
	createIncomingSubgroups,
	type IncomingSubgroups,
} from "./incoming-subgroups.ts";
import {
	isMandatoryTrackProperty,
	type KeyValuePair,
} from "./key-value-pair.ts";
import type { SessionLimits } from "./limits.ts";
import {
	createOutgoingSubgroups,
	type OutgoingSubgroups,
	type SubgroupOptions,
} from "./outgoing-subgroups.ts";
import {
	endsBefore,
	type Location,
	type MessageParameters,
	parametersOutsideScope,
} from "./parameters.ts";
import type {
	RequestClosedReason,
	SessionOutput,
	SessionResult,
} from "./session-output.ts";
import {
	type FullTrackName,
	sameFullTrackName,
	type TrackNamespace,
} from "./track.ts";

export interface PublishNamespaceRequest {
	trackNamespace: TrackNamespace;
	parameters?: MessageParameters;
}

export interface PublishRequest extends FullTrackName {
	trackAlias: bigint;
	parameters?: MessageParameters;
	trackProperties?: KeyValuePair[];
}

export interface SubscribeRequest extends FullTrackName {
	parameters?: MessageParameters;
}

export interface FetchRequest {
	range: FetchRange;
	parameters?: MessageParameters;
}

/** What a fetch stream that arrived belongs to. */
export type FetchStreamOpened =
	/** A FETCH that is open: the bytes of the stream are its response. */
	| "fetch"
	/**
	 * A request of this endpoint that is over. For a FETCH that was cancelled
	 * or rejected, the publisher opened the stream before it knew (§5.2:
	 * "some Objects could be received even if the FETCH results in error").
	 * The stream was stopped; nothing is wrong.
	 */
	| "ended";

/** What the application answers a SUBSCRIBE of the peer with (§10.8). */
export interface SubscribeAnswer {
	/**
	 * Parameters of the SUBSCRIBE_OK. LARGEST_OBJECT is where a publisher
	 * says how far it is, and §10.2.11 has a MUST for it: "If Objects have
	 * been published on this Track the Publisher MUST include this
	 * parameter." The session knows nothing of earlier subscriptions to the
	 * Track, so its caller has to keep that.
	 */
	parameters?: MessageParameters;
	trackProperties?: KeyValuePair[];
}

/** What became of a SUBSCRIBE of the peer: it is refused with this code, or it waits for the application on this stream. */
export type PeerSubscribeOpened = { refused: bigint } | { stream: number };

export interface RequestsOptions {
	limits: SessionLimits;
	/** Hands out the numbers of the streams this endpoint opens, which requests share with the control stream. */
	allocateStream(): number;
	/** §10.1: checks a Request ID the peer used, and throws the `MoqtError` when it is wrong. */
	checkPeerRequestId(requestId: bigint): void;
}

/**
 * The requests this endpoint makes and the subscriptions that come of them.
 *
 * Methods that take `out` report something that happened (bytes from the
 * peer, a reset, an expired timer) and throw the `MoqtError` that ends the
 * session. The others are called for the application and throw when they are
 * misused.
 */
export interface Requests {
	/** Subgroup streams the peer opens are handed to this. */
	readonly incoming: IncomingSubgroups;
	/** Subgroup streams this endpoint writes. */
	readonly outgoing: OutgoingSubgroups;
	publishNamespace(request: PublishNamespaceRequest): SessionResult<bigint>;
	publish(request: PublishRequest): SessionResult<bigint>;
	subscribe(request: SubscribeRequest): SessionResult<bigint>;
	/**
	 * Asks for a range of a track (§10.12).
	 *
	 * The request is over when it was rejected, when it was cancelled, or
	 * when both its FETCH_OK came and its fetch stream ended. WHY both, where
	 * §5.2 has the state kept "until ... the FETCH data stream receives a FIN
	 * or is reset": the response "can come at any time relative to object
	 * delivery" (§10.12.3), and a FETCH_OK that comes after the stream ended
	 * would have no request to go to. A publisher that never answers leaves
	 * the request open: there is no clock here, so the caller cancels it when
	 * it has waited long enough.
	 *
	 * Throws a `RangeError` for a parameter that may not appear in FETCH, for
	 * a Standalone FETCH that ends before it starts (§10.12.3), and for a
	 * Joining FETCH that names a request that is not a SUBSCRIBE of this
	 * endpoint that is Pending or Established (§10.12.2).
	 */
	fetch(request: FetchRequest): SessionResult<bigint>;
	/**
	 * What the Request ID of a FETCH_HEADER refers to (§11.4.4), on the
	 * stream `peerStream`. The stream may come before the FETCH_OK (§10.13).
	 *
	 * WHY any request of this endpoint that is over counts as a FETCH that
	 * ended, and no list of the requests that ended is kept: a list has an
	 * end, and what falls off it reads as a FETCH that was never sent, which
	 * closes the session. The Rust implementation has the same, and why a
	 * relay needs it. The price: a second stream for a FETCH that is over,
	 * and a fetch stream for a SUBSCRIBE that is over, are stopped and not
	 * found out.
	 *
	 * Throws a PROTOCOL_VIOLATION for an ID this endpoint never gave a
	 * request, for the ID of an open request that is no FETCH, and for a
	 * second stream of an open FETCH (§10.12.3: "The Objects in the response
	 * are delivered on a single unidirectional stream"). The draft names no
	 * code for any of them.
	 */
	fetchStreamOpened(
		requestId: bigint,
		peerStream: number,
		out: SessionOutput[],
	): FetchStreamOpened;
	/**
	 * Bytes of the fetch stream `peerStream` after its FETCH_HEADER, and its
	 * end with `fin`. Does nothing for a stream that is not the one of an
	 * open FETCH: what the peer sent before it saw the stream stopped.
	 */
	fetchStreamData(
		requestId: bigint,
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): void;
	/** The peer reset the fetch stream `peerStream`. */
	fetchStreamReset(
		requestId: bigint,
		peerStream: number,
		code: number | undefined,
		out: SessionOutput[],
	): void;
	cancelRequest(requestId: bigint): SessionOutput[];
	/**
	 * A SUBSCRIBE of the peer, whole, with a Request ID that was checked.
	 * Unless it is refused here, the stream it came on is adopted
	 * (`adopt-peer-stream`), and the request waits for `acceptSubscribe` or
	 * `rejectPeerRequest`. What arrives on the stream from then on goes to
	 * `receiveStreamData`, `streamReset` and `streamStopped`, with the number
	 * returned.
	 */
	peerSubscribeOpened(
		message: Subscribe,
		peerStream: number,
		out: SessionOutput[],
	): PeerSubscribeOpened;
	/** Whether the Request ID names a SUBSCRIBE of the peer that waits for its answer or was accepted, and is not over (§10.12.2). */
	isOpenPeerSubscribe(requestId: bigint): boolean;
	/**
	 * Accepts a SUBSCRIBE of the peer: sends SUBSCRIBE_OK. Returns the Track
	 * Alias this endpoint chose. From here on the subscription is one this
	 * endpoint is the publisher of, like one made with PUBLISH, under the
	 * peer's Request ID.
	 */
	acceptSubscribe(
		requestId: bigint,
		answer: SubscribeAnswer,
	): SessionResult<bigint>;
	/** Refuses a request of the peer that waits for its answer: REQUEST_ERROR and a FIN (§3.3.2). */
	rejectPeerRequest(
		requestId: bigint,
		errorCode: bigint,
		errorReason: string,
	): SessionOutput[];
	publishDone(
		requestId: bigint,
		statusCode: bigint,
		errorReason: string,
	): SessionOutput[];
	openSubgroup(
		requestId: bigint,
		options: SubgroupOptions,
	): SessionResult<number>;
	/** The caller's wait for late streams after PUBLISH_DONE ran out. */
	subscriptionExpired(requestId: bigint, out: SessionOutput[]): void;
	/**
	 * The wait for the subscriber to end its direction of the request stream
	 * after this endpoint's PUBLISH_DONE is over: that direction is stopped.
	 * Does nothing when the subscriber has ended it.
	 */
	publishDonePeerEndExpired(requestId: bigint, out: SessionOutput[]): void;
	receiveStreamData(
		stream: number,
		data: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): void;
	/** The peer reset its side of a request stream this endpoint opened. */
	streamReset(
		stream: number,
		code: number | undefined,
		out: SessionOutput[],
	): void;
	/** The peer sent STOP_SENDING on a request stream this endpoint opened. */
	streamStopped(
		stream: number,
		code: number | undefined,
		out: SessionOutput[],
	): void;
	/** How many request streams something is kept of: the open requests, and the ones that ended while the peer can still send on their stream. */
	heldStreams(): number;
	clear(): void;
}

/** Subscription states of §5.1; PUBLISH_NAMESPACE requests use the same three. */
type RequestState = "Pending" | "Established" | "Terminated";

interface RequestCommon {
	requestId: bigint;
	stream: number;
	state: RequestState;
	/** Bytes of an incomplete control message from the peer. */
	inbound: Uint8Array;
	/**
	 * Nothing more is sent on the request stream: this endpoint closed or reset
	 * its side, or the peer stopped it.
	 */
	sendClosed: boolean;
	/**
	 * This endpoint ended the request (PUBLISH_DONE). What the peer sent before
	 * it saw that can still arrive and is not a mistake of the peer.
	 */
	endedLocally: boolean;
	/**
	 * The peer finished its direction of the request stream while the request
	 * went on, which only the subscriber of an Established PUBLISH can do
	 * without ending the request.
	 */
	peerFinished: boolean;
}

interface NamespaceEntry extends RequestCommon {
	kind: "PUBLISH_NAMESPACE";
}

/**
 * A subscription this endpoint is the publisher of: one it made with
 * PUBLISH, or one the peer made with a SUBSCRIBE that was accepted. The two
 * differ in who opened the request stream and whose Request ID names them,
 * and in nothing that follows.
 */
interface PublishEntry extends RequestCommon {
	kind: "PUBLISH";
	/** Who opened the request: this endpoint with PUBLISH, or the peer with SUBSCRIBE. */
	openedBy: "self" | "peer";
	fullTrackName: FullTrackName;
	trackAlias: bigint;
	/** The Forward State of §5.1. */
	forward: boolean;
}

/** A SUBSCRIBE of the peer that waits for the application's answer: Pending (Publisher) of §5.1. */
interface PeerSubscribeEntry extends RequestCommon {
	kind: "PEER_SUBSCRIBE";
	fullTrackName: FullTrackName;
	/** FORWARD of the SUBSCRIBE (§10.2.12). */
	forward: boolean;
}

interface SubscribeEntry extends RequestCommon {
	kind: "SUBSCRIBE";
	/** Set by SUBSCRIBE_OK. */
	trackAlias: bigint | undefined;
}

/**
 * A FETCH. Its state is Pending until FETCH_OK and Established after; the
 * draft names no states for it (§5.2).
 */
interface FetchEntry extends RequestCommon {
	kind: "FETCH";
	/**
	 * The Start Location, where this endpoint knows it: not for a Relative
	 * Joining FETCH, whose start the publisher counts from the subscription.
	 */
	start: Location | undefined;
	/**
	 * What this endpoint saw of the fetch stream: nothing, the stream with
	 * this number, or its end.
	 */
	fetchStream: "none" | "ended" | { peerStream: number };
}

type RequestEntry =
	| NamespaceEntry
	| PublishEntry
	| PeerSubscribeEntry
	| SubscribeEntry
	| FetchEntry;

/** §10.2.12: FORWARD defaults to 1 wherever it is omitted, except in REQUEST_UPDATE. */
const DEFAULT_FORWARD = true;

/** PUBLISH_DONE status code UPDATE_FAILED (§10.11). */
const UPDATE_FAILED = 0x8n;

export function createRequests(options: RequestsOptions): Requests {
	// §10.1: the client generates even Request IDs starting at 0.
	let nextRequestId = 0n;
	/** Requests that are not Terminated, by Request ID. */
	const requests = new Map<bigint, RequestEntry>();
	/**
	 * Requests by stream, kept after the request ended for as long as the peer
	 * can still send on the stream, so that a message after the end is seen.
	 */
	const requestStreams = new Map<number, RequestEntry>();
	/**
	 * Track Aliases of this endpoint's active PUBLISH requests. §11.1 forbids
	 * one alias for two Tracks "simultaneously", so an alias is free again when
	 * its subscription ends.
	 */
	const publishedAliases = new Set<bigint>();
	/**
	 * Where the search for a free Track Alias for an accepted SUBSCRIBE of the
	 * peer starts: counted up from 0, past the ones in use, as in the Rust
	 * session (`next_free_alias`).
	 */
	let nextServedAlias = 0n;
	const outgoing = createOutgoingSubgroups(options.allocateStream);
	// The three callbacks are function declarations further down, so that this
	// table of dependencies can stand before the code that uses it.
	const incoming = createIncomingSubgroups({
		limits: options.limits,
		hasPendingSubscribe,
		onMalformedTrack,
		onDrained: endAfterPublishDone,
	});

	function hasPendingSubscribe(): boolean {
		for (const request of requests.values()) {
			if (request.kind === "SUBSCRIBE" && request.state === "Pending") {
				return true;
			}
		}
		return false;
	}

	/**
	 * §5.1 allows one subscription to a Track in each role: whether this
	 * endpoint is the publisher of one, through its own PUBLISH or a
	 * SUBSCRIBE of the peer, answered or not.
	 */
	const isPublishing = (name: FullTrackName): boolean => {
		for (const request of requests.values()) {
			if (
				(request.kind === "PUBLISH" || request.kind === "PEER_SUBSCRIBE") &&
				sameFullTrackName(request.fullTrackName, name)
			) {
				return true;
			}
		}
		return false;
	};

	/** Requests of the peer that are open: waiting for their answer, or accepted. */
	const openPeerRequests = (): number => {
		let count = 0;
		for (const request of requests.values()) {
			if (
				request.kind === "PEER_SUBSCRIBE" ||
				(request.kind === "PUBLISH" && request.openedBy === "peer")
			) {
				count += 1;
			}
		}
		return count;
	};

	const closedEvent = (
		request: RequestEntry,
		reason: RequestClosedReason,
		resetCode?: number,
	): SessionOutput => ({
		type: "event",
		event: {
			type: "request-closed",
			requestId: request.requestId,
			reason,
			resetCode,
		},
	});

	/**
	 * §10.11: the subscription received PUBLISH_DONE and lives on only for the
	 * Objects that are still on their way.
	 */
	const isDraining = (request: RequestEntry): boolean =>
		request.kind === "SUBSCRIBE" &&
		request.trackAlias !== undefined &&
		incoming.aliasState(request.trackAlias) === "draining";

	const finishSend = (request: RequestEntry, out: SessionOutput[]): void => {
		if (request.sendClosed) return;
		request.sendClosed = true;
		out.push({ type: "finish", stream: request.stream });
	};

	/**
	 * This endpoint sent PUBLISH_DONE for its PUBLISH, the application's or
	 * the UPDATE_FAILED of a refused REQUEST_UPDATE. How the subscriber then
	 * ends its direction of the request stream is reported, and the caller's
	 * wait for it ends with `publishDonePeerEndExpired`.
	 */
	const awaitsPeerEnd = (request: RequestEntry): boolean =>
		request.kind === "PUBLISH" && request.endedLocally;

	const reportPeerEndAfterPublishDone = (
		request: RequestEntry,
		how: "fin" | "reset",
		resetCode: number | undefined,
		out: SessionOutput[],
	): void => {
		if (!awaitsPeerEnd(request)) return;
		out.push({
			type: "event",
			event: {
				type: "publish-done-peer-ended",
				requestId: request.requestId,
				how,
				resetCode,
			},
		});
	};

	/** Abandons the request stream in both directions (§3.3.2). The peer's later bytes on it are not read. */
	const resetRequestStream = (
		request: RequestEntry,
		code: number,
		out: SessionOutput[],
	): void => {
		request.sendClosed = true;
		requestStreams.delete(request.stream);
		out.push({ type: "reset", stream: request.stream, code });
	};

	/**
	 * The one way a request ends. Moves it to Terminated, releases what it
	 * holds, then lets `announce` add the outputs that tell the peer and the
	 * application.
	 *
	 * WHY held Objects are resolved here: a SUBSCRIBE that leaves Pending
	 * decides whether the subgroup streams held for an unknown Track Alias can
	 * still find a subscription. Doing it in the one place every ending passes
	 * through means a new way to end a request cannot forget it.
	 *
	 * `code` is the Stream Reset Error Code (§3.3.3) for the data streams of
	 * the request that are stopped here.
	 */
	const endRequest = (
		request: RequestEntry,
		out: SessionOutput[],
		announce: () => void,
		code: number = StreamResetErrorCode.CANCELLED,
	): void => {
		const wasPendingSubscribe =
			request.kind === "SUBSCRIBE" && request.state === "Pending";
		request.state = "Terminated";
		request.inbound = new Uint8Array(0);
		requests.delete(request.requestId);
		if (request.kind === "PUBLISH") {
			publishedAliases.delete(request.trackAlias);
			// §5.1.1 and §11.4.3: open streams of a subscription that stops are reset.
			outgoing.resetAll(request.requestId, out);
			outgoing.forget(request.requestId);
		}
		if (request.kind === "SUBSCRIBE" && request.trackAlias !== undefined) {
			incoming.release(request.trackAlias, code, out);
		}
		// §5.2: "If the data stream is already open, the subscriber wishing to
		// cancel the FETCH MAY send STOP_SENDING for the data stream as well".
		// It is always done: nobody reads the stream from here on. A stream
		// that comes later finds a request that is over and is stopped then.
		if (request.kind === "FETCH" && typeof request.fetchStream === "object") {
			const { peerStream } = request.fetchStream;
			request.fetchStream = "ended";
			out.push({ type: "stop-sending", peerStream, code });
		}
		announce();
		if (wasPendingSubscribe) incoming.resolveHeld(out);
	};

	/** Ends a request this endpoint gives up on, and tells the application why. */
	const abandonRequest = (
		request: RequestEntry,
		reason: RequestClosedReason,
		out: SessionOutput[],
		peerResetCode?: number,
	): void => {
		// §3.3.3: the code says why this endpoint lets go of the streams.
		const code =
			reason === "malformed-track"
				? StreamResetErrorCode.MALFORMED_TRACK
				: StreamResetErrorCode.CANCELLED;
		endRequest(
			request,
			out,
			() => {
				out.push(closedEvent(request, reason, peerResetCode));
				resetRequestStream(request, code, out);
			},
			code,
		);
	};

	function onMalformedTrack(requestId: bigint, out: SessionOutput[]): void {
		const request = requests.get(requestId);
		// §2.4.2: the subscriber MUST cancel the subscription.
		if (request !== undefined) abandonRequest(request, "malformed-track", out);
	}

	/** §10.11: the subscription that received PUBLISH_DONE has nothing more to wait for. */
	function endAfterPublishDone(requestId: bigint, out: SessionOutput[]): void {
		const request = requests.get(requestId);
		if (request?.kind !== "SUBSCRIBE" || request.trackAlias === undefined) {
			return;
		}
		if (incoming.aliasState(request.trackAlias) !== "draining") return;
		endRequest(request, out, () => {
			out.push(closedEvent(request, "publish-done"));
			finishSend(request, out);
		});
	}

	const onRequestError = (
		request: RequestEntry,
		message: RequestError,
		out: SessionOutput[],
	): void => {
		// §10.6.1: Track Name is not meaningful for a namespace-scoped request.
		// (The other MUST of §10.6.1, a non-zero Connect URI Length, binds a
		// server that receives a Redirect, which this endpoint never is.)
		if (
			request.kind === "PUBLISH_NAMESPACE" &&
			message.redirect !== undefined &&
			message.redirect.trackName.length > 0
		) {
			throw protocolViolation(
				"Redirect for PUBLISH_NAMESPACE has a non-empty Track Name",
			);
		}
		endRequest(request, out, () => {
			out.push({
				type: "event",
				event: {
					type: "request-error",
					requestId: request.requestId,
					errorCode: message.errorCode,
					retryInterval: message.retryInterval,
					errorReason: message.errorReason,
					redirect: message.redirect,
				},
			});
			// Nothing more is sent on a request that failed.
			finishSend(request, out);
		});
	};

	const onRequestOk = (
		request: NamespaceEntry | PublishEntry,
		message: RequestOk,
		out: SessionOutput[],
	): void => {
		const scope =
			request.kind === "PUBLISH" ? "PUBLISH_OK" : "PUBLISH_NAMESPACE_OK";
		// The codec accepted what any REQUEST_OK may carry; §10.2.1 is checked
		// here, where the request is known.
		const outside = parametersOutsideScope(message.parameters, scope);
		if (outside.length > 0) {
			throw protocolViolation(`${outside.join(", ")} not allowed in ${scope}`);
		}
		// §10.5: Track Properties are empty in PUBLISH_OK and PUBLISH_NAMESPACE_OK.
		if (message.trackProperties.length > 0) {
			throw protocolViolation(`Track Properties in ${scope}`);
		}
		request.state = "Established";
		if (request.kind === "PUBLISH_NAMESPACE") {
			out.push({
				type: "event",
				event: { type: "publish-namespace-ok", requestId: request.requestId },
			});
			return;
		}
		request.forward = message.parameters.forward ?? DEFAULT_FORWARD;
		// §11.4.3: a stream whose Objects are left out because of the Forward
		// State is reset, not left open.
		if (!request.forward) outgoing.resetAll(request.requestId, out);
		out.push({
			type: "event",
			event: {
				type: "publish-ok",
				requestId: request.requestId,
				parameters: message.parameters,
			},
		});
	};

	const onSubscribeOk = (
		request: SubscribeEntry,
		message: SubscribeOk,
		out: SessionOutput[],
	): void => {
		const aliasState = incoming.aliasState(message.trackAlias);
		// §11.1. Checked before the Track Properties: an error that ends the
		// session takes precedence over one that ends a subscription.
		if (aliasState === "established") {
			throw new MoqtError(
				SessionErrorCode.DUPLICATE_TRACK_ALIAS,
				`Track Alias ${message.trackAlias} is in use by another subscription`,
			);
		}
		// No Mandatory Track Property is implemented, so each one is unknown
		// (§2.5.1: "MUST NOT process ... that track"). A SUBSCRIBE_OK that is not
		// processed does not take the alias: a draining subscription keeps it,
		// and a free alias is only remembered, so that streams the publisher
		// opens for it are dropped at once (§11.1).
		if (
			message.trackProperties.some(({ type }) => isMandatoryTrackProperty(type))
		) {
			if (aliasState === "free") incoming.retire(message.trackAlias);
			abandonRequest(request, "unsupported-track-property", out);
			return;
		}
		incoming.establish(message.trackAlias, request.requestId, out);
		request.trackAlias = message.trackAlias;
		request.state = "Established";
		out.push({
			type: "event",
			event: {
				type: "subscribe-ok",
				requestId: request.requestId,
				trackAlias: message.trackAlias,
				parameters: message.parameters,
				trackProperties: message.trackProperties,
			},
		});
		// The SUBSCRIBE left Pending by being accepted.
		incoming.resolveHeld(out);
	};

	/** A FETCH whose FETCH_OK came and whose fetch stream ended has nothing more to come. */
	const endFetchIfServed = (
		request: FetchEntry,
		out: SessionOutput[],
	): void => {
		if (request.state !== "Established" || request.fetchStream !== "ended") {
			return;
		}
		endRequest(request, out, () => {
			out.push(closedEvent(request, "fetch-served"));
			finishSend(request, out);
		});
	};

	const onFetchOk = (
		request: FetchEntry,
		message: FetchOk,
		out: SessionOutput[],
	): void => {
		// §10.13: "If End Location is smaller than the Start Location in the
		// corresponding FETCH the receiver MUST close the session with a
		// PROTOCOL_VIOLATION."
		if (
			request.start !== undefined &&
			endsBefore(request.start, message.endLocation)
		) {
			throw protocolViolation("FETCH_OK ends before the FETCH starts");
		}
		// §2.5.1, as for SUBSCRIBE_OK: no Mandatory Track Property is
		// understood, so the track is not processed.
		if (
			message.trackProperties.some(({ type }) => isMandatoryTrackProperty(type))
		) {
			abandonRequest(request, "unsupported-track-property", out);
			return;
		}
		request.state = "Established";
		out.push({
			type: "event",
			event: {
				type: "fetch-ok",
				requestId: request.requestId,
				endOfTrack: message.endOfTrack,
				endLocation: message.endLocation,
				parameters: message.parameters,
				trackProperties: message.trackProperties,
			},
		});
		endFetchIfServed(request, out);
	};

	/** The FETCH whose fetch stream is `peerStream`, while both are open. */
	const fetchOfStream = (
		requestId: bigint,
		peerStream: number,
	): FetchEntry | undefined => {
		const request = requests.get(requestId);
		if (request?.kind !== "FETCH") return undefined;
		if (typeof request.fetchStream !== "object") return undefined;
		return request.fetchStream.peerStream === peerStream ? request : undefined;
	};

	const fetchStreamEnded = (
		request: FetchEntry,
		how: "fin" | "reset",
		resetCode: number | undefined,
		out: SessionOutput[],
	): void => {
		request.fetchStream = "ended";
		out.push({
			type: "event",
			event: {
				type: "fetch-stream-ended",
				requestId: request.requestId,
				how,
				resetCode,
			},
		});
		endFetchIfServed(request, out);
	};

	const onPublishDone = (
		request: SubscribeEntry,
		trackAlias: bigint,
		message: PublishDone,
		out: SessionOutput[],
	): void => {
		// WHY the subscription does not end here: PUBLISH_DONE travels on the
		// request stream and usually overtakes Objects still on their way
		// (§10.11). It ends when Stream Count streams have ended, when the caller
		// reports that its wait ran out, or when the alias is given away.
		out.push({
			type: "event",
			event: {
				type: "publish-done",
				requestId: request.requestId,
				statusCode: message.statusCode,
				streamCount: message.streamCount,
				errorReason: message.errorReason,
			},
		});
		incoming.expectStreams(trackAlias, message.streamCount, out);
	};

	/**
	 * §10.9: the subscriber of a subscription made with PUBLISH may update it.
	 * Updates are not implemented, so the update fails: §4 asks a limited
	 * endpoint for NOT_SUPPORTED, and §10.9.1 says "When a REQUEST_UPDATE is
	 * unsuccessful, the publisher MUST also terminate the subscription by
	 * sending a PUBLISH_DONE with error code UPDATE_FAILED."
	 */
	const onRequestUpdate = (
		request: PublishEntry,
		message: RequestUpdate,
		out: SessionOutput[],
	): void => {
		// §10.1: REQUEST_UPDATE consumes a Request ID of its sender.
		options.checkPeerRequestId(message.requestId);
		const { stream, requestId } = request;
		out.push({
			type: "send",
			stream,
			data: encodeControlMessage({
				type: "REQUEST_ERROR",
				errorCode: RequestErrorCode.NOT_SUPPORTED,
				retryInterval: 0n,
				errorReason: "REQUEST_UPDATE is not implemented",
			}),
		});
		// §10.11: PUBLISH_DONE is not sent until every stream of the subscription
		// is closed, so the count is taken before `endRequest` resets them and
		// the message is written after.
		const streamCount = outgoing.streamCount(requestId);
		request.endedLocally = true;
		endRequest(request, out, () => {
			out.push({
				type: "send",
				stream,
				data: encodeControlMessage({
					type: "PUBLISH_DONE",
					statusCode: UPDATE_FAILED,
					streamCount,
					errorReason: "REQUEST_UPDATE is not implemented",
				}),
			});
			out.push(closedEvent(request, "update-failed"));
			finishSend(request, out);
		});
	};

	/**
	 * A REQUEST_UPDATE for a SUBSCRIBE of the peer that has no answer yet. It
	 * is refused, and the request is over with that.
	 *
	 * WHY the request ends: responses carry no Request ID, so the
	 * REQUEST_ERROR for the update can only be read as the response to the
	 * SUBSCRIBE itself. The Rust session does the same (`answer_update`).
	 */
	const onUpdateBeforeAnswer = (
		request: PeerSubscribeEntry,
		message: RequestUpdate,
		out: SessionOutput[],
	): void => {
		// §10.1: REQUEST_UPDATE consumes a Request ID of its sender.
		options.checkPeerRequestId(message.requestId);
		request.endedLocally = true;
		endRequest(request, out, () => {
			out.push({
				type: "send",
				stream: request.stream,
				data: encodeControlMessage({
					type: "REQUEST_ERROR",
					errorCode: RequestErrorCode.NOT_SUPPORTED,
					retryInterval: 0n,
					errorReason: "REQUEST_UPDATE is not implemented",
				}),
			});
			out.push(closedEvent(request, "update-failed"));
			finishSend(request, out);
		});
	};

	const onResponse = (
		request: RequestEntry,
		message: ControlMessage,
		out: SessionOutput[],
	): void => {
		// §5.1 and §6.2: exactly one response per request. §10.11: PUBLISH_DONE
		// is the final message.
		const unexpected = protocolViolation(
			`unexpected ${message.type} on the ${request.kind} request stream`,
		);
		if (isDraining(request)) throw unexpected;
		// §10.9: the subscriber sends nothing else on the stream of its
		// SUBSCRIBE.
		if (request.kind === "PEER_SUBSCRIBE") {
			if (message.type !== "REQUEST_UPDATE") throw unexpected;
			onUpdateBeforeAnswer(request, message, out);
			return;
		}
		switch (message.type) {
			case "REQUEST_ERROR":
				if (request.state !== "Pending") throw unexpected;
				onRequestError(request, message, out);
				return;
			case "REQUEST_OK":
				if (
					request.state !== "Pending" ||
					request.kind === "SUBSCRIBE" ||
					request.kind === "FETCH"
				) {
					throw unexpected;
				}
				onRequestOk(request, message, out);
				return;
			case "FETCH_OK":
				if (request.state !== "Pending" || request.kind !== "FETCH") {
					throw unexpected;
				}
				onFetchOk(request, message, out);
				return;
			case "SUBSCRIBE_OK":
				if (request.state !== "Pending" || request.kind !== "SUBSCRIBE") {
					throw unexpected;
				}
				onSubscribeOk(request, message, out);
				return;
			case "PUBLISH_DONE": {
				if (request.state !== "Established" || request.kind !== "SUBSCRIBE") {
					throw unexpected;
				}
				const { trackAlias } = request;
				if (trackAlias === undefined) throw unexpected;
				onPublishDone(request, trackAlias, message, out);
				return;
			}
			case "REQUEST_UPDATE":
				// §10.9: only the subscriber of an Established subscription this
				// endpoint is the publisher of may send it.
				if (request.state !== "Established" || request.kind !== "PUBLISH") {
					throw unexpected;
				}
				onRequestUpdate(request, message, out);
				return;
			default:
				throw unexpected;
		}
	};

	/**
	 * Sends a request on a new bidirectional stream. `build` gets the Request
	 * ID and stream; no request is registered and no Request ID is used up if
	 * it or the encoding throws.
	 */
	const openRequest = (
		build: (common: RequestCommon) => {
			entry: RequestEntry;
			message: ControlMessage;
		},
	): SessionResult<bigint> => {
		const { entry, message } = build({
			requestId: nextRequestId,
			stream: options.allocateStream(),
			state: "Pending",
			inbound: new Uint8Array(0),
			sendClosed: false,
			endedLocally: false,
			peerFinished: false,
		});
		const data = encodeControlMessage(message);
		// §10.1: each endpoint increments its Request ID by 2.
		nextRequestId += 2n;
		requests.set(entry.requestId, entry);
		requestStreams.set(entry.stream, entry);
		return {
			value: entry.requestId,
			outputs: [{ type: "open-bidi-stream", stream: entry.stream, data }],
		};
	};

	const activePublish = (requestId: bigint): PublishEntry => {
		const request = requests.get(requestId);
		if (request?.kind !== "PUBLISH") {
			throw new InactiveError(
				`Request ID ${requestId} is not an active PUBLISH of this session`,
			);
		}
		return request;
	};

	const pendingPeerSubscribe = (requestId: bigint): PeerSubscribeEntry => {
		const request = requests.get(requestId);
		if (request?.kind !== "PEER_SUBSCRIBE") {
			throw new InactiveError(
				`Request ID ${requestId} is not a SUBSCRIBE of the peer that waits for its answer`,
			);
		}
		return request;
	};

	/**
	 * A subscription this endpoint is the publisher of, whose subscriber can
	 * end its direction of the request stream and leave the subscription as
	 * it is: answered or not.
	 */
	const outlivesPeerFin = (request: RequestEntry): boolean =>
		(request.kind === "PUBLISH" && request.state === "Established") ||
		(request.kind === "PEER_SUBSCRIBE" && request.state === "Pending");

	return {
		incoming,
		outgoing,

		publishNamespace(request) {
			return openRequest((common) => ({
				entry: { ...common, kind: "PUBLISH_NAMESPACE" },
				message: {
					type: "PUBLISH_NAMESPACE",
					requestId: common.requestId,
					trackNamespace: request.trackNamespace,
					parameters: request.parameters ?? {},
				},
			}));
		},

		publish(request) {
			if (publishedAliases.has(request.trackAlias)) {
				throw new RangeError(
					`Track Alias ${request.trackAlias} is already in use (§11.1)`,
				);
			}
			// §5.1: one subscription to a Track in each role. The Rust session
			// refuses the same call.
			if (isPublishing(request)) {
				throw new RangeError(
					"this endpoint is already the publisher of a subscription to the Track (§5.1)",
				);
			}
			const parameters = request.parameters ?? {};
			const result = openRequest((common) => ({
				entry: {
					...common,
					kind: "PUBLISH",
					openedBy: "self",
					fullTrackName: {
						trackNamespace: request.trackNamespace,
						trackName: request.trackName,
					},
					trackAlias: request.trackAlias,
					forward: parameters.forward ?? DEFAULT_FORWARD,
				},
				message: {
					type: "PUBLISH",
					requestId: common.requestId,
					trackNamespace: request.trackNamespace,
					trackName: request.trackName,
					trackAlias: request.trackAlias,
					parameters,
					trackProperties: request.trackProperties ?? [],
				},
			}));
			publishedAliases.add(request.trackAlias);
			return result;
		},

		subscribe(request) {
			return openRequest((common) => ({
				entry: { ...common, kind: "SUBSCRIBE", trackAlias: undefined },
				message: {
					type: "SUBSCRIBE",
					requestId: common.requestId,
					trackNamespace: request.trackNamespace,
					trackName: request.trackName,
					parameters: request.parameters ?? {},
				},
			}));
		},

		fetch(request) {
			const { range } = request;
			// §10.12.2: "a subscription in the same session in the Established or
			// Pending (subscriber) states". FIXME: a subscription the peer made
			// with PUBLISH can be joined as well (§5.1); this endpoint accepts no
			// PUBLISH, so it has none (docs/moqt-implementation-notes.md, known
			// gaps).
			// A SUBSCRIBE that got its PUBLISH_DONE is over for this, though it
			// is kept here for the streams still on their way (§10.11): the
			// Rust session has dropped it by then, and refuses the same.
			const refuseUnlessOpenSubscribe = (joiningRequestId: bigint): void => {
				const joined = requests.get(joiningRequestId);
				if (joined?.kind !== "SUBSCRIBE" || isDraining(joined)) {
					throw new RangeError(
						`Joining Request ID ${joiningRequestId} is not an open SUBSCRIBE of this session (§10.12.2)`,
					);
				}
			};
			let start: Location | undefined;
			switch (range.fetchType) {
				case "Standalone":
					if (endsBefore(range.startLocation, range.endLocation)) {
						throw new RangeError(
							"the range of the FETCH ends before it starts (§10.12.3)",
						);
					}
					start = range.startLocation;
					break;
				case "RelativeJoining":
					refuseUnlessOpenSubscribe(range.joiningRequestId);
					break;
				case "AbsoluteJoining":
					refuseUnlessOpenSubscribe(range.joiningRequestId);
					// §10.12.2.1: "the publisher sets the Start Location to
					// {Joining Start, 0}".
					start = { group: range.joiningStart, object: 0n };
					break;
			}
			return openRequest((common) => ({
				entry: { ...common, kind: "FETCH", start, fetchStream: "none" },
				message: {
					type: "FETCH",
					requestId: common.requestId,
					range,
					parameters: request.parameters ?? {},
				},
			}));
		},

		fetchStreamOpened(requestId, peerStream, out) {
			const request = requests.get(requestId);
			if (request?.kind === "FETCH") {
				if (request.fetchStream !== "none") {
					throw protocolViolation("a second fetch stream for one FETCH");
				}
				request.fetchStream = { peerStream };
				return "fetch";
			}
			// §10.1: the client's Request IDs are the even ones, counted up by 2.
			const issued = requestId % 2n === 0n && requestId < nextRequestId;
			// A SUBSCRIBE that got its PUBLISH_DONE is a request that is over,
			// as in the Rust session, which has dropped it by then; it is kept
			// here only for the streams still on their way (§10.11).
			const open = request !== undefined && !isDraining(request);
			if (open || !issued) {
				throw protocolViolation("fetch stream for no FETCH of this endpoint");
			}
			out.push({
				type: "stop-sending",
				peerStream,
				code: StreamResetErrorCode.CANCELLED,
			});
			return "ended";
		},

		fetchStreamData(requestId, peerStream, data, fin, out) {
			const request = fetchOfStream(requestId, peerStream);
			if (request === undefined) return;
			if (data.length > 0) {
				out.push({
					type: "event",
					event: { type: "fetch-data", requestId, data },
				});
			}
			if (fin) fetchStreamEnded(request, "fin", undefined, out);
		},

		fetchStreamReset(requestId, peerStream, code, out) {
			const request = fetchOfStream(requestId, peerStream);
			if (request !== undefined) fetchStreamEnded(request, "reset", code, out);
		},

		cancelRequest(requestId) {
			const request = requests.get(requestId);
			if (request === undefined) return [];
			const out: SessionOutput[] = [];
			endRequest(request, out, () =>
				resetRequestStream(request, StreamResetErrorCode.CANCELLED, out),
			);
			return out;
		},

		peerSubscribeOpened(message, peerStream, out) {
			// The order of the Rust session (`on_peer_request_opened`): the
			// subscription that exists is named before the load is.
			if (isPublishing(message)) {
				return { refused: RequestErrorCode.DUPLICATE_SUBSCRIPTION };
			}
			if (openPeerRequests() >= options.limits.maxPeerRequests) {
				return { refused: RequestErrorCode.EXCESSIVE_LOAD };
			}
			const entry: PeerSubscribeEntry = {
				requestId: message.requestId,
				stream: options.allocateStream(),
				state: "Pending",
				inbound: new Uint8Array(0),
				sendClosed: false,
				endedLocally: false,
				peerFinished: false,
				kind: "PEER_SUBSCRIBE",
				fullTrackName: {
					trackNamespace: message.trackNamespace,
					trackName: message.trackName,
				},
				forward: message.parameters.forward ?? DEFAULT_FORWARD,
			};
			requests.set(entry.requestId, entry);
			requestStreams.set(entry.stream, entry);
			out.push({ type: "adopt-peer-stream", peerStream, stream: entry.stream });
			out.push({
				type: "event",
				event: {
					type: "peer-subscribe",
					requestId: message.requestId,
					trackNamespace: message.trackNamespace,
					trackName: message.trackName,
					parameters: message.parameters,
				},
			});
			return { stream: entry.stream };
		},

		isOpenPeerSubscribe(requestId) {
			const request = requests.get(requestId);
			return (
				request?.kind === "PEER_SUBSCRIBE" ||
				(request?.kind === "PUBLISH" && request.openedBy === "peer")
			);
		},

		acceptSubscribe(requestId, answer) {
			const pending = pendingPeerSubscribe(requestId);
			let trackAlias = nextServedAlias;
			while (publishedAliases.has(trackAlias)) trackAlias += 1n;
			// Before anything changes: a parameter that is none of SUBSCRIBE_OK
			// throws here.
			const data = encodeControlMessage({
				type: "SUBSCRIBE_OK",
				trackAlias,
				parameters: answer.parameters ?? {},
				trackProperties: answer.trackProperties ?? [],
			});
			nextServedAlias = trackAlias + 1n;
			publishedAliases.add(trackAlias);
			const { kind: _, ...kept } = pending;
			const entry: PublishEntry = {
				...kept,
				kind: "PUBLISH",
				openedBy: "peer",
				state: "Established",
				trackAlias,
			};
			requests.set(requestId, entry);
			// A subscriber that had finished its direction is still known by
			// its stream: its STOP_SENDING can come.
			requestStreams.set(entry.stream, entry);
			return {
				value: trackAlias,
				outputs: [{ type: "send", stream: entry.stream, data }],
			};
		},

		rejectPeerRequest(requestId, errorCode, errorReason) {
			const request = pendingPeerSubscribe(requestId);
			const data = encodeControlMessage({
				type: "REQUEST_ERROR",
				errorCode,
				retryInterval: 0n,
				errorReason,
			});
			const out: SessionOutput[] = [];
			// What the peer sent before it saw the answer still arrives, and
			// gets none.
			request.endedLocally = true;
			endRequest(request, out, () => {
				out.push({ type: "send", stream: request.stream, data });
				out.push({
					type: "event",
					event: {
						type: "peer-request-rejected",
						messageType: SUBSCRIBE_MESSAGE_TYPE,
						requestId,
						errorCode,
					},
				});
				finishSend(request, out);
			});
			// The subscriber had finished its direction already; nothing more
			// comes on the stream.
			if (request.peerFinished) requestStreams.delete(request.stream);
			return out;
		},

		// §5.1: "The publisher terminates a subscription in the Pending
		// (Publisher) or Established states by sending PUBLISH_DONE".
		publishDone(requestId, statusCode, errorReason) {
			const request = activePublish(requestId);
			if (outgoing.hasOpen(requestId)) {
				throw new Error(
					"PUBLISH_DONE must not be sent while subgroup streams are open (§10.11)",
				);
			}
			const data = encodeControlMessage({
				type: "PUBLISH_DONE",
				statusCode,
				streamCount: outgoing.streamCount(requestId),
				errorReason,
			});
			const out: SessionOutput[] = [];
			request.endedLocally = true;
			endRequest(request, out, () => {
				// A peer that stopped the stream reads no PUBLISH_DONE.
				if (!request.sendClosed) {
					out.push({ type: "send", stream: request.stream, data });
				}
				finishSend(request, out);
			});
			// The subscriber had finished its direction already; nothing more
			// comes on the stream.
			if (request.peerFinished) {
				requestStreams.delete(request.stream);
				reportPeerEndAfterPublishDone(request, "fin", undefined, out);
			}
			return out;
		},

		publishDonePeerEndExpired(requestId, out) {
			for (const [stream, request] of requestStreams) {
				if (request.requestId !== requestId || !awaitsPeerEnd(request)) {
					continue;
				}
				// What the peer sends on the stream from here on is not read.
				requestStreams.delete(stream);
				out.push({
					type: "stop-peer-direction",
					stream,
					code: StreamResetErrorCode.CANCELLED,
				});
				return;
			}
		},

		openSubgroup(requestId, subgroupOptions) {
			const request = activePublish(requestId);
			// §5.1: Objects may be sent before PUBLISH_OK, but never in Forward
			// State 0. A stream stays open only while this holds: `endRequest` and
			// a PUBLISH_OK with FORWARD 0 reset it, so writing needs no check.
			if (!request.forward) {
				throw new Error("the subscription is in Forward State 0 (§5.1)");
			}
			return outgoing.open(requestId, request.trackAlias, subgroupOptions);
		},

		subscriptionExpired: endAfterPublishDone,

		receiveStreamData(stream, data, fin, out) {
			const request = requestStreams.get(stream);
			if (request === undefined) return;
			request.inbound = drainControlMessages(
				request.inbound,
				data,
				(message) => {
					// A response handled earlier in this call made this endpoint
					// reset the stream; what the peer sent after it is not read.
					if (requestStreams.get(stream) !== request) return;
					// §5.1: the peer answered, or updated, before it saw that this
					// endpoint had ended the request. That is not a second response,
					// and it is ignored. An update still used up a Request ID of the
					// peer (§10.1), which is recorded so that it leaves no gap.
					if (request.endedLocally) {
						if (message.type === "REQUEST_UPDATE") {
							options.checkPeerRequestId(message.requestId);
						}
						return;
					}
					if (request.state === "Terminated") {
						throw protocolViolation(
							`${message.type} on a ${request.kind} request stream that had ended`,
						);
					}
					onResponse(request, message, out);
				},
			);
			if (!fin || requestStreams.get(stream) !== request) return;
			if (request.inbound.length > 0) {
				throw protocolViolation("FIN inside a control message");
			}
			// §5.1: the subscriber of a subscription made with PUBLISH ends it with
			// STOP_SENDING. A subscriber that only finishes its direction can send
			// no REQUEST_UPDATE any more, and has asked for nothing to end.
			// The same holds for a SUBSCRIBE of the peer, before its answer and
			// after: the Rust client goes on serving a subscriber that only
			// finished its direction.
			// WHY the stream stays known: STOP_SENDING for it can still come.
			if (outlivesPeerFin(request)) {
				request.peerFinished = true;
				return;
			}
			requestStreams.delete(stream);
			// The publisher has answered and has nothing more to say on the
			// request stream; the FETCH goes on until its fetch stream ends
			// (§5.2).
			if (request.kind === "FETCH" && request.state === "Established") return;
			if (request.state === "Terminated") {
				reportPeerEndAfterPublishDone(request, "fin", undefined, out);
				return;
			}
			// After PUBLISH_DONE the FIN is expected (§10.11) and the subscription
			// lives on until its late streams are accounted for.
			if (isDraining(request)) return;
			// §11.4.1: the end of a request stream ends the request.
			endRequest(request, out, () => {
				out.push(closedEvent(request, "peer-fin"));
				finishSend(request, out);
			});
		},

		streamReset(stream, code, out) {
			const request = requestStreams.get(stream);
			if (request === undefined) return;
			// Nothing more arrives on the stream. A subscription that already has
			// its PUBLISH_DONE needs nothing more from it, and keeps draining so
			// that late Objects still arrive (§10.11).
			if (request.state === "Terminated" || isDraining(request)) {
				requestStreams.delete(stream);
				reportPeerEndAfterPublishDone(request, "reset", code, out);
				return;
			}
			abandonRequest(request, "peer-reset", out, code);
		},

		streamStopped(stream, code, out) {
			const request = requestStreams.get(stream);
			if (request === undefined) return;
			request.sendClosed = true;
			// §5.1: STOP_SENDING is how the subscriber ends a subscription made
			// with PUBLISH.
			// WHY NOT end every request that is stopped: STOP_SENDING closes only
			// the direction this endpoint sends in. The response to a SUBSCRIBE,
			// its PUBLISH_DONE and the Objects of a draining subscription are
			// still on their way and have to be read.
			// A subscriber that stops the stream of its SUBSCRIBE reads no
			// answer and no PUBLISH_DONE on it (§3.3.2 has it cancel that way).
			if (outlivesPeerFin(request)) {
				abandonRequest(request, "peer-reset", out, code);
			}
		},

		heldStreams: () => requestStreams.size,

		clear() {
			requests.clear();
			requestStreams.clear();
			publishedAliases.clear();
			nextServedAlias = 0n;
			outgoing.clear();
			incoming.clear();
		},
	};
}
