/**
 * Session termination error codes (draft-ietf-moq-transport-18 §3.5). Only the
 * codes this implementation raises are listed.
 */
export const SessionErrorCode = {
	NO_ERROR: 0x0,
	INTERNAL_ERROR: 0x1,
	PROTOCOL_VIOLATION: 0x3,
	INVALID_REQUEST_ID: 0x4,
	DUPLICATE_TRACK_ALIAS: 0x5,
	INVALID_PATH: 0x8,
	CONTROL_MESSAGE_TIMEOUT: 0x11,
	INVALID_AUTHORITY: 0x19,
} satisfies Record<string, number>;

/** Stream reset and STOP_SENDING error codes (§3.3.3) this implementation sends. */
export const StreamResetErrorCode = {
	/** A local limit was exceeded. */
	INTERNAL_ERROR: 0x0,
	/** A subscription ended or was cancelled. */
	CANCELLED: 0x1,
	/**
	 * A malformed Track was detected (§2.4.2). §3.3.3 words this code for a
	 * relay publisher; it is used here too because it tells the sender why.
	 */
	MALFORMED_TRACK: 0x12,
} satisfies Record<string, number>;

/** REQUEST_ERROR codes (§15.10.2): the ones this implementation sends, and one an application refuses a request with. */
export const RequestErrorCode = {
	/** An error of this endpoint: a handler of the application failed. */
	INTERNAL_ERROR: 0x0n,
	/**
	 * The request was not answered in time (§10.6). Not sent by the
	 * library; what a relay answers a SUBSCRIBE with that it held for a
	 * publisher that did not come (RENDEZVOUS_TIMEOUT, §10.2.6).
	 */
	TIMEOUT: 0x2n,
	NOT_SUPPORTED: 0x3n,
	/**
	 * The peer sent GOAWAY, or received one, and takes no new request
	 * (§10.4, §10.6.2). Not sent by the library.
	 */
	GOING_AWAY: 0x6n,
	/** Too many requests of the peer are open (§10.6.2). */
	EXCESSIVE_LOAD: 0x9n,
	/** The Track is not one this endpoint has (§10.6.2). Not sent by the library; for an application that refuses a request. */
	DOES_NOT_EXIST: 0x10n,
	/** The request would make a second subscription to a Track in one role (§5.1). */
	DUPLICATE_SUBSCRIPTION: 0x19n,
	/** The range of a FETCH is not valid (§10.6.2). */
	INVALID_RANGE: 0x11n,
	/** The Joining Request ID of a Joining FETCH names no subscription of the session (§10.12.2). */
	INVALID_JOINING_REQUEST_ID: 0x32n,
} satisfies Record<string, bigint>;

/** What an endpoint that refuses for its own load gives as the wait before a retry. **Provisional.** */
const RETRY_WHEN_LOADED_MS = 1000n;

/** The longest wait a caller is handed, in milliseconds: the longest delay a timer of a browser takes. */
export const LONGEST_RETRY_WAIT_MS = 2_147_483_647;

/**
 * The Retry Interval of a REQUEST_ERROR this endpoint makes up itself, by
 * its Error Code (`fixtures/retry-interval/made-up.tsv`, which the Rust
 * implementation reads too). §10.6.2: "The minimum time (in milliseconds)
 * before the request SHOULD be sent again, plus one. If the value is 0, the
 * request SHOULD NOT be retried."
 *
 * WHY EXCESSIVE_LOAD alone: "The sender SHOULD use the Retry Interval to
 * indicate when the request can be retried" is said of that code and of no
 * other, and a limit of this endpoint is one that has room again later.
 * For the others this endpoint does not know whether the same request can
 * succeed later.
 */
export function madeUpRetryInterval(errorCode: bigint): bigint {
	return errorCode === RequestErrorCode.EXCESSIVE_LOAD
		? RETRY_WHEN_LOADED_MS + 1n
		: 0n;
}

/**
 * After how many milliseconds a request that got this Retry Interval may be
 * sent again; `undefined` where it "SHOULD NOT be retried" (§10.6.2).
 */
export function retryAfterMs(retryInterval: bigint): bigint | undefined {
	return retryInterval === 0n ? undefined : retryInterval - 1n;
}

/** The same as a wait a timer takes: no more than `LONGEST_RETRY_WAIT_MS`. */
export function retryWaitMs(retryInterval: bigint): number | undefined {
	const after = retryAfterMs(retryInterval);
	if (after === undefined) return undefined;
	return after > BigInt(LONGEST_RETRY_WAIT_MS)
		? LONGEST_RETRY_WAIT_MS
		: Number(after);
}

/**
 * An error that ends the session. `code` is the Session termination error
 * code to close the session with.
 */
export class MoqtError extends Error {
	override readonly name: string = "MoqtError";
	readonly code: number;

	constructor(code: number, message: string) {
		super(message);
		this.code = code;
	}
}

/**
 * Input that draft 18 defines but this implementation does not handle yet.
 *
 * WHY a separate class from a PROTOCOL_VIOLATION: the peer did nothing wrong,
 * so the session is closed with INTERNAL_ERROR and callers can tell the two
 * apart.
 */
export class NotImplementedError extends MoqtError {
	override readonly name = "NotImplementedError";

	constructor(what: string) {
		super(SessionErrorCode.INTERNAL_ERROR, `not implemented: ${what}`);
	}
}

export function protocolViolation(message: string): MoqtError {
	return new MoqtError(SessionErrorCode.PROTOCOL_VIOLATION, message);
}

/**
 * The application used something that is no longer there: a subgroup stream
 * that is closed, or a subscription that has ended. The caller knows whether
 * that is the end of a Track or a mistake.
 */
export class InactiveError extends Error {
	override readonly name = "InactiveError";
}
