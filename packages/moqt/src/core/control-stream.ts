import { drainControlMessages } from "./control-message.ts";
import { MoqtError, protocolViolation, SessionErrorCode } from "./error.ts";
import type { SessionOutput } from "./session-output.ts";

/** Setup Option types of §10.3.1. A client must not receive the first two. */
const SETUP_OPTION_PATH = 0x01n;
const SETUP_OPTION_AUTHORITY = 0x05n;
export const SETUP_OPTION_MOQT_IMPLEMENTATION = 0x07n;

export interface PeerControlStream {
	/** Bytes of the peer's control stream, starting with its SETUP. Throws the `MoqtError` that ends the session. */
	receive(data: Uint8Array, fin: boolean, out: SessionOutput[]): void;
}

export function createPeerControlStream(): PeerControlStream {
	let buffer: Uint8Array = new Uint8Array(0);
	let setupReceived = false;

	return {
		receive(data, fin, out) {
			buffer = drainControlMessages(buffer, data, (message) => {
				// §10.3: SETUP is the first message on the control stream. The only
				// other message draft 18 sends there is GOAWAY, which the decoder
				// reports as not implemented.
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
