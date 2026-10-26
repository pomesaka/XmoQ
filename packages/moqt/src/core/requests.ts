import type { Bytes } from "./bytes.ts";
import {
	type ControlMessage,
	checkRequestGoAway,
	drainControlMessages,
	encodeControlMessage,
	FETCH_MESSAGE_TYPE,
	type Fetch,
	type FetchOk,
	type FetchRange,
	type GoAway,
	type PublishDone,
	type RequestError,
	type RequestOk,
	type RequestUpdate,
	SUBSCRIBE_MESSAGE_TYPE,
	type Subscribe,
	type SubscribeOk,
	TRACK_STATUS_MESSAGE_TYPE,
	type TrackStatus,
} from "./control-message.ts";
import {
	InactiveError,
	MoqtError,
	madeUpRetryInterval,
	protocolViolation,
	RequestErrorCode,
	SessionErrorCode,
	StreamResetErrorCode,
} from "./error.ts";
import {
	createFetchStreamEncoder,
	type FetchObject,
	type FetchStreamEncoder,
	type RangeEnd,
} from "./fetch.ts";
import { trackPropertiesRefusal } from "./immutable-properties.ts";
import {
	createIncomingSubgroups,
	type IncomingSubgroups,
} from "./incoming-subgroups.ts";
import type { KeyValuePair } from "./key-value-pair.ts";
import type { SessionLimits } from "./limits.ts";
import {
	createOutgoingSubgroups,
	type OutgoingSubgroups,
	type SubgroupOptions,
} from "./outgoing-subgroups.ts";
import {
	endsBefore,
	type GroupOrder,
	type Location,
	type MessageParameters,
	parametersOutsideScope,
} from "./parameters.ts";
import {
	DATA_STREAM_SEND_ORDER,
	type RequestClosedReason,
	type SessionOutput,
	type SessionResult,
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
	/**
	 * The longest Object Payload Length taken on the subgroup streams of
	 * this subscription, in bytes: a whole number from 1. Nothing of this
	 * goes on the wire; it is a limit of this endpoint, like
	 * `limits.maxObjectPayloadLength`, and an Object over it ends its stream
	 * in the same way (`subgroup-abandoned`, STOP_SENDING with
	 * INTERNAL_ERROR), while the subscription goes on. Left out, or above
	 * the connection's limit, the connection's limit is the one.
	 *
	 * WHY above it is the connection's and not a `RangeError`: the
	 * connection's limit bounds every stream anyway, so a larger number asks
	 * for nothing that could be given, and a caller that is handed a
	 * connection (`@xmoq/msf`, `@xmoq/chat`) does not know its limit.
	 *
	 * It applies from the SUBSCRIBE_OK on. An Object whose Object Payload
	 * Length was read before then, while the Track Alias of its stream was
	 * unknown, was taken under the connection's limit and is delivered.
	 */
	maxObjectPayloadLength?: number;
}

export interface FetchRequest {
	range: FetchRange;
	parameters?: MessageParameters;
}

/** What a SUBSCRIBE_NAMESPACE asks for (§10.18). AUTHORIZATION_TOKEN is the one parameter it may carry (§10.2.2). */
export interface SubscribeNamespaceRequest {
	/** 0 to 32 fields; none asks for every namespace (§6.1). */
	trackNamespacePrefix: TrackNamespace;
	parameters?: MessageParameters;
}

