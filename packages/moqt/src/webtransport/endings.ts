import type { Redirect } from "../core/control-message.ts";

/** How the session ended. */
export type SessionClose =
	/** This endpoint called `close()`. */
	| { type: "local"; code: number; reason: string }
	/** The peer closed the transport session. `code` is its Session termination error code (§3.5). */
	| { type: "peer"; code: number | undefined; reason: string | undefined }
	/**
	 * The session could not go on: the peer broke a rule of the draft (a
	 * `MoqtError` with the code the session was closed with), the transport
	 * failed, or this endpoint failed.
	 */
	| { type: "error"; error: Error };

/** How a subscription made with SUBSCRIBE ended. */
export type SubscriptionEnd =
	/** The publisher ended it with PUBLISH_DONE (§10.11); `statusCode` says why. */
	| { type: "publish-done"; statusCode: bigint; errorReason: string }
	/** The publisher closed the request stream without PUBLISH_DONE. */
	| { type: "peer-closed" }
	/** The publisher reset the request stream. `code` is a Stream Reset Error Code (§3.3.3). */
	| { type: "peer-reset"; code: number | undefined }
	/** This endpoint cancelled it. */
	| { type: "cancelled" }
	/** §2.4.2: the Track was malformed, so this endpoint cancelled it. */
	| { type: "malformed-track" }
	| { type: "session-closed"; close: SessionClose };

/** How a subscription made with PUBLISH ended. */
export type PublishEnd =
	/** This endpoint sent PUBLISH_DONE. */
	| { type: "done" }
	/** The subscriber closed the request stream. */
	| { type: "peer-closed" }
	/** The subscriber cancelled: it reset the request stream or sent STOP_SENDING for it. `code` is a Stream Reset Error Code (§3.3.3). */
	| { type: "peer-reset"; code: number | undefined }
	/** The subscriber sent a REQUEST_UPDATE, which is not implemented and ends the subscription (§10.9.1). */
	| { type: "update-failed" }
	| { type: "session-closed"; close: SessionClose };

/**
 * How a FETCH ended. The Objects are in `Fetch.body`, which says by itself
 * whether all of them came.
 */
export type FetchEnd =
	/** FETCH_OK came, and the publisher finished the fetch stream: the response is whole. */
	| { type: "served" }
	/** FETCH_OK came, and the publisher reset the fetch stream. `code` is a Stream Reset Error Code (§3.3.3). */
	| { type: "stream-reset"; code: number | undefined }
	/** The publisher answered REQUEST_ERROR; `error` has what it said. */
	| { type: "rejected"; error: RequestFailedError }
	/** The publisher closed the request stream without an answer. */
	| { type: "peer-closed" }
	/** The publisher reset the request stream (§5.2). `code` is a Stream Reset Error Code (§3.3.3). */
	| { type: "peer-reset"; code: number | undefined }
	/** §2.5.1: the FETCH_OK had a Mandatory Track Property this endpoint does not understand, so it cancelled. */
	| { type: "unsupported-track-property" }
	/** This endpoint cancelled it: `cancel()`, the `signal` of the call, or a reader that cancelled `body`. */
	| { type: "cancelled" }
	| { type: "session-closed"; close: SessionClose };

/**
 * How the subscriber ended its direction of the request stream after this
 * endpoint's PUBLISH_DONE. These say what was seen and no more: the draft
 * gives that direction no meaning after PUBLISH_DONE (§10.11, §5.1.1).
 */
export type PeerStreamEnd =
	/**
	 * The subscriber finished its direction. This does not say that
	 * everything arrived. It can be read so only when the peer is an XmoQ
	 * relay, which finishes once as many subgroup streams as the Stream
	 * Count named have ended there. A peer that knows nothing of that
	 * finishes for reasons of its own. A subscriber that had finished its
	 * direction before PUBLISH_DONE is reported in the same way, and then
	 * the FIN says nothing about the streams, whatever the peer is.
	 */
	| { type: "fin" }
	/**
	 * The subscriber reset its direction. An XmoQ relay does so (CANCELLED)
	 * when its wait for the counted streams ran out. `code` is a Stream Reset
	 * Error Code (§3.3.3).
	 */
	| { type: "reset"; code: number | undefined }
	/**
	 * This endpoint saw neither before its wait was over, and stopped the
	 * subscriber's direction (CANCELLED). That is not to say the subscriber
	 * did not end it: a FIN that arrives as the wait ends is not seen.
	 */
	| { type: "nothing-in-time" }
	/** The session ended first. */
	| { type: "session-closed"; close: SessionClose };

function describeClose(close: SessionClose): string {
	switch (close.type) {
		case "local":
			return `the session was closed locally: ${close.reason}`;
		case "peer":
			return `the peer closed the session: code=${close.code ?? "unknown"} reason=${close.reason ?? ""}`;
		case "error":
			return `the session failed: ${close.error.message}`;
	}
}

