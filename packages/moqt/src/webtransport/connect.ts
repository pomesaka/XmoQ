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

/** What the connection reads of a `WebTransportError`. */
interface StreamError {
	readonly source: string;
	readonly streamErrorCode: number | null;
}

type StreamErrorConstructor = new (...args: never[]) => StreamError;

/**
 * The error that resets a stream, or stops its sender, with `code`: the
 * browser takes the code from a `WebTransportError` given to `abort()` or
 * `cancel()` and uses 0 for anything else (W3C WebTransport, "abort a
 * WebTransportSendStream" and "cancel a WebTransportReceiveStream").
 *
 * WHY two forms: the specification has `new WebTransportError(message,
 * options)` (§12, Candidate Recommendation Snapshot of 30 July 2026 and
 * Editor's Draft of 6 October 2026). Chromium 156.0.8078.4 takes one
 * dictionary, `{ message, streamErrorCode }`, and throws a TypeError when the
 * first argument is a string (checked in a page on 2026-10-10).
 * WHY the form of the specification first: a browser that follows it reads a
 * dictionary given first as the message and builds an error without a code,
 * which is why the code of what was built is compared as well.
 * WHY `Reflect.construct`: `WebTransportError` in lib.dom.d.ts of TypeScript
 * 6.0.3 declares the form of the specification only.
 *
 * Throws when neither form gives an error with `code`. That is the case in
 * Chromium 156 for a code above 255, which it clamps to 255
 * (`MAX_STREAM_ERROR_CODE` in connection.ts keeps callers below it).
 */
export function createStreamError(
	errorConstructor: StreamErrorConstructor,
	code: number,
	message: string,
): StreamError {
	const forms: unknown[][] = [
		[message, { streamErrorCode: code }],
		[{ message, streamErrorCode: code }],
	];
	for (const form of forms) {
		let built: unknown;
		try {
			built = Reflect.construct(errorConstructor, form);
		} catch (error) {
			if (error instanceof TypeError) continue;
			throw error;
		}
		if (built instanceof errorConstructor && built.streamErrorCode === code) {
			return built;
		}
	}
	throw new Error(
		`WebTransportError could not be built with stream error code ${code} in either of its two forms`,
	);
}

/** The members of the browser's `WebTransport` that the connection uses. */
type BrowserTransport = Omit<
	MoqtTransport,
	| "streamError"
	| "isStreamError"
	| "isStopWithoutCode"
	| "isSessionError"
	| "streamErrorCode"
>;

/** `errorConstructor` is the browser's `WebTransportError`; tests give their own. */
export function adaptWebTransport(
	webTransport: BrowserTransport,
	errorConstructor: StreamErrorConstructor,
): MoqtTransport {
	return {
		closed: webTransport.closed,
		incomingUnidirectionalStreams: webTransport.incomingUnidirectionalStreams,
		incomingBidirectionalStreams: webTransport.incomingBidirectionalStreams,
		datagrams: webTransport.datagrams,
		createUnidirectionalStream: (options) =>
			webTransport.createUnidirectionalStream(options),
		createBidirectionalStream: () => webTransport.createBidirectionalStream(),
		close: (info) => webTransport.close(info),
		streamError: (code, message) =>
			createStreamError(errorConstructor, code, message),
		// A WebTransportError says in `source` whether a stream or the session
		// failed.
		isStreamError: (error) =>
			error instanceof errorConstructor && error.source === "stream",
		// WHY a NetworkError is a stop: Chromium rejects a writer with it, and
		// not with a WebTransportError, when the data pipe of the stream closes
		// before the STOP_SENDING is reported, which happens while a write is
		// under way (`OutgoingStream::HandlePipeClosed`). In Chromium 156, 56
		// of 720 writers of a stream that was stopped got it, and none of 1080
		// writers of a session that ended or was lost, which all got a
		// WebTransportError with `source` "session"
		// (docs/experiments/2026-10-11-write-abort-in-chromium.md). The code
		// of the stop does not reach the page then.
		// "None in 1080" is not "cannot", so this reading must close no
		// session: the session takes it in a way that does nothing for the
		// control stream (`Session.localStreamAborted`).
		isStopWithoutCode: (error) =>
			error instanceof DOMException && error.name === "NetworkError",
		isSessionError: (error) =>
			error instanceof errorConstructor && error.source === "session",
		streamErrorCode: (error) =>
			error instanceof errorConstructor
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
	return connectOver(
		adaptWebTransport(webTransport, WebTransportError),
		options,
	);
}