/** What a TRACK_STATUS asks about (§10.14). AUTHORIZATION_TOKEN is the one parameter it may carry (§10.2.2). */
export interface TrackStatusRequest extends FullTrackName {
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

/**
 * What the application answers a SUBSCRIBE of the peer with (§10.8), and a
 * TRACK_STATUS: "the same parameters and Track Properties it would have set
 * in a SUBSCRIBE_OK" (§10.14), of which LARGEST_OBJECT alone may appear in
 * its REQUEST_OK (§10.2.11).
 */
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

/** What the application answers a FETCH of the peer with: the fields of FETCH_OK (§10.13). */
export interface FetchAnswer {
	/**
	 * The end of what the fetch stream will cover, in the encoding of a
	 * FETCH: the last Object plus 1, or Object 0 for the whole Group.
	 */
	endLocation: Location;
	/** Whether `endLocation` is the end of the Track. False when left out. */
	endOfTrack?: boolean;
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
	/** How many requests of the peer that the session refused by itself are kept until their stream is gone. */
	refusedPeerRequests(): number;
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
	/**
	 * Throws a `RangeError`, and sends nothing, for a `maxObjectPayloadLength`
	 * that is not a whole number from 1.
	 */
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
	 * Asks what a SUBSCRIBE of the Track would be answered with (§10.14).
	 * The one answer ends the request: the event `track-status-ok`, or
	 * `request-error`. Throws a `RangeError`, and sends nothing, for a
	 * parameter that may not appear in TRACK_STATUS: any but
	 * AUTHORIZATION_TOKEN (§10.2.2).
	 */
	trackStatus(request: TrackStatusRequest): SessionResult<bigint>;
	/**
	 * Asks the peer for the namespaces under a prefix, now and as they come
	 * and go (§6.1, §10.18). The answer is the event `namespaces-subscribed`
	 * or `request-error`; then each NAMESPACE and NAMESPACE_DONE is the event
	 * `namespace`, with the whole namespace, the prefix before the suffix the
	 * peer sent (§10.16). The request ends with the peer's FIN or reset, after
	 * which every namespace told of counts as done (§10.18, "SHOULD treat this
	 * as though each active namespace received a NAMESPACE_DONE"; the
	 * application is told `request-closed`, and the session tells of no
	 * NAMESPACE_DONE for them), with `cancelRequest`, or with the session.
	 *
	 * More than `MAX_NAMESPACES_OF_A_SUBSCRIPTION` namespaces at once end the
	 * request alone: its stream is reset and the peer's direction stopped
	 * with CANCELLED, as the Rust callers do, and the application is told
	 * `request-closed` of `too-many-namespaces`.
	 *
	 * Throws a `RangeError`, and sends nothing, for a parameter other than
	 * AUTHORIZATION_TOKEN and for a prefix of more than 32 fields.
	 */
	subscribeNamespace(request: SubscribeNamespaceRequest): SessionResult<bigint>;
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
	 * Asks the publisher to change parameters of an Established subscription
	 * this endpoint made with SUBSCRIBE (§10.9): sends a REQUEST_UPDATE on
	 * the stream of that SUBSCRIBE. A parameter that is left out stays as
	 * it was. The answer comes as the event `update-ok` or `update-error`.
	 *
	 * Throws a `RangeError` for a request that is not an Established
	 * SUBSCRIBE of this endpoint, for a parameter that may not appear in
	 * REQUEST_UPDATE, and when `MAX_UNANSWERED_UPDATES` updates wait for
	 * their answer. Nothing is sent and no Request ID is used up then.
	 * **A publisher that answers none leaves the subscription unable to be
	 * updated after that many**: nothing here has a clock, and a caller
	 * that stops waiting for an answer does not make room.
	 */
	updateSubscription(
		requestId: bigint,
		parameters: MessageParameters,
	): SessionOutput[];
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
	/**
	 * A Standalone FETCH of the peer, whole, with a range that does not end
	 * before it starts and a Request ID that was checked. Like
	 * `peerSubscribeOpened`: refused here, or its stream is adopted and it
	 * waits for `acceptFetch` or `rejectPeerRequest`.
	 */
	peerFetchOpened(
		message: Fetch,
		peerStream: number,
		out: SessionOutput[],
	): PeerSubscribeOpened;
	/**
	 * A TRACK_STATUS of the peer, whole, with a Request ID that was checked.
	 * Like `peerSubscribeOpened`: refused here, or its stream is adopted and
	 * it waits for `acceptTrackStatus` or `rejectPeerRequest`.
	 */
	peerTrackStatusOpened(
		message: TrackStatus,
		peerStream: number,
		out: SessionOutput[],
	): PeerSubscribeOpened;
	/**
	 * Answers a TRACK_STATUS of the peer with REQUEST_OK (TRACK_STATUS_OK)
	 * and a FIN: "The bidi stream is closed with
	 * a FIN after TRACK_STATUS_OK or REQUEST_ERROR are sent" (§10.14). How the
	 * peer then ends its direction is reported as after a refusal
	 * (`peer-direction-ended`). Throws a `RangeError`, and leaves the request
	 * waiting, for a parameter other than LARGEST_OBJECT (§10.2.11).
	 */
	acceptTrackStatus(
		requestId: bigint,
		answer: SubscribeAnswer,
	): SessionOutput[];
	/**
	 * Accepts a FETCH of the peer: sends FETCH_OK, then a REQUEST_OK for each
	 * REQUEST_UPDATE that came before, and opens the fetch stream with its
	 * FETCH_HEADER. Throws a `RangeError`, and leaves the FETCH waiting, for
	 * an End Location before the Start Location of the FETCH: its receiver
	 * would close the session (§10.13). An End Location beyond what was asked
	 * for is not checked, as in the Rust session.
	 */
	acceptFetch(requestId: bigint, answer: FetchAnswer): SessionOutput[];
	/**
	 * Writes the next Object of an accepted FETCH to its fetch stream. Throws
	 * a `RangeError`, and writes nothing, for an Object that does not come
	 * after what was written before it by the Group Order of the FETCH;
	 * whether it lies in the range that was asked for is not checked.
	 */
	writeFetchObject(requestId: bigint, object: FetchObject): SessionOutput[];
	/** Writes an End of Range indicator to the fetch stream (§11.4.4.2). */
	writeFetchRangeEnd(
		requestId: bigint,
		kind: RangeEnd,
		location: Location,
	): SessionOutput[];
	/**
	 * The fetch stream is whole: it is finished, and the FETCH is over. This
	 * endpoint's direction of the request stream is finished with it where
	 * the subscriber has ended its own, and else when the subscriber does, or
	 * with `finishServedFetchSend`.
	 */
	finishFetch(requestId: bigint): SessionOutput[];
	/**
	 * The caller's wait for the subscriber of a FETCH that was served to end
	 * its direction of the request stream ran out: this endpoint finishes its
	 * own. Does nothing when that is done.
	 */
	finishServedFetchSend(requestId: bigint, out: SessionOutput[]): void;
	/** Gives an accepted FETCH up: both of its streams are reset (§5.2). */
	resetFetch(requestId: bigint, code: number): SessionOutput[];
	/** Whether the Request ID names a SUBSCRIBE of the peer that waits for its answer or was accepted, and is not over (§10.12.2). */
	isOpenPeerSubscribe(requestId: bigint): boolean;
	/** Whether a request of the peer with this Request ID is kept: open, or over with its stream not yet gone. */
	holdsPeerRequestId(requestId: bigint): boolean;
	/** How many requests of the peer are kept in that way. */
	heldPeerRequests(): number;
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
	 * The caller's wait for the streams of a subscription whose
	 * REQUEST_UPDATE was refused ran out (`update-refused`): the streams that
	 * are left are reset, and PUBLISH_DONE UPDATE_FAILED is sent. Does
	 * nothing where it has ended.
	 */
	updateRefusalExpired(requestId: bigint, out: SessionOutput[]): void;
	/**
	 * The wait for the subscriber to end its direction of the request stream
	 * after this endpoint's PUBLISH_DONE is over: that direction is stopped.
	 * Does nothing when the subscriber has ended it.
	 */
	peerEndExpired(requestId: bigint, out: SessionOutput[]): void;
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
	/**
	 * A REQUEST_UPDATE of the subscriber was refused: the subscription ends
	 * with PUBLISH_DONE UPDATE_FAILED once its streams are closed.
	 */
	endsWithUpdateFailed?: true;
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
	/** The track (`trackKey`). */
	track: string;
	/** Set by SUBSCRIBE_OK. */
	trackAlias: bigint | undefined;
	/** `SubscribeRequest.maxObjectPayloadLength`, resolved against the connection's limit. */
	maxObjectPayloadLength: number;
	/** REQUEST_UPDATEs this endpoint sent that have no answer yet (`updateSubscription`). */
	updates: number;
	/** Whether the publisher refused one of them. */
	updateRefused: boolean;
}

/**
 * A FETCH. Its state is Pending until FETCH_OK and Established after; the
 * draft names no states for it (§5.2).
 */
interface FetchEntry extends RequestCommon {
	kind: "FETCH";
	/** The track (`trackKey`): that of the FETCH, or of the SUBSCRIBE a Joining FETCH names. */
	track: string;
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

/**
 * A Standalone FETCH of the peer: Pending until the application answers,
 * Established while its fetch stream is written.
 */
interface PeerFetchEntry extends RequestCommon {
	kind: "PEER_FETCH";
	start: Location;
	groupOrder: GroupOrder;
	/** REQUEST_UPDATEs that came before the answer and get their REQUEST_OK after the FETCH_OK. */
	heldUpdates: number;
	/** The fetch stream and what encodes it, once the FETCH was accepted. */
	writing: { stream: number; encoder: FetchStreamEncoder } | undefined;
	/** This endpoint finished the fetch stream: how the peer ends its direction of the request stream is waited for, as after PUBLISH_DONE. */
	served: boolean;
}

/** A SUBSCRIBE_NAMESPACE of this endpoint (§10.18). */
interface SubscribeNamespaceEntry extends RequestCommon {
	kind: "SUBSCRIBE_NAMESPACE";
	prefix: TrackNamespace;
	/** The suffixes the peer named with NAMESPACE and has not ended, by `namespaceKey`. */
	named: Set<string>;
}

/** A TRACK_STATUS of this endpoint, which its one answer ends (§10.14). */
interface TrackStatusEntry extends RequestCommon {
	kind: "TRACK_STATUS";
}

/** A TRACK_STATUS of the peer that waits for the application's answer. */
interface PeerTrackStatusEntry extends RequestCommon {
	kind: "PEER_TRACK_STATUS";
}

type RequestEntry =
	| NamespaceEntry
	| PublishEntry
	| PeerSubscribeEntry
	| PeerFetchEntry
	| PeerTrackStatusEntry
	| SubscribeEntry
	| FetchEntry
	| TrackStatusEntry
	| SubscribeNamespaceEntry;

/**
 * Namespaces one SUBSCRIBE_NAMESPACE of this endpoint keeps at once: decided
 * on 2026-10-24, as many as the own relay tells of under a prefix, and the
 * same as the Rust session (`MAX_NAMESPACES_OF_A_SUBSCRIPTION`). WHY a limit:
 * how many the peer names is the peer's to choose, and each is kept until its
 * NAMESPACE_DONE.
 */
export const MAX_NAMESPACES_OF_A_SUBSCRIPTION = 4096;

/** §2.4.1: a Track Namespace has at most 32 fields, and with a Track Name at most 4,096 bytes. */
const MAX_NAMESPACE_FIELDS = 32;
const MAX_NAMESPACE_BYTES = 4096;

/** A key of a namespace for a `Set`: the fields' bytes in hex, joined with dots. */
const namespaceKey = (namespace: TrackNamespace): string =>
	namespace
		.map((field) =>
			Array.from(field, (byte) => byte.toString(16).padStart(2, "0")).join(""),
		)
		.join(".");

/** A key of a Full Track Name for comparing: its namespace's key, a slash, and its name in hex. */
const trackKey = (namespace: TrackNamespace, name: Uint8Array): string =>
	`${namespaceKey(namespace)}/${namespaceKey([name])}`;

/** §10.2.12: FORWARD defaults to 1 wherever it is omitted, except in REQUEST_UPDATE. */
const DEFAULT_FORWARD = true;

/** REQUEST_OK without a parameter: the answer to a REQUEST_UPDATE that changes nothing. */
const REQUEST_OK = encodeControlMessage({
	type: "REQUEST_OK",
	parameters: {},
	trackProperties: [],
});

/**
 * REQUEST_UPDATEs for a FETCH of the peer that may wait for the answer to
 * the FETCH, each of which is owed a REQUEST_OK after it. One more and the
 * FETCH is refused with EXCESSIVE_LOAD.
 *
 * WHY a limit: the answers are sent in one go when the application's comes,
 * and how many there are is the peer's to choose. The number and what
 * happens beyond it are the relay's (`MAX_HELD_FETCH_UPDATES` in
 * crates/relay-core/src/fetch.rs); the Rust session leaves it to its
 * caller. Provisional.
 */
const MAX_HELD_FETCH_UPDATES = 64;

/**
 * REQUEST_UPDATEs of one subscription that may wait for their answer.
 * **Provisional**; `MAX_UNANSWERED_UPDATES` of the Rust session is the same.
 * WHY a limit: a publisher that answers none would have the count grow with
 * every call.
 */
export const MAX_UNANSWERED_UPDATES = 16;

/** The Message Type that opened each kind of request of the peer that waits for its answer. */
const PEER_REQUEST_MESSAGE_TYPES: Record<
	(PeerSubscribeEntry | PeerFetchEntry | PeerTrackStatusEntry)["kind"],
	bigint
> = {
	PEER_SUBSCRIBE: SUBSCRIBE_MESSAGE_TYPE,
	PEER_FETCH: FETCH_MESSAGE_TYPE,
	PEER_TRACK_STATUS: TRACK_STATUS_MESSAGE_TYPE,
};

/** PUBLISH_DONE status code UPDATE_FAILED (§10.11). */
const UPDATE_FAILED = 0x8n;

/**
 * §10.2.1, with what §10.2 lets the update of a subscription carry: the codec
 * let through what the update of any request may carry. As in the Rust
 * session (`on_peer_request_message`).
 */
function checkUpdateOfSubscription(message: RequestUpdate): void {
	const outside = parametersOutsideScope(message.parameters, "REQUEST_UPDATE");
	if (outside.length > 0) {
		throw protocolViolation(
			`${outside.join(", ")} not allowed in the REQUEST_UPDATE of a subscription`,
		);
	}
}

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
	/** The FETCHes of the peer that are being served, by the number of their fetch stream. */
	const fetchStreams = new Map<number, PeerFetchEntry>();
	const outgoing = createOutgoingSubgroups(
		options.allocateStream,
		(requestId, out) => {
			const request = requests.get(requestId);
			if (request?.kind === "PUBLISH" && request.endsWithUpdateFailed) {
				endWithUpdateFailed(request, out);
			}
		},
	);
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

