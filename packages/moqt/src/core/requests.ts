import {
	type ControlMessage,
	drainControlMessages,
	encodeControlMessage,
	type PublishDone,
	type RequestError,
	type RequestOk,
	type RequestUpdate,
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
	type MessageParameters,
	parametersOutsideScope,
} from "./parameters.ts";
import type {
	RequestClosedReason,
	SessionOutput,
	SessionResult,
} from "./session-output.ts";
import type { FullTrackName, TrackNamespace } from "./track.ts";

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
	cancelRequest(requestId: bigint): SessionOutput[];
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
}

interface NamespaceEntry extends RequestCommon {
	kind: "PUBLISH_NAMESPACE";
}

interface PublishEntry extends RequestCommon {
	kind: "PUBLISH";
	trackAlias: bigint;
	/** The Forward State of §5.1. */
	forward: boolean;
}

interface SubscribeEntry extends RequestCommon {
	kind: "SUBSCRIBE";
	/** Set by SUBSCRIBE_OK. */
	trackAlias: bigint | undefined;
}

type RequestEntry = NamespaceEntry | PublishEntry | SubscribeEntry;

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
		switch (message.type) {
			case "REQUEST_ERROR":
				if (request.state !== "Pending") throw unexpected;
				onRequestError(request, message, out);
				return;
			case "REQUEST_OK":
				if (request.state !== "Pending" || request.kind === "SUBSCRIBE") {
					throw unexpected;
				}
				onRequestOk(request, message, out);
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
				// §10.9: on a stream this endpoint opened, only the subscriber of an
				// Established PUBLISH may send it.
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
			const parameters = request.parameters ?? {};
			const result = openRequest((common) => ({
				entry: {
					...common,
					kind: "PUBLISH",
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

		cancelRequest(requestId) {
			const request = requests.get(requestId);
			if (request === undefined) return [];
			const out: SessionOutput[] = [];
			endRequest(request, out, () =>
				resetRequestStream(request, StreamResetErrorCode.CANCELLED, out),
			);
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
			return out;
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
			requestStreams.delete(stream);
			if (request.state === "Terminated") return;
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
			if (request.kind === "PUBLISH" && request.state === "Established") {
				abandonRequest(request, "peer-reset", out, code);
			}
		},

		clear() {
			requests.clear();
			requestStreams.clear();
			publishedAliases.clear();
			outgoing.clear();
			incoming.clear();
		},
	};
}
