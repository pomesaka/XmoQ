import { drainControlMessages, type GoAway } from "./control-message.ts";
import { MoqtError, protocolViolation, SessionErrorCode } from "./error.ts";
import type { SessionOutput } from "./session-output.ts";

/** Setup Option types of §10.3.1. A client must not receive the first two. */
const SETUP_OPTION_PATH = 0x01n;
const SETUP_OPTION_AUTHORITY = 0x05n;
/** §10.3.1.4: "The AUTHORIZATION TOKEN Setup Option (Option Type 0x03)". */
export const SETUP_OPTION_AUTHORIZATION_TOKEN = 0x03n;
export const SETUP_OPTION_MOQT_IMPLEMENTATION = 0x07n;

export interface PeerControlStream {
	/** Bytes of the peer's control stream, starting with its SETUP. Throws the `MoqtError` that ends the session. */
	receive(data: Uint8Array, fin: boolean, out: SessionOutput[]): void;
	/** Whether the peer sent GOAWAY on it. */
	readonly peerGoingAway: boolean;
}

/**
 * The rules §10.4 sets for a GOAWAY on the control stream, as the receiver
 * of this package, a client, has them. A client never gets the one for a
 * server ("If a server receives a GOAWAY with a non-zero New Session URI
 * Length it MUST close the session with a PROTOCOL_VIOLATION"). The Rust
 * implementation checks the same.
 */
function checkGoAway(message: GoAway, alreadyGoingAway: boolean): void {
	// "The endpoint MUST close the session with a PROTOCOL_VIOLATION if it
	// receives more than one GOAWAY on the control stream".
	if (alreadyGoingAway) throw protocolViolation("a second GOAWAY");
	// "Request ID: Present only when sent on the control stream." WHY a
	// violation where it is missing: without it the receiver cannot tell
	// which of its requests the peer took.
	if (message.requestId === undefined) {
		throw protocolViolation("a GOAWAY without a Request ID");
	}
	// "If the parity of the Request ID does not match the receiver's parity,
	// the endpoint MUST close the session with INVALID_REQUEST_ID." A client's
	// Request IDs are even (§10.1).
	if (message.requestId % 2n !== 0n) {
		throw new MoqtError(
			SessionErrorCode.INVALID_REQUEST_ID,
			`a GOAWAY with the Request ID ${message.requestId} of a server`,
		);
	}
}

export function createPeerControlStream(): PeerControlStream {
	let buffer: Uint8Array = new Uint8Array(0);
	let setupReceived = false;
	let peerGoingAway = false;

	return {
		get peerGoingAway() {
			return peerGoingAway;
		},
		receive(data, fin, out) {
			buffer = drainControlMessages(buffer, data, (message) => {
				// §10.3: SETUP is the first message on the control stream. The only
				// other message draft 18 sends there is GOAWAY.
				if (message.type === "GOAWAY" && setupReceived) {
					checkGoAway(message, peerGoingAway);
					peerGoingAway = true;
					out.push({
						type: "event",
						event: {
							type: "go-away",
							newSessionUri: message.newSessionUri,
							timeout: message.timeout,
						},
					});
					return;
				}
				if (message.type !== "SETUP" || setupReceived) {
					throw protocolViolation(
						`${message.type} is not allowed here on the control stream`,
					);
				}
				setupReceived = true;
				for (const option of message.setupOptions) {
					// §10.3.1.1 and §10.3.1.2: neither option may come from a server
					// or over WebTransport.
					if (option.type === SETUP_OPTION_PATH) {
						throw new MoqtError(
							SessionErrorCode.INVALID_PATH,
							"the server sent a PATH option",
						);
					}
					if (option.type === SETUP_OPTION_AUTHORITY) {
						throw new MoqtError(
							SessionErrorCode.INVALID_AUTHORITY,
							"the server sent an AUTHORITY option",
						);
					}
				}
				out.push({
					type: "event",
					event: { type: "setup", setupOptions: message.setupOptions },
				});
			});
			// §3.3: a control stream must not be closed during the session.
			if (fin) throw protocolViolation("the peer closed its control stream");
		},
	};
}
