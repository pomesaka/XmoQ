import { SessionErrorCode } from "../core/error.ts";
import {
	connectOver,
	type MoqtConnection,
	type SessionConnectOptions,
} from "./connection.ts";
import type { MoqtTransport } from "./transport.ts";

/** WebTransport subprotocol of draft-ietf-moq-transport-18 (§3.1). */
const MOQT_PROTOCOL = "moqt-18";

/**
 * WHY a local interface: `protocols` is how a WebTransport client fills
 * WT-Available-Protocols, which §3.1 uses for version negotiation, and
 * `WebTransportOptions` in lib.dom.d.ts of TypeScript 6.0.3 does not declare
 * it.
 */
interface WebTransportOptionsWithProtocols extends WebTransportOptions {
	protocols?: string[];
}

export interface ConnectOptions extends SessionConnectOptions {
	/** Pins a certificate no root store trusts, such as a development one. */
	serverCertificateHashes?: WebTransportHash[];
}

function adapt(webTransport: WebTransport): MoqtTransport {
	return {
		closed: webTransport.closed,
		incomingUnidirectionalStreams: webTransport.incomingUnidirectionalStreams,
		incomingBidirectionalStreams: webTransport.incomingBidirectionalStreams,
		createUnidirectionalStream: () => webTransport.createUnidirectionalStream(),
		createBidirectionalStream: () => webTransport.createBidirectionalStream(),
		close: (info) => webTransport.close(info),
		streamError: (code, message) =>
			new WebTransportError(message, { streamErrorCode: code }),
		// A WebTransportError says in `source` whether a stream or the session
		// failed.
		isStreamError: (error) =>
			error instanceof WebTransportError && error.source === "stream",
		streamErrorCode: (error) =>
			error instanceof WebTransportError
				? (error.streamErrorCode ?? undefined)
				: undefined,
	};
}

/**
 * Why a session whose negotiated subprotocol is `protocol` cannot be used, or
 * nothing when it can. §3.1: the version is negotiated by the subprotocol, so
 * without the expected one the wire format of the peer is unknown.
 */
export function protocolRefusal(protocol: unknown): string | undefined {
	if (protocol === MOQT_PROTOCOL) return undefined;
	return `the server selected protocol ${String(protocol)}, not ${MOQT_PROTOCOL}`;
}

/** Opens a WebTransport session to `url` and runs a MOQT session over it. */
export async function connect(
	url: string,
	options: ConnectOptions = {},
): Promise<MoqtConnection> {
	const transportOptions: WebTransportOptionsWithProtocols = {
		protocols: [MOQT_PROTOCOL],
	};
	if (options.serverCertificateHashes !== undefined) {
		transportOptions.serverCertificateHashes = options.serverCertificateHashes;
	}
	const webTransport = new WebTransport(url, transportOptions);
	// When the connection fails, `closed` rejects along with `ready`, and only
	// `ready` is awaited here. In Chromium 156 the unobserved `closed` raised no
	// `unhandledrejection` (checked on 2026-10-10 with a connection to a port
	// nobody listens on); other browsers were not checked, so it is observed
	// here. Once connected, `connectOver` reads `closed` for its outcome.
	webTransport.closed.catch(() => {});
	await webTransport.ready;

	// WHY Reflect.get: `WebTransport` in lib.dom.d.ts of TypeScript 6.0.3 has no
	// `protocol` (TS2339). Chromium 156 has it; the runs in
	// docs/experiments/2026-10-10-own-ts-client-vs-moqtail.md read "moqt-18".
	const refusal = protocolRefusal(Reflect.get(webTransport, "protocol"));
	if (refusal !== undefined) {
		// WHY INTERNAL_ERROR and not VERSION_NEGOTIATION_FAILED: §3.5 defines the
		// latter as "The client didn't offer a version supported by the server",
		// a server's verdict. Here the client is the one that cannot go on.
		webTransport.close({
			closeCode: SessionErrorCode.INTERNAL_ERROR,
			reason: refusal,
		});
		throw new Error(refusal);
	}
	return connectOver(adapt(webTransport), options);
}
