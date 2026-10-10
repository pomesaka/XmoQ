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
 * | `SubgroupClosedError` | The subgroup stream is closed while its Track goes on | Open another subgroup |
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
	/** §2.5.1: the SUBSCRIBE_OK had a Mandatory Track Property this endpoint does not understand, so it cancelled. */
	| { type: "unsupported-track-property" };

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
}