/**
 * Why a call on a connection failed. An application tells the cases apart by
 * the class, never by the message:
 *
 * | Class | Meaning | What to do |
 * |---|---|---|
 * | `RequestFailedError` | The peer answered REQUEST_ERROR | Retry after `retryInterval`, follow `redirect`, or give up |
 * | `RequestAbortedError` | The request ended without an answer | The request can be made again |
 * | `SessionClosedError` | The session is over | Connect again |
 * | `SubscriptionEndedError` | The Objects of a subscription stopped before its publisher finished it | See `end` |
 * | `PublishEndedError` | The subscription made with PUBLISH is over | Stop publishing; see `end` |
 * | `SubgroupClosedError` | The subgroup stream is closed while its Track goes on; `metStop` says whether this call met the subscriber's STOP_SENDING, and `code` has its code where the browser passed it on | Open another subgroup |
 * | `FetchEndedError` | The FETCH ended before its fetch stream did | See `end` |
 * | `FetchStreamResetError` | The publisher reset the fetch stream | What was read before it is good; see `code` |
 */
export class RequestFailedError extends Error {
	override readonly name = "RequestFailedError";
	/** A REQUEST_ERROR code (§15.10.2). */
	readonly errorCode: bigint;
	/** Milliseconds plus one before the request may be sent again; 0 means do not retry (§10.6.2). */
	readonly retryInterval: bigint;
	readonly errorReason: string;
	readonly redirect: Redirect | undefined;

	constructor(fields: {
		errorCode: bigint;
		retryInterval: bigint;
		errorReason: string;
		redirect?: Redirect;
	}) {
		super(
			`REQUEST_ERROR 0x${fields.errorCode.toString(16)}: ${fields.errorReason}`,
		);
		this.errorCode = fields.errorCode;
		this.retryInterval = fields.retryInterval;
		this.errorReason = fields.errorReason;
		this.redirect = fields.redirect;
	}
}

/** Why a request ended before it was answered. */
export type RequestAbort =
	/** The peer closed the request stream. */
	| { type: "peer-closed" }
	/** The peer reset the request stream. `code` is a Stream Reset Error Code (§3.3.3). */
	| { type: "peer-reset"; code: number | undefined }
	/** §2.5.1: the SUBSCRIBE_OK or FETCH_OK had a Mandatory Track Property this endpoint does not understand, so it cancelled. */
	| { type: "unsupported-track-property" }
	/** This endpoint cancelled the request before it was answered. Only a FETCH is handed out before its answer. */
	| { type: "cancelled" };

export class RequestAbortedError extends Error {
	override readonly name = "RequestAbortedError";
	readonly abort: RequestAbort;

	constructor(abort: RequestAbort) {
		super(`the request ended without a response: ${abort.type}`);
		this.abort = abort;
	}
}

export class SessionClosedError extends Error {
	override readonly name = "SessionClosedError";
	/**
	 * How the session ended, as `MoqtConnection.closed` has it. One exception:
	 * a write that the browser failed for the end of the session, when the
	 * browser had not reported that end a moment later, has `type` "error"
	 * with the browser's error here, and `closed` has the rest when it comes.
	 */
	readonly close: SessionClose;

	constructor(close: SessionClose) {
		super(describeClose(close));
		this.close = close;
	}
}

/** `Subscription.objects` fails with this; `end` is what `Subscription.ended` resolves with. */
export class SubscriptionEndedError extends Error {
	override readonly name = "SubscriptionEndedError";
	readonly end: SubscriptionEnd;

	constructor(end: SubscriptionEnd) {
		super(
			end.type === "session-closed"
				? describeClose(end.close)
				: `the subscription ended: ${end.type}`,
		);
		this.end = end;
	}
}

export class PublishEndedError extends Error {
	override readonly name = "PublishEndedError";
	readonly end: PublishEnd;

	constructor(end: PublishEnd) {
		super(
			end.type === "session-closed"
				? describeClose(end.close)
				: `the subscription made with PUBLISH ended: ${end.type}`,
		);
		this.end = end;
	}
}

export class SubgroupClosedError extends Error {
	override readonly name = "SubgroupClosedError";
	/**
	 * Whether this call is the one that met the subscriber's STOP_SENDING:
	 * its write was under way, or was made, when the stop came. False for
	 * every call after that one, and for a stream this endpoint closed or
	 * reset itself. So it does not say whether the stream was ever stopped.
	 */
	readonly metStop: boolean;
	/**
	 * With `metStop`: the Stream Reset Error Code (§3.3.3) of the
	 * STOP_SENDING. `undefined` there means that the subscriber stopped the
	 * stream and the browser did not pass the code on, which Chromium does
	 * when the stop arrives while a write is under way.
	 */
	readonly code: number | undefined;

	constructor(message: string, stop?: { code: number | undefined }) {
		super(message);
		this.metStop = stop !== undefined;
		this.code = stop?.code;
	}
}

/**
 * `Fetch.body` fails with this when the FETCH ended while its fetch stream
 * had not; `end` is what `Fetch.ended` resolves with.
 */
export class FetchEndedError extends Error {
	override readonly name = "FetchEndedError";
	readonly end: FetchEnd;

	constructor(end: FetchEnd) {
		super(
			end.type === "session-closed"
				? describeClose(end.close)
				: `the FETCH ended: ${end.type}`,
		);
		this.end = end;
	}
}

/** `Fetch.body` fails with this when the publisher reset the fetch stream. */
export class FetchStreamResetError extends Error {
	override readonly name = "FetchStreamResetError";
	/** A Stream Reset Error Code (§3.3.3), when the transport reported one. */
	readonly code: number | undefined;

	constructor(code: number | undefined) {
		super(`the publisher reset the fetch stream: code=${code ?? "unknown"}`);
		this.code = code;
	}
}