	const openedByThePeer = (request: RequestEntry): boolean =>
		request.kind === "PEER_SUBSCRIBE" ||
		request.kind === "PEER_FETCH" ||
		request.kind === "PEER_TRACK_STATUS" ||
		(request.kind === "PUBLISH" && request.openedBy === "peer");

	/**
	 * Requests of the peer that are kept here: the ones that wait for their
	 * answer or were accepted, and the ones that are over while their stream
	 * is not yet gone (rejected, served, or ended with PUBLISH_DONE). The
	 * Rust session counts the same (`peer_request_count`).
	 */
	const heldPeerRequests = (): number => {
		let count = 0;
		for (const request of requestStreams.values()) {
			if (openedByThePeer(request)) count += 1;
		}
		return count;
	};

	/** Whether the requests of the peer that are kept, here and where the session refused them, are as many as may be. */
	const peerRequestsAreFull = (): boolean =>
		heldPeerRequests() + options.refusedPeerRequests() >=
		options.limits.maxPeerRequests;

	/** FETCHes of the peer that are open: waiting for their answer, or being served. */
	const openPeerFetches = (): number => {
		let count = 0;
		for (const request of requests.values()) {
			if (request.kind === "PEER_FETCH") count += 1;
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
	 * A FETCH that was served kept this endpoint's direction of the request
	 * stream open for the subscriber to end its own first (`finishFetch`):
	 * finishes it now.
	 */
	const finishServedSend = (
		request: RequestEntry,
		out: SessionOutput[],
	): void => {
		if (request.kind === "PEER_FETCH" && request.served) {
			finishSend(request, out);
		}
	};

	/**
	 * This endpoint has said the last thing it says on the request stream and
	 * finished its direction, or, after a FETCH it served, will: PUBLISH_DONE
	 * for its PUBLISH (the application's or the UPDATE_FAILED of a refused
	 * REQUEST_UPDATE), the end of a FETCH it served, or REQUEST_ERROR for a
	 * request of the peer. How the peer
	 * then ends its direction is reported, and the caller's wait for it ends
	 * with `peerEndExpired`, so that a peer that does not end its direction
	 * does not hold the stream, and its place among the requests, for good.
	 */
	const awaitsPeerEnd = (request: RequestEntry): boolean =>
		request.endedLocally;

	const reportPeerEnd = (
		request: RequestEntry,
		how: "fin" | "reset",
		resetCode: number | undefined,
		out: SessionOutput[],
	): void => {
		if (!awaitsPeerEnd(request)) return;
		out.push({
			type: "event",
			event: {
				type: "peer-direction-ended",
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
		// §5.2: a publisher that learns of the cancellation "MUST reset the
		// bidi request stream and unidirectional data stream associated with
		// the FETCH". A fetch stream that was finished is not among these.
		if (request.kind === "PEER_FETCH" && request.writing !== undefined) {
			const { stream } = request.writing;
			request.writing = undefined;
			fetchStreams.delete(stream);
			out.push({ type: "reset", stream, code });
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

	/**
	 * §2.4.2: "When a subscriber detects a Malformed Track, it MUST cancel
	 * any corresponding subscription or fetches for that Track from that
	 * publisher". The subscription, then the FETCH requests of its track in
	 * the order they were sent, as the Rust session (`cancel_malformed`).
	 */
	function onMalformedTrack(requestId: bigint, out: SessionOutput[]): void {
		const request = requests.get(requestId);
		if (request === undefined) return;
		const fetches =
			request.kind === "SUBSCRIBE"
				? [...requests.values()].filter(
						(other) => other.kind === "FETCH" && other.track === request.track,
					)
				: [];
		abandonRequest(request, "malformed-track", out);
		for (const fetch of fetches) abandonRequest(fetch, "malformed-track", out);
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
			(request.kind === "PUBLISH_NAMESPACE" ||
				request.kind === "SUBSCRIBE_NAMESPACE") &&
			message.redirect !== undefined &&
			message.redirect.trackName.length > 0
		) {
			throw protocolViolation(
				`Redirect for ${request.kind} has a non-empty Track Name`,
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

	/**
	 * After a refusal the publisher ends the subscription (§10.9.1: it "MUST
	 * also terminate the subscription by sending a PUBLISH_DONE with error
	 * code UPDATE_FAILED"), and what answers the later updates is taken
	 * with no event. WHY each answer counts one, where a refusal could be
	 * read as the answer to all that wait: a publisher that coalesces sends
	 * "only a single REQUEST_ERROR" (§10.9.1), but one that does not answers
	 * each, and both are followed by the PUBLISH_DONE, which ends the
	 * request whatever still waits. `fixtures/update-answers/` has the rows,
	 * which the Rust session reads too.
	 */
	const onUpdateAnswered = (
		request: SubscribeEntry,
		message: RequestOk | RequestError,
		out: SessionOutput[],
	): void => {
		const { requestId } = request;
		if (message.type === "REQUEST_OK") {
			// What a REQUEST_UPDATE_OK may carry: §10.2.10, §10.2.11.
			const outside = Object.keys(message.parameters).filter(
				(name) => name !== "expires" && name !== "largestObject",
			);
			if (outside.length > 0) {
				throw protocolViolation(
					`${outside.join(", ")} not allowed in REQUEST_UPDATE_OK`,
				);
			}
			// §10.5: Track Properties are empty in REQUEST_UPDATE_OK.
			if (message.trackProperties.length > 0) {
				throw protocolViolation("Track Properties in REQUEST_UPDATE_OK");
			}
		}
		request.updates -= 1;
		if (request.updateRefused) return;
		if (message.type === "REQUEST_OK") {
			out.push({
				type: "event",
				event: { type: "update-ok", requestId, parameters: message.parameters },
			});
			return;
		}
		request.updateRefused = true;
		out.push({
			type: "event",
			event: {
				type: "update-error",
				requestId,
				errorCode: message.errorCode,
				retryInterval: message.retryInterval,
				errorReason: message.errorReason,
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
		// A Mandatory Track Property (§2.5.1: "MUST NOT process ... that
		// track"), and a Track that Immutable Properties make malformed (§12.7;
		// §2.4.2: "it MUST cancel any corresponding subscription"). A
		// SUBSCRIBE_OK that is not processed does not take the alias: a
		// draining subscription keeps it, and a free alias is only remembered,
		// so that streams the publisher opens for it are dropped at once (§11.1).
		const refusal = trackPropertiesRefusal(message.trackProperties);
		if (refusal !== undefined) {
			if (aliasState === "free") incoming.retire(message.trackAlias);
			abandonRequest(request, refusal, out);
			return;
		}
		incoming.establish(
			message.trackAlias,
			request.requestId,
			request.maxObjectPayloadLength,
			out,
		);
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

	/**
	 * TRACK_STATUS_OK (§10.14, a REQUEST_OK): what the peer would have set in
	 * a SUBSCRIBE_OK. LARGEST_OBJECT is the one parameter it may carry
	 * (§10.2.11), and its Track Properties are the Track's (§10.5). The
	 * request is over; the peer closes its stream with a FIN.
	 */
	const onTrackStatusOk = (
		request: TrackStatusEntry,
		message: RequestOk,
		out: SessionOutput[],
	): void => {
		const outside = parametersOutsideScope(
			message.parameters,
			"TRACK_STATUS_OK",
		);
		if (outside.length > 0) {
			throw protocolViolation(
				`${outside.join(", ")} not allowed in TRACK_STATUS_OK`,
			);
		}
		endRequest(request, out, () => {
			out.push({
				type: "event",
				event: {
					type: "track-status-ok",
					requestId: request.requestId,
					parameters: message.parameters,
					trackProperties: message.trackProperties,
				},
			});
			// WHY only now, where nothing was to be sent after the
			// TRACK_STATUS ("the first and only message", §10.14): this
			// package reads a FIN of a request's sender as its end (§11.4.1),
			// and a peer may read it so as well.
			finishSend(request, out);
		});
	};

	/**
	 * What comes on the stream of a SUBSCRIBE_NAMESPACE of this endpoint
	 * (§10.18): REQUEST_OK or REQUEST_ERROR first ("If the subscriber
	 * receives any message other than a REQUEST_OK or a REQUEST_ERROR as the
	 * first message ... it MUST close the session with a
	 * PROTOCOL_VIOLATION"), then NAMESPACE and NAMESPACE_DONE. As in the Rust
	 * session (`answered_namespaces`, `on_namespace`).
	 */
	const onNamespaceMessage = (
		request: SubscribeNamespaceEntry,
		message: ControlMessage,
		out: SessionOutput[],
	): void => {
		if (request.state === "Pending") {
			if (message.type === "REQUEST_ERROR") {
				onRequestError(request, message, out);
				return;
			}
			if (message.type !== "REQUEST_OK") {
				throw protocolViolation(
					`${message.type} before the answer to SUBSCRIBE_NAMESPACE`,
				);
			}
			// No parameter of §10.2 lists this REQUEST_OK, and no Track Property
			// belongs to it (§10.5).
			if (
				Object.keys(message.parameters).length > 0 ||
				message.trackProperties.length > 0
			) {
				throw protocolViolation(
					"a REQUEST_OK of SUBSCRIBE_NAMESPACE with parameters or properties",
				);
			}
			request.state = "Established";
			out.push({
				type: "event",
				event: { type: "namespaces-subscribed", requestId: request.requestId },
			});
			return;
		}
		if (message.type !== "NAMESPACE" && message.type !== "NAMESPACE_DONE") {
			throw protocolViolation(
				`unexpected ${message.type} on the SUBSCRIBE_NAMESPACE request stream`,
			);
		}
		const suffix = message.trackNamespaceSuffix;
		const trackNamespace = [...request.prefix, ...suffix];
		// A namespace longer than §2.4.1 allows breaks the rule for any
		// namespace.
		if (
			trackNamespace.length > MAX_NAMESPACE_FIELDS ||
			trackNamespace.reduce((sum, field) => sum + field.length, 0) >
				MAX_NAMESPACE_BYTES
		) {
			throw protocolViolation("a namespace of NAMESPACE that is too long");
		}
		const key = namespaceKey(suffix);
		if (message.type === "NAMESPACE_DONE") {
			// §10.18: "If a subscriber receives a NAMESPACE_DONE before the
			// corresponding NAMESPACE, it MUST close the session with a
			// 'PROTOCOL_VIOLATION'."
			if (!request.named.delete(key)) {
				throw protocolViolation(
					"NAMESPACE_DONE before the corresponding NAMESPACE",
				);
			}
			out.push({
				type: "event",
				event: {
					type: "namespace",
					requestId: request.requestId,
					done: true,
					trackNamespace,
				},
			});
			return;
		}
		// WHY a NAMESPACE for a namespace named before is told again: the draft
		// does not make it an error, and the Rust session tells it again.
		if (
			!request.named.has(key) &&
			request.named.size >= MAX_NAMESPACES_OF_A_SUBSCRIPTION
		) {
			abandonRequest(request, "too-many-namespaces", out);
			return;
		}
		request.named.add(key);
		out.push({
			type: "event",
			event: {
				type: "namespace",
				requestId: request.requestId,
				done: false,
				trackNamespace,
			},
		});
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
		// §2.5.1 and §12.7, as for SUBSCRIBE_OK: the track is not processed,
		// and a malformed one has its FETCH cancelled (§2.4.2).
		const refusal = trackPropertiesRefusal(message.trackProperties);
		if (refusal !== undefined) {
			abandonRequest(request, refusal, out);
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
	 * §10.9: the subscriber of an Established subscription may update it.
	 *
	 * An update with no parameter but delivery timeouts
	 * (SUBGROUP_DELIVERY_TIMEOUT, OBJECT_DELIVERY_TIMEOUT) is answered with
	 * REQUEST_OK. WHY the Object's too: a SUBSCRIBE that sets it is accepted
	 * the same way, and a parameter that is taken at first and ends the
	 * subscription when it is sent again would be the worse of the two. Nothing here changes by it: this endpoint
	 * acts on no delivery timeout, that of a SUBSCRIBE included (the relay
	 * does, and applies the update). WHY one without a parameter is
	 * accepted: §10.9 has "If a parameter previously set on the request is
	 * not present in REQUEST_UPDATE, its value remains unchanged", and
	 * §13.6.1 names "REQUEST_UPDATE with no modified Message Parameters" as
	 * a way to keep a session from idling out.
	 *
	 * NEW_GROUP_REQUEST is let by as well and handed to the application
	 * (§10.2.13). WHY accepted whatever the Track is: it changes nothing of
	 * the subscription, so a REQUEST_OK promises nothing that then does not
	 * happen ("The Original Publisher MAY delay the NEW_GROUP_REQUEST", and
	 * one that "does not support dynamic Groups ... ignores the parameter").
	 * The session does not know whether the Track has DYNAMIC_GROUPS.
	 *
	 * Any other update fails: §4 asks a limited endpoint for NOT_SUPPORTED,
	 * and §10.9.1 says "When a REQUEST_UPDATE is unsuccessful, the publisher
	 * MUST also terminate the subscription by sending a PUBLISH_DONE with
	 * error code UPDATE_FAILED." WHY not accepted without effect: a
	 * REQUEST_OK for a filter or a Forward State that then does not change
	 * would have the subscriber wait for Objects that do not come. The Rust
	 * session has the same rule (`only_the_subgroup_timeout`).
	 */
	const onRequestUpdate = (
		request: PublishEntry,
		message: RequestUpdate,
		out: SessionOutput[],
	): void => {
		// §10.1: REQUEST_UPDATE consumes a Request ID of its sender.
		options.checkPeerRequestId(message.requestId);
		checkUpdateOfSubscription(message);
		// §10.9.1 lets one REQUEST_ERROR answer several updates; what comes
		// while the subscription ends gets nothing, as in the Rust session.
		if (request.endsWithUpdateFailed) return;
		const { stream, requestId } = request;
		if (
			Object.keys(message.parameters).every(
				(name) =>
					name === "subgroupDeliveryTimeout" ||
					name === "objectDeliveryTimeout" ||
					name === "newGroupRequest",
			)
		) {
			out.push({ type: "send", stream, data: REQUEST_OK });
			const group = message.parameters.newGroupRequest;
			if (group !== undefined) {
				out.push({
					type: "event",
					event: { type: "new-group-request", requestId, group },
				});
			}
			return;
		}
		out.push({
			type: "send",
			stream,
			data: encodeControlMessage({
				type: "REQUEST_ERROR",
				errorCode: RequestErrorCode.NOT_SUPPORTED,
				retryInterval: madeUpRetryInterval(RequestErrorCode.NOT_SUPPORTED),
				errorReason: "REQUEST_UPDATE is not implemented",
			}),
		});
		// WHY the streams that are being written are let end, and not reset:
		// it is the update that is refused, and the subscriber has given up
		// none of the Objects it was sent; a reset would lose the ones of the
		// Group under way. §10.11: PUBLISH_DONE "after all data streams are
		// closed". The Rust client does the same (it refuses between two
		// Objects, each on a stream it has closed).
		request.endsWithUpdateFailed = true;
		if (!outgoing.hasOpen(requestId)) endWithUpdateFailed(request, out);
		else {
			out.push({ type: "event", event: { type: "update-refused", requestId } });
		}
	};

	/**
	 * The subscription whose REQUEST_UPDATE was refused has no stream open:
	 * PUBLISH_DONE UPDATE_FAILED, with the streams it opened, ends it.
	 */
	const endWithUpdateFailed = (
		request: PublishEntry,
		out: SessionOutput[],
	): void => {
		const streamCount = outgoing.streamCount(request.requestId);
		request.endedLocally = true;
		endRequest(request, out, () => {
			out.push({
				type: "send",
				stream: request.stream,
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
	 * Refuses a request of the peer that has no answer yet, for a
	 * REQUEST_UPDATE that came for it: the REQUEST_ERROR is the one answer
	 * the request gets, and the application is told that it is gone.
	 */
	const refuseBeforeAnswer = (
		request: PeerSubscribeEntry | PeerFetchEntry,
		errorCode: bigint,
		errorReason: string,
		out: SessionOutput[],
	): void => {
		request.endedLocally = true;
		endRequest(request, out, () => {
			out.push({
				type: "send",
				stream: request.stream,
				data: encodeControlMessage({
					type: "REQUEST_ERROR",
					errorCode,
					retryInterval: madeUpRetryInterval(errorCode),
					errorReason,
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
		checkUpdateOfSubscription(message);
		refuseBeforeAnswer(
			request,
			RequestErrorCode.NOT_SUPPORTED,
			"REQUEST_UPDATE is not implemented",
			out,
		);
	};

	/**
	 * A REQUEST_UPDATE for a FETCH of the peer. It is accepted and changes
	 * nothing: a refusal would cost the subscriber its fetch (§10.9.1 has the
	 * publisher reset the fetch stream then), where it may only have meant to
	 * change a priority, which nothing here uses.
	 *
	 * WHY one that comes before the answer is held: responses carry no
	 * Request ID, so a REQUEST_OK sent first would be read as the answer to
	 * the FETCH. The same in the Rust session (`fixtures/peer-fetch/`).
	 */
	const onUpdateOfPeerFetch = (
		request: PeerFetchEntry,
		message: RequestUpdate,
		out: SessionOutput[],
	): void => {
		// §10.1: REQUEST_UPDATE consumes a Request ID of its sender.
		options.checkPeerRequestId(message.requestId);
		// §10.2.1, with what §10.2 lets the update of a FETCH carry.
		const outside = parametersOutsideScope(
			message.parameters,
			"REQUEST_UPDATE_OF_FETCH",
		);
		if (outside.length > 0) {
			throw protocolViolation(
				`${outside.join(", ")} not allowed in the REQUEST_UPDATE of a FETCH`,
			);
		}
		if (request.state === "Pending") {
			request.heldUpdates += 1;
			if (request.heldUpdates <= MAX_HELD_FETCH_UPDATES) return;
			// One more than may wait: the FETCH is refused, and none of the
			// updates gets an answer, as the relay does for a subscriber's.
			refuseBeforeAnswer(
				request,
				RequestErrorCode.EXCESSIVE_LOAD,
				"too many REQUEST_UPDATE messages before the answer",
				out,
			);
			return;
		}
		out.push({ type: "send", stream: request.stream, data: REQUEST_OK });
	};

	/** Whether this endpoint opened the request. */
	const ownRequest = (request: RequestEntry): boolean =>
		request.kind === "SUBSCRIBE" ||
		request.kind === "FETCH" ||
		request.kind === "TRACK_STATUS" ||
		request.kind === "PUBLISH_NAMESPACE" ||
		request.kind === "SUBSCRIBE_NAMESPACE" ||
		(request.kind === "PUBLISH" && request.openedBy === "self");
	/** The requests the peer sent a GOAWAY on the stream of (§10.4). */
	const moved = new WeakSet<RequestEntry>();
	/**
	 * A GOAWAY on a request stream (§10.4): "the endpoint SHOULD re-issue
	 * that specific request on a session at the specified URI (or the current
	 * session if no URI is provided), and close the old request stream". The
	 * request is as it was; for a request of this endpoint the application is
	 * told, and sending it again is the application's. One on a request of the
	 * peer says that the peer will send it elsewhere, and nothing is done, also
	 * where this endpoint has ended it.
	 */
	const onRequestGoAway = (
		request: RequestEntry,
		message: GoAway,
		out: SessionOutput[],
	): void => {
		checkRequestGoAway(message, moved.has(request));
		moved.add(request);
		if (!ownRequest(request)) return;
		out.push({
			type: "event",
			event: {
				type: "request-go-away",
				requestId: request.requestId,
				newSessionUri: message.newSessionUri,
				timeout: message.timeout,
			},
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
		if (request.kind === "PEER_FETCH") {
			if (message.type !== "REQUEST_UPDATE") throw unexpected;
			onUpdateOfPeerFetch(request, message, out);
			return;
		}
		// §10.14: TRACK_STATUS is "the first and only message", and "the
		// subscriber cannot send REQUEST_UPDATE". WHY the session is closed
		// for one, as for any message a request of its kind does not take:
		// the draft names no consequence, and an update of a request that
		// creates no state has nothing to apply to. As in the Rust session.
		if (request.kind === "PEER_TRACK_STATUS") throw unexpected;
		if (request.kind === "SUBSCRIBE_NAMESPACE") {
			onNamespaceMessage(request, message, out);
			return;
		}
		// §10.9: "The receiver of a REQUEST_UPDATE MUST respond with exactly
		// one REQUEST_OK or REQUEST_ERROR". Responses carry no Request ID, so
		// they answer the updates in the order sent.
		if (
			request.kind === "SUBSCRIBE" &&
			request.state === "Established" &&
			request.updates > 0 &&
			(message.type === "REQUEST_OK" || message.type === "REQUEST_ERROR")
		) {
			onUpdateAnswered(request, message, out);
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
				if (request.kind === "TRACK_STATUS") {
					onTrackStatusOk(request, message, out);
					return;
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

	const pendingPeerTrackStatus = (requestId: bigint): PeerTrackStatusEntry => {
		const request = requests.get(requestId);
		if (request?.kind !== "PEER_TRACK_STATUS") {
			throw new InactiveError(
				`Request ID ${requestId} is not a TRACK_STATUS of the peer that waits for its answer`,
			);
		}
		return request;
	};

	/**
	 * Sends the one answer of a request of the peer that waits for it, and a
	 * FIN: the request is over. `announce` adds what tells the application.
	 */
	const answerPeerRequest = (
		request: PeerSubscribeEntry | PeerFetchEntry | PeerTrackStatusEntry,
		data: Bytes,
		announce: (out: SessionOutput[]) => void,
	): SessionOutput[] => {
		const out: SessionOutput[] = [];
		// What the peer sent before it saw the answer still arrives, and
		// gets none.
		request.endedLocally = true;
		endRequest(request, out, () => {
			out.push({ type: "send", stream: request.stream, data });
			announce(out);
			finishSend(request, out);
		});
		// The subscriber had finished its direction already; nothing more
		// comes on the stream.
		if (request.peerFinished) {
			requestStreams.delete(request.stream);
			reportPeerEnd(request, "fin", undefined, out);
		}
		return out;
	};

	/** A FETCH of the peer in the state named: waiting for its answer, or being served. */
	const peerFetch = (
		requestId: bigint,
		state: "Pending" | "Established",
	): PeerFetchEntry => {
		const request = requests.get(requestId);
		if (request?.kind !== "PEER_FETCH" || request.state !== state) {
			throw new InactiveError(
				state === "Pending"
					? `Request ID ${requestId} is not a FETCH of the peer that waits for its answer`
					: `Request ID ${requestId} is not a FETCH of the peer that is being served`,
			);
		}
		return request;
	};

	/** Writes to the fetch stream of a FETCH that is being served. */
	const toFetchStream = (
		requestId: bigint,
		encode: (encoder: FetchStreamEncoder) => Bytes,
	): SessionOutput[] => {
		const { writing } = peerFetch(requestId, "Established");
		if (writing === undefined) {
			throw new InactiveError(`the fetch stream of ${requestId} has ended`);
		}
		return [
			{ type: "send", stream: writing.stream, data: encode(writing.encoder) },
		];
	};

	/**
	 * A subscription this endpoint is the publisher of, whose subscriber can
	 * end its direction of the request stream and leave the subscription as
	 * it is: answered or not.
	 */
	const outlivesPeerFin = (request: RequestEntry): boolean =>
		(request.kind === "PUBLISH" && request.state === "Established") ||
		(request.kind === "PEER_SUBSCRIBE" && request.state === "Pending") ||
		// The subscriber of a FETCH has said everything with the FETCH, and
		// that of a TRACK_STATUS with it (§10.14: "the first and only
		// message").
		((request.kind === "PEER_FETCH" || request.kind === "PEER_TRACK_STATUS") &&
			request.state !== "Terminated");

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
			const asked = request.maxObjectPayloadLength;
			if (asked !== undefined && !(Number.isSafeInteger(asked) && asked >= 1)) {
				throw new RangeError(
					`maxObjectPayloadLength is a whole number of bytes from 1, not ${asked}`,
				);
			}
			const connectionLimit = options.limits.maxObjectPayloadLength;
			const maxObjectPayloadLength =
				asked === undefined
					? connectionLimit
					: Math.min(asked, connectionLimit);
			return openRequest((common) => ({
				entry: {
					...common,
					kind: "SUBSCRIBE",
					track: trackKey(request.trackNamespace, request.trackName),
					trackAlias: undefined,
					maxObjectPayloadLength,
					updates: 0,
					updateRefused: false,
				},
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
			/** The track of the SUBSCRIBE a Joining FETCH names. */
			const joinedTrack = (joiningRequestId: bigint): string => {
				const joined = requests.get(joiningRequestId);
				if (joined?.kind !== "SUBSCRIBE" || isDraining(joined)) {
					throw new RangeError(
						`Joining Request ID ${joiningRequestId} is not an open SUBSCRIBE of this session (§10.12.2)`,
					);
				}
				return joined.track;
			};
			let start: Location | undefined;
			let track: string;
			switch (range.fetchType) {
				case "Standalone":
					if (endsBefore(range.startLocation, range.endLocation)) {
						throw new RangeError(
							"the range of the FETCH ends before it starts (§10.12.3)",
						);
					}
					start = range.startLocation;
					track = trackKey(range.trackNamespace, range.trackName);
					break;
				case "RelativeJoining":
					track = joinedTrack(range.joiningRequestId);
					break;
				case "AbsoluteJoining":
					track = joinedTrack(range.joiningRequestId);
					// §10.12.2.1: "the publisher sets the Start Location to
					// {Joining Start, 0}".
					start = { group: range.joiningStart, object: 0n };
					break;
			}
			return openRequest((common) => ({
				entry: { ...common, kind: "FETCH", track, start, fetchStream: "none" },
				message: {
					type: "FETCH",
					requestId: common.requestId,
					range,
					parameters: request.parameters ?? {},
				},
			}));
		},

		trackStatus(request) {
			return openRequest((common) => ({
				entry: { ...common, kind: "TRACK_STATUS" },
				message: {
					type: "TRACK_STATUS",
					requestId: common.requestId,
					trackNamespace: request.trackNamespace,
					trackName: request.trackName,
					parameters: request.parameters ?? {},
				},
			}));
		},

		subscribeNamespace(request) {
			// The prefix and the parameters are checked by the encoding.
			return openRequest((common) => ({
				entry: {
					...common,
					kind: "SUBSCRIBE_NAMESPACE",
					prefix: request.trackNamespacePrefix,
					named: new Set(),
				},
				message: {
					type: "SUBSCRIBE_NAMESPACE",
					requestId: common.requestId,
					trackNamespacePrefix: request.trackNamespacePrefix,
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

		updateSubscription(requestId, parameters) {
			const request = requests.get(requestId);
			if (
				request === undefined ||
				request.kind !== "SUBSCRIBE" ||
				request.state !== "Established" ||
				request.endedLocally ||
				// §5.1: PUBLISH_DONE terminated the subscription; the entry is
				// kept for the streams that are still counted, and is no
				// subscription to update. The Rust session has dropped the
				// request by then.
				isDraining(request)
			) {
				throw new RangeError(
					`request ${requestId} is not an Established SUBSCRIBE of this endpoint`,
				);
			}
			if (request.updates >= MAX_UNANSWERED_UPDATES) {
				throw new RangeError(
					`${MAX_UNANSWERED_UPDATES} updates of request ${requestId} wait for their answer`,
				);
			}
			// Throws for a parameter that is not allowed, before anything is
			// taken. §10.1: a REQUEST_UPDATE consumes a Request ID.
			const data = encodeControlMessage({
				type: "REQUEST_UPDATE",
				requestId: nextRequestId,
				parameters,
			});
			nextRequestId += 2n;
			request.updates += 1;
			return [{ type: "send", stream: request.stream, data }];
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
			if (peerRequestsAreFull()) {
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

		peerFetchOpened(message, peerStream, out) {
			const { range } = message;
			if (range.fetchType !== "Standalone") {
				throw new Error("only a Standalone FETCH is handed on");
			}
			if (
				peerRequestsAreFull() ||
				openPeerFetches() >= options.limits.maxPeerFetches
			) {
				return { refused: RequestErrorCode.EXCESSIVE_LOAD };
			}
			const entry: PeerFetchEntry = {
				requestId: message.requestId,
				stream: options.allocateStream(),
				state: "Pending",
				inbound: new Uint8Array(0),
				sendClosed: false,
				endedLocally: false,
				peerFinished: false,
				kind: "PEER_FETCH",
				start: range.startLocation,
				// §10.2.8: Ascending where the FETCH does not say.
				groupOrder: message.parameters.groupOrder ?? "Ascending",
				heldUpdates: 0,
				writing: undefined,
				served: false,
			};
			requests.set(entry.requestId, entry);
			requestStreams.set(entry.stream, entry);
			out.push({ type: "adopt-peer-stream", peerStream, stream: entry.stream });
			out.push({
				type: "event",
				event: {
					type: "peer-fetch",
					requestId: message.requestId,
					trackNamespace: range.trackNamespace,
					trackName: range.trackName,
					startLocation: range.startLocation,
					endLocation: range.endLocation,
					parameters: message.parameters,
				},
			});
			return { stream: entry.stream };
		},

		peerTrackStatusOpened(message, peerStream, out) {
			if (peerRequestsAreFull()) {
				return { refused: RequestErrorCode.EXCESSIVE_LOAD };
			}
			const entry: PeerTrackStatusEntry = {
				requestId: message.requestId,
				stream: options.allocateStream(),
				state: "Pending",
				inbound: new Uint8Array(0),
				sendClosed: false,
				endedLocally: false,
				peerFinished: false,
				kind: "PEER_TRACK_STATUS",
			};
			requests.set(entry.requestId, entry);
			requestStreams.set(entry.stream, entry);
			out.push({ type: "adopt-peer-stream", peerStream, stream: entry.stream });
			out.push({
				type: "event",
				event: {
					type: "peer-track-status",
					requestId: message.requestId,
					trackNamespace: message.trackNamespace,
					trackName: message.trackName,
					parameters: message.parameters,
				},
			});
			return { stream: entry.stream };
		},

		acceptTrackStatus(requestId, answer) {
			const request = pendingPeerTrackStatus(requestId);
			const parameters = answer.parameters ?? {};
			// The codec takes in a REQUEST_OK what any of its kinds may carry.
			const outside = parametersOutsideScope(parameters, "TRACK_STATUS_OK");
			if (outside.length > 0) {
				throw new RangeError(
					`${outside.join(", ")} not allowed in TRACK_STATUS_OK (§10.2.11)`,
				);
			}
			// Before anything changes: a value that does not fit its field
			// throws here.
			const data = encodeControlMessage({
				type: "REQUEST_OK",
				parameters,
				trackProperties: answer.trackProperties ?? [],
			});
			return answerPeerRequest(request, data, () => {});
		},

		acceptFetch(requestId, answer) {
			const request = peerFetch(requestId, "Pending");
			if (endsBefore(request.start, answer.endLocation)) {
				throw new RangeError(
					"the FETCH_OK ends before the FETCH starts (§10.13)",
				);
			}
			// Before anything changes: a value that does not fit its field
			// throws here.
			const ok = encodeControlMessage({
				type: "FETCH_OK",
				endOfTrack: answer.endOfTrack ?? false,
				endLocation: answer.endLocation,
				parameters: {},
				trackProperties: answer.trackProperties ?? [],
			});
			const encoder = createFetchStreamEncoder(
				{ requestId },
				request.groupOrder,
			);
			const stream = options.allocateStream();
			request.state = "Established";
			request.writing = { stream, encoder };
			fetchStreams.set(stream, request);
			const out: SessionOutput[] = [
				{ type: "send", stream: request.stream, data: ok },
			];
			for (; request.heldUpdates > 0; request.heldUpdates -= 1) {
				out.push({ type: "send", stream: request.stream, data: REQUEST_OK });
			}
			// WHY at once, where §10.13 lets the stream come before or after
			// the FETCH_OK: a relay that has the FETCH_OK waits a limited time
			// for the stream to open, and the application then has nothing to
			// do in time.
			out.push({
				type: "open-uni-stream",
				stream,
				data: encoder.header,
				sendOrder: DATA_STREAM_SEND_ORDER,
			});
			return out;
		},

		writeFetchObject: (requestId, object) =>
			toFetchStream(requestId, (encoder) => encoder.object(object)),

		writeFetchRangeEnd: (requestId, kind, location) =>
			toFetchStream(requestId, (encoder) => encoder.endOfRange(kind, location)),

		finishFetch(requestId) {
			const request = peerFetch(requestId, "Established");
			const out: SessionOutput[] = [];
			if (request.writing !== undefined) {
				const { stream } = request.writing;
				// Before `endRequest`, which resets a fetch stream that is open.
				request.writing = undefined;
				fetchStreams.delete(stream);
				out.push({ type: "finish", stream });
			}
			request.endedLocally = true;
			request.served = true;
			// WHY this endpoint's direction of the request stream stays open
			// while the subscriber's is: the two streams of a FETCH arrive
			// apart, and a subscriber can take a FIN of the request stream
			// that it sees before the end of the fetch stream for a FETCH
			// that failed (moqtransport 0.14.0 does, about one time in eight
			// on loopback). That is a reading of §11.4.1, "Termination of a
			// bidi request stream terminates the ... Fetch ... request", with a
			// FIN counted as a termination; this endpoint reads it so for a
			// peer's FIN on other requests as well. §5.2 has the data stream's
			// FIN end the FETCH and asks nothing of the request stream; a
			// subscriber ends it when it has what it asked for.
			// WHY NOT finish it at once, which frees the stream sooner and
			// which §5.2 does not forbid: a subscriber of that library would
			// lose FETCHes that were answered whole, and nothing is gained
			// for a subscriber that ends its own direction, as both own ones
			// do. The cost is a subscriber that never ends it: the stream is
			// held for the caller's two waits, and not for one.
			endRequest(request, out, () => {
				if (request.peerFinished) finishSend(request, out);
			});
			// The subscriber had finished its direction already; nothing more
			// comes on the stream.
			if (request.peerFinished) {
				requestStreams.delete(request.stream);
				reportPeerEnd(request, "fin", undefined, out);
			}
			return out;
		},

		resetFetch(requestId, code) {
			const request = peerFetch(requestId, "Established");
			const out: SessionOutput[] = [];
			endRequest(
				request,
				out,
				() => resetRequestStream(request, code, out),
				code,
			);
			return out;
		},

		holdsPeerRequestId(requestId) {
			for (const request of requestStreams.values()) {
				if (request.requestId === requestId && openedByThePeer(request)) {
					return true;
				}
			}
			return false;
		},

		heldPeerRequests,

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
			const waiting = requests.get(requestId);
			const request =
				waiting?.kind === "PEER_FETCH"
					? peerFetch(requestId, "Pending")
					: waiting?.kind === "PEER_TRACK_STATUS"
						? pendingPeerTrackStatus(requestId)
						: pendingPeerSubscribe(requestId);
			const data = encodeControlMessage({
				type: "REQUEST_ERROR",
				errorCode,
				retryInterval: madeUpRetryInterval(errorCode),
				errorReason,
			});
			const messageType = PEER_REQUEST_MESSAGE_TYPES[request.kind];
			return answerPeerRequest(request, data, (out) => {
				out.push({
					type: "event",
					event: {
						type: "peer-request-rejected",
						messageType,
						requestId,
						errorCode,
					},
				});
			});
		},

		// §5.1: "The publisher terminates a subscription in the Pending
		// (Publisher) or Established states by sending PUBLISH_DONE".
		publishDone(requestId, statusCode, errorReason) {
			const request = activePublish(requestId);
			if (request.endsWithUpdateFailed) {
				throw new InactiveError(
					"a refused REQUEST_UPDATE ends the subscription with UPDATE_FAILED (§10.9.1)",
				);
			}
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
				reportPeerEnd(request, "fin", undefined, out);
			}
			return out;
		},

		peerEndExpired(requestId, out) {
			for (const [stream, request] of requestStreams) {
				if (request.requestId !== requestId || !awaitsPeerEnd(request)) {
					continue;
				}
				// A FETCH that was served may still have this endpoint's
				// direction open, where the caller did not end its first wait
				// with `finishServedFetchSend`: the stream is not left half open.
				finishServedSend(request, out);
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

		finishServedFetchSend(requestId, out) {
			for (const request of requestStreams.values()) {
				if (request.requestId === requestId) finishServedSend(request, out);
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
			if (request.endsWithUpdateFailed) {
				throw new InactiveError(
					"the subscription ends: a REQUEST_UPDATE of it was refused (§10.9.1)",
				);
			}
			return outgoing.open(requestId, request.trackAlias, subgroupOptions);
		},

		subscriptionExpired: endAfterPublishDone,

		updateRefusalExpired(requestId, out) {
			const request = requests.get(requestId);
			// Its streams were closed in time, and it has ended.
			if (request?.kind !== "PUBLISH" || !request.endsWithUpdateFailed) return;
			// The streams that are left are reset (`endRequest`), and then
			// PUBLISH_DONE follows (§10.11).
			endWithUpdateFailed(request, out);
		},

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
						// Held to its rules on a request of the peer, as the Rust
						// session does; on one of this endpoint's, as the Rust
						// session does too, it crossed the end and is not read.
						if (message.type === "GOAWAY" && !ownRequest(request)) {
							onRequestGoAway(request, message, out);
						}
						return;
					}
					if (request.state === "Terminated") {
						throw protocolViolation(
							`${message.type} on a ${request.kind} request stream that had ended`,
						);
					}
					if (message.type === "GOAWAY") {
						onRequestGoAway(request, message, out);
						return;
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
				finishServedSend(request, out);
				reportPeerEnd(request, "fin", undefined, out);
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
				finishServedSend(request, out);
				reportPeerEnd(request, "reset", code, out);
				return;
			}
			abandonRequest(request, "peer-reset", out, code);
		},

		streamStopped(stream, code, out) {
			// WHY a stop of the fetch stream alone ends the FETCH, where the
			// draft has the subscriber stop the request stream (§5.2: "It MUST
			// send STOP_SENDING for the bidi request stream", and MAY stop the
			// data stream as well): a FETCH cannot go on without its stream.
			// The own relay reads a subscriber in the same way.
			const fetched = fetchStreams.get(stream);
			if (fetched !== undefined) {
				abandonRequest(fetched, "peer-reset", out, code);
				return;
			}
			const request = requestStreams.get(stream);
			if (request === undefined) return;
			// A FETCH that was served, whose subscriber stops this endpoint's
			// direction while it is still open for the subscriber to end its
			// own first (`finishFetch`). §5.2: "The Publisher can destroy fetch
			// state as soon as it has received a STOP_SENDING. It MUST reset
			// the bidi request stream". The subscriber wants nothing more on
			// the stream, so its own direction is stopped as well and the
			// stream forgotten, as the relay does: nothing waits any longer.
			// WHY NOT only mark the direction as closed, and wait for the
			// subscriber's: the stream and its place among the requests of
			// the peer would be held for the whole of both waits.
			if (
				request.kind === "PEER_FETCH" &&
				request.served &&
				!request.sendClosed
			) {
				resetRequestStream(request, StreamResetErrorCode.CANCELLED, out);
				out.push({
					type: "stop-peer-direction",
					stream,
					code: StreamResetErrorCode.CANCELLED,
				});
				reportPeerEnd(request, "reset", code, out);
				return;
			}
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
			fetchStreams.clear();
			outgoing.clear();
			incoming.clear();
		},
	};
}
