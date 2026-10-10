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

/** REQUEST_ERROR codes (§15.10.2) this implementation sends. */
export const RequestErrorCode = {
	NOT_SUPPORTED: 0x3n,
} satisfies Record<string, bigint>;

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
