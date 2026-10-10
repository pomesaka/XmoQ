import {
	type ByteReader,
	type Bytes,
	type ByteWriter,
	concatBytes,
	createByteReader,
	createByteWriter,
	ShortBufferError,
} from "./bytes.ts";
import {
	MoqtError,
	NotImplementedError,
	protocolViolation,
	SessionErrorCode,
} from "./error.ts";
import {
	decodeKeyValuePairs,
	encodeKeyValuePairs,
	type KeyValuePair,
	TooManyEntriesError,
} from "./key-value-pair.ts";
import {
	decodeLocation,
	decodeMessageParameters,
	encodeLocation,
	encodeMessageParameters,
	type Location,
	type MessageParameters,
} from "./parameters.ts";
import {
	decodeFullTrackName,
	decodeTrackNamespace,
	encodeFullTrackName,
	encodeTrackNamespace,
	type FullTrackName,
	type TrackNamespace,
} from "./track.ts";
import { readLength, wireEnum } from "./wire.ts";

/** Redirect structure of §10.6.1. */
export interface Redirect extends FullTrackName {
	connectUri: string;
}

export interface Setup {
	type: "SETUP";
	setupOptions: KeyValuePair[];
}

export interface Subscribe extends FullTrackName {
	type: "SUBSCRIBE";
	requestId: bigint;
	parameters: MessageParameters;
}

export interface SubscribeOk {
	type: "SUBSCRIBE_OK";
	trackAlias: bigint;
	parameters: MessageParameters;
	trackProperties: KeyValuePair[];
}

export interface Publish extends FullTrackName {
	type: "PUBLISH";
	requestId: bigint;
	trackAlias: bigint;
	parameters: MessageParameters;
	trackProperties: KeyValuePair[];
}

export interface PublishNamespace {
	type: "PUBLISH_NAMESPACE";
	requestId: bigint;
	trackNamespace: TrackNamespace;
	parameters: MessageParameters;
}

/**
 * REQUEST_OK of §10.5. The draft calls it PUBLISH_OK or PUBLISH_NAMESPACE_OK
 * after the request it answers; the wire message is the same.
 */
export interface RequestOk {
	type: "REQUEST_OK";
	parameters: MessageParameters;
	trackProperties: KeyValuePair[];
}

export interface RequestError {
	type: "REQUEST_ERROR";
	errorCode: bigint;
	/** Milliseconds plus one; 0 means do not retry (§10.6.2). */
	retryInterval: bigint;
	errorReason: string;
	/** Present only when `errorCode` is REDIRECT. */
	redirect?: Redirect;
}

export interface PublishDone {
	type: "PUBLISH_DONE";
	statusCode: bigint;
	streamCount: bigint;
	errorReason: string;
}

/**
 * REQUEST_UPDATE of §10.9, for a subscription. The update of a namespace
 * subscription, which carries TRACK_NAMESPACE_PREFIX, is not implemented.
 */
export interface RequestUpdate {
	type: "REQUEST_UPDATE";
	requestId: bigint;
	parameters: MessageParameters;
}

/** Which Objects a FETCH asks for (§10.12). */
export type FetchRange =
	/** Fetch Type 0x1: a range of a track, independent of any subscription (§10.12.1). */
	| ({
			fetchType: "Standalone";
			startLocation: Location;
			/**
			 * "The end Location, plus 1. A Location.Object value of 0 means the
			 * entire group is requested."
			 */
			endLocation: Location;
	  } & FullTrackName)
	/**
	 * Fetch Type 0x2: up to the Joining Location of a subscription of the
	 * same session, from `joiningStart` Groups before it (§10.12.2).
	 */
	| {
			fetchType: "RelativeJoining";
			joiningRequestId: bigint;
			joiningStart: bigint;
	  }
	/** Fetch Type 0x3: the same, from the Group `joiningStart` (§10.12.2). */
	| {
			fetchType: "AbsoluteJoining";
			joiningRequestId: bigint;
			joiningStart: bigint;
	  };

/** FETCH (§10.12.3). */
export interface Fetch {
	type: "FETCH";
	requestId: bigint;
	range: FetchRange;
	parameters: MessageParameters;
}

/** FETCH_OK (§10.13). */
export interface FetchOk {
	type: "FETCH_OK";
	/** Whether the End Location is the final Object of the track. */
	endOfTrack: boolean;
	/**
	 * The end of the range the response covers, in the encoding of a FETCH:
	 * the last Object plus 1, or Object 0 for a whole Group.
	 */
	endLocation: Location;
	/** No parameter of the draft may appear here; one that does is a PROTOCOL_VIOLATION. */
	parameters: MessageParameters;
	trackProperties: KeyValuePair[];
}

export type ControlMessage =
	| Setup
	| RequestUpdate
	| Subscribe
	| SubscribeOk
	| Publish
	| PublishNamespace
	| RequestOk
	| RequestError
	| PublishDone
	| Fetch
	| FetchOk;

type MessageName = ControlMessage["type"];

/** Message Type values of Table 5 (§10) for the messages implemented here. */
const MESSAGE_TYPES: Record<MessageName, bigint> = {
	SETUP: 0x2f00n,
	REQUEST_UPDATE: 0x2n,
	SUBSCRIBE: 0x3n,
	SUBSCRIBE_OK: 0x4n,
	REQUEST_ERROR: 0x5n,
	PUBLISH_NAMESPACE: 0x6n,
	REQUEST_OK: 0x7n,
	PUBLISH_DONE: 0xbn,
	FETCH: 0x16n,
	FETCH_OK: 0x18n,
	PUBLISH: 0x1dn,
};

/**
 * Message Types that Table 5 lists and this implementation does not decode.
 * Receiving one closes the session with INTERNAL_ERROR: the peer is within
 * the draft, this endpoint is not complete.
 *
 * WHY PUBLISH_OK (0x1E) is here although §10.5 defines PUBLISH_OK as a
 * REQUEST_OK (0x7) and gives 0x1E no format: Table 5 lists it, so a peer that
 * sends it has not left the draft.
 */
const UNIMPLEMENTED_MESSAGE_TYPES = {
	NAMESPACE: 0x8n,
	TRACK_STATUS: 0xdn,
	NAMESPACE_DONE: 0xen,
	PUBLISH_BLOCKED: 0xfn,
	// FIXME: GOAWAY is a legitimate message (§10.4) and closing the session on
	// it turns a planned migration into an error. Decode it and report it to
	// the application.
	GOAWAY: 0x10n,
	PUBLISH_OK: 0x1en,
	SUBSCRIBE_NAMESPACE: 0x50n,
	SUBSCRIBE_TRACKS: 0x51n,
} satisfies Record<string, bigint>;

/** §3.3: the seven message types a request stream can begin with. */
const REQUEST_MESSAGE_TYPES: ReadonlySet<bigint> = new Set([
	UNIMPLEMENTED_MESSAGE_TYPES.TRACK_STATUS,
	MESSAGE_TYPES.SUBSCRIBE,
	MESSAGE_TYPES.PUBLISH,
	MESSAGE_TYPES.FETCH,
	MESSAGE_TYPES.PUBLISH_NAMESPACE,
	UNIMPLEMENTED_MESSAGE_TYPES.SUBSCRIBE_NAMESPACE,
	UNIMPLEMENTED_MESSAGE_TYPES.SUBSCRIBE_TRACKS,
]);

/** The Message Type of FETCH, the one request that is decoded whole before it is refused. */
export const FETCH_MESSAGE_TYPE = MESSAGE_TYPES.FETCH;

/** The Message Type of SUBSCRIBE, which is decoded whole and handed to the application. */
export const SUBSCRIBE_MESSAGE_TYPE = MESSAGE_TYPES.SUBSCRIBE;

/**
 * WHY SETUP has one value in two tables: Table 3 (stream types, §3.4) and
 * Table 5 (message types, §10) both say 0x2F00, because the control stream
 * begins with the SETUP message itself.
 */
export const SETUP_MESSAGE_TYPE = MESSAGE_TYPES.SETUP;

type MessageTypeClass =
	| { kind: "implemented"; name: MessageName }
	| { kind: "unimplemented"; name: string }
	| { kind: "unknown" };

const IMPLEMENTED_MESSAGE_NAMES = wireEnum(MESSAGE_TYPES);
const UNIMPLEMENTED_MESSAGE_NAMES: ReadonlyMap<bigint, string> = new Map(
	Object.entries(UNIMPLEMENTED_MESSAGE_TYPES).map(([name, value]) => [
		value,
		name,
	]),
);

/** The one place that decides how a Message Type is treated. */
function classifyMessageType(messageType: bigint): MessageTypeClass {
	const implemented = IMPLEMENTED_MESSAGE_NAMES.fromWire(messageType);
	if (implemented !== undefined) {
		return { kind: "implemented", name: implemented };
	}
	const unimplemented = UNIMPLEMENTED_MESSAGE_NAMES.get(messageType);
	if (unimplemented !== undefined) {
		return { kind: "unimplemented", name: unimplemented };
	}
	return { kind: "unknown" };
}

function unsupportedMessageType(
	messageType: bigint,
	messageClass: Exclude<MessageTypeClass, { kind: "implemented" }>,
): MoqtError {
	if (messageClass.kind === "unimplemented") {
		return new NotImplementedError(`${messageClass.name} message`);
	}
	// §10: "An endpoint that receives an unknown message type MUST close the
	// session." The draft names no code; PROTOCOL_VIOLATION is used.
	return protocolViolation(
		`unknown Message Type 0x${messageType.toString(16)}`,
	);
}

/** REQUEST_ERROR code REDIRECT (§15.10.2), the only code that adds a field to the message. */
const REDIRECT = 0x34n;

/** §1.4.4 */
const MAX_REASON_PHRASE_LENGTH = 1024;
/** Message Length is a 16-bit field (§10). */
const MAX_MESSAGE_PAYLOAD_LENGTH = 0xffff;

// WHY not `fatal`: §1.4.4 says a Reason Phrase is UTF-8 and names no error for
// bytes that are not. The text is diagnostic and is never encoded again by a
// client, so malformed bytes become U+FFFD instead of ending the session.
const utf8Decoder = new TextDecoder();
const utf8Encoder = new TextEncoder();

function decodeReasonPhrase(reader: ByteReader): string {
	const length = readLength(
		reader,
		MAX_REASON_PHRASE_LENGTH,
		"Reason Phrase Length",
	);
	return utf8Decoder.decode(reader.take(length));
}

function encodeReasonPhrase(writer: ByteWriter, reason: string): void {
	const bytes = utf8Encoder.encode(reason);
	if (bytes.length > MAX_REASON_PHRASE_LENGTH) {
		throw new RangeError(
			`Reason Phrase exceeds ${MAX_REASON_PHRASE_LENGTH} bytes`,
		);
	}
	writer.varint(bytes.length).bytes(bytes);
}

function decodeRedirect(reader: ByteReader): Redirect {
	const uriLength = readLength(reader, reader.remaining, "Connect URI Length");
	const connectUri = utf8Decoder.decode(reader.take(uriLength));
	return { connectUri, ...decodeFullTrackName(reader) };
}

function encodeRedirect(writer: ByteWriter, redirect: Redirect): void {
	const uri = utf8Encoder.encode(redirect.connectUri);
	writer.varint(uri.length).bytes(uri);
	encodeFullTrackName(writer, redirect);
}

const FETCH_TYPES = wireEnum<FetchRange["fetchType"]>({
	Standalone: 0x1n,
	RelativeJoining: 0x2n,
	AbsoluteJoining: 0x3n,
});

function decodeFetchRange(reader: ByteReader): FetchRange {
	const value = reader.varint();
	const fetchType = FETCH_TYPES.fromWire(value);
	switch (fetchType) {
		case "Standalone": {
			const fullTrackName = decodeFullTrackName(reader);
			const startLocation = decodeLocation(reader);
			const endLocation = decodeLocation(reader);
			return { fetchType, ...fullTrackName, startLocation, endLocation };
		}
		case "RelativeJoining":
		case "AbsoluteJoining": {
			const joiningRequestId = reader.varint();
			const joiningStart = reader.varint();
			return { fetchType, joiningRequestId, joiningStart };
		}
		// §10.12: "An endpoint that receives a Fetch Type other than 0x1, 0x2
		// or 0x3 MUST close the session with a PROTOCOL_VIOLATION."
		case undefined:
			throw protocolViolation(`unknown Fetch Type ${value}`);
	}
}

function encodeFetchRange(writer: ByteWriter, range: FetchRange): void {
	writer.varint(FETCH_TYPES.toWire(range.fetchType));
	switch (range.fetchType) {
		case "Standalone":
			encodeFullTrackName(writer, range);
			encodeLocation(writer, range.startLocation);
			encodeLocation(writer, range.endLocation);
			break;
		case "RelativeJoining":
		case "AbsoluteJoining":
			writer.varint(range.joiningRequestId).varint(range.joiningStart);
			break;
	}
}

/**
 * End Of Track of FETCH_OK. §10.13 gives the field 8 bits and the values 1
 * and 0. WHY any other value is a violation: nothing says what it would
 * mean, and FORWARD, the other field of that kind, is read so (§10.2.12). The
 * Rust implementation reads it the same way.
 */
function decodeEndOfTrack(value: number): boolean {
	switch (value) {
		case 0:
			return false;
		case 1:
			return true;
		default:
			throw protocolViolation(`End Of Track value ${value} is neither 0 nor 1`);
	}
}

function decodePayload(name: MessageName, reader: ByteReader): ControlMessage {
	switch (name) {
		case "SETUP":
			return { type: name, setupOptions: decodeKeyValuePairs(reader) };
		case "REQUEST_UPDATE": {
			const requestId = reader.varint();
			// Every update is refused (see `requests.ts`), but its parameters are
			// still held to §10.2.1: a parameter that is not allowed here is a
			// PROTOCOL_VIOLATION, not an update to refuse.
			const parameters = decodeMessageParameters(reader, name);
			return { type: name, requestId, parameters };
		}
		case "SUBSCRIBE": {
			const requestId = reader.varint();
			const fullTrackName = decodeFullTrackName(reader);
			const parameters = decodeMessageParameters(reader, name);
			return { type: name, requestId, ...fullTrackName, parameters };
		}
		case "SUBSCRIBE_OK": {
			const trackAlias = reader.varint();
			const parameters = decodeMessageParameters(reader, name);
			const trackProperties = decodeKeyValuePairs(reader);
			return { type: name, trackAlias, parameters, trackProperties };
		}
		case "PUBLISH": {
			const requestId = reader.varint();
			const fullTrackName = decodeFullTrackName(reader);
			const trackAlias = reader.varint();
			const parameters = decodeMessageParameters(reader, name);
			const trackProperties = decodeKeyValuePairs(reader);
			return {
				type: name,
				requestId,
				...fullTrackName,
				trackAlias,
				parameters,
				trackProperties,
			};
		}
		case "PUBLISH_NAMESPACE": {
			const requestId = reader.varint();
			const trackNamespace = decodeTrackNamespace(reader);
			const parameters = decodeMessageParameters(reader, name);
			return { type: name, requestId, trackNamespace, parameters };
		}
		case "REQUEST_OK": {
			const parameters = decodeMessageParameters(reader, name);
			const trackProperties = decodeKeyValuePairs(reader);
			return { type: name, parameters, trackProperties };
		}
		case "REQUEST_ERROR": {
			const errorCode = reader.varint();
			const retryInterval = reader.varint();
			const errorReason = decodeReasonPhrase(reader);
			if (errorCode !== REDIRECT) {
				return { type: name, errorCode, retryInterval, errorReason };
			}
			const redirect = decodeRedirect(reader);
			return { type: name, errorCode, retryInterval, errorReason, redirect };
		}
		case "PUBLISH_DONE": {
			const statusCode = reader.varint();
			const streamCount = reader.varint();
			const errorReason = decodeReasonPhrase(reader);
			return { type: name, statusCode, streamCount, errorReason };
		}
		case "FETCH": {
			const requestId = reader.varint();
			const range = decodeFetchRange(reader);
			const parameters = decodeMessageParameters(reader, name);
			return { type: name, requestId, range, parameters };
		}
		case "FETCH_OK": {
			const endOfTrack = decodeEndOfTrack(reader.uint8());
			const endLocation = decodeLocation(reader);
			const parameters = decodeMessageParameters(reader, name);
			const trackProperties = decodeKeyValuePairs(reader);
			return {
				type: name,
				endOfTrack,
				endLocation,
				parameters,
				trackProperties,
			};
		}
	}
}

export type DecodeResult<T> =
	/** The input ends inside the value; call again with more bytes. */
	| { status: "incomplete" }
	| { status: "ok"; value: T; bytesRead: number }
	| { status: "error"; error: MoqtError };

/** Message Type and Message Length of a control message (Figure 3), without the payload. */
function decodeFrameHeader(
	reader: ByteReader,
): { messageType: bigint; payloadLength: number } | undefined {
	try {
		const messageType = reader.varint();
		const payloadLength = reader.uint16();
		return { messageType, payloadLength };
	} catch (error) {
		if (error instanceof ShortBufferError) return undefined;
		throw error;
	}
}

/**
 * Decodes one control message from the front of `buffer`. Because Message
 * Length is 16 bits, a caller never has to hold more than 9 + 2 + 65535 bytes
 * before this returns "ok" or "error".
 */
export function decodeControlMessage(
	buffer: Uint8Array,
): DecodeResult<ControlMessage> {
	const reader = createByteReader(buffer);
	const header = decodeFrameHeader(reader);
	if (header === undefined) return { status: "incomplete" };
	const { messageType, payloadLength } = header;

	// Decided before the payload arrives, so an unsupported type fails without
	// buffering it.
	const messageClass = classifyMessageType(messageType);
	if (messageClass.kind !== "implemented") {
		return {
			status: "error",
			error: unsupportedMessageType(messageType, messageClass),
		};
	}
	if (reader.remaining < payloadLength) return { status: "incomplete" };

	const payload = reader.sub(payloadLength);
	try {
		const value = decodePayload(messageClass.name, payload);
		// §10: a Message Length that does not match the payload is a PROTOCOL_VIOLATION.
		if (payload.remaining > 0) {
			throw protocolViolation(
				`${value.type} leaves ${payload.remaining} bytes of its Message Length unread`,
			);
		}
		return { status: "ok", value, bytesRead: reader.offset };
	} catch (error) {
		if (error instanceof ShortBufferError) {
			return {
				status: "error",
				error: protocolViolation(
					"Message Length is shorter than the message payload",
				),
			};
		}
		if (error instanceof MoqtError) return { status: "error", error };
		if (error instanceof TooManyEntriesError) {
			// A local limit on a control message ends the session; the peer broke
			// no rule, so the code is INTERNAL_ERROR.
			return {
				status: "error",
				error: new MoqtError(SessionErrorCode.INTERNAL_ERROR, error.message),
			};
		}
		throw error;
	}
}

/**
 * Appends `data` to `buffer`, hands every complete control message to
 * `onMessage` and returns the bytes of the incomplete message that follows.
 * Throws the `MoqtError` of a message that cannot be decoded.
 */
export function drainControlMessages(
	buffer: Uint8Array,
	data: Uint8Array,
	onMessage: (message: ControlMessage) => void,
): Uint8Array {
	let rest: Uint8Array =
		buffer.length === 0 ? data : concatBytes([buffer, data]);
	for (;;) {
		const decoded = decodeControlMessage(rest);
		if (decoded.status === "error") throw decoded.error;
		if (decoded.status === "incomplete") return rest.slice();
		rest = rest.subarray(decoded.bytesRead);
		onMessage(decoded.value);
	}
}

/**
 * Reads the start of the first message on a bidirectional stream the peer
 * opened: its Message Type and its Request ID. Every message a request stream
 * can begin with has Request ID as its first field (Figures 10, 13, 17 and
 * the FETCH, SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS formats; TRACK_STATUS
 * uses the SUBSCRIBE format, §10.14), so the rest of the message is not
 * needed to reject the request. `messageLength` is how many bytes the whole
 * message takes, with its Message Type and Message Length.
 */
export function decodeRequestStart(buffer: Uint8Array): DecodeResult<{
	messageType: bigint;
	requestId: bigint;
	messageLength: number;
}> {
	const reader = createByteReader(buffer);
	const header = decodeFrameHeader(reader);
	if (header === undefined) return { status: "incomplete" };
	const { messageType, payloadLength } = header;

	if (!REQUEST_MESSAGE_TYPES.has(messageType)) {
		const messageClass = classifyMessageType(messageType);
		// §3.3: a bidirectional stream that begins with any other message type is
		// a PROTOCOL_VIOLATION.
		const error =
			messageClass.kind === "unknown"
				? unsupportedMessageType(messageType, messageClass)
				: protocolViolation(`a request stream began with ${messageClass.name}`);
		return { status: "error", error };
	}
	const messageLength = reader.offset + payloadLength;
	// A varint takes at most 9 bytes, so this much of the payload decides it.
	const needed = Math.min(payloadLength, 9);
	const arrived = Math.min(needed, reader.remaining);
	const start = reader.sub(arrived);
	try {
		const requestId = start.varint();
		return {
			status: "ok",
			value: { messageType, requestId, messageLength },
			bytesRead: reader.offset - start.remaining,
		};
	} catch (error) {
		if (!(error instanceof ShortBufferError)) throw error;
		if (arrived < needed) return { status: "incomplete" };
		return {
			status: "error",
			error: protocolViolation("Message Length ends inside the Request ID"),
		};
	}
}

function encodePayload(writer: ByteWriter, message: ControlMessage): void {
	switch (message.type) {
		case "SETUP":
			encodeKeyValuePairs(writer, message.setupOptions);
			break;
		case "REQUEST_UPDATE":
			writer.varint(message.requestId);
			encodeMessageParameters(writer, message.parameters, message.type);
			break;
		case "SUBSCRIBE":
			writer.varint(message.requestId);
			encodeFullTrackName(writer, message);
			encodeMessageParameters(writer, message.parameters, message.type);
			break;
		case "SUBSCRIBE_OK":
			writer.varint(message.trackAlias);
			encodeMessageParameters(writer, message.parameters, message.type);
			encodeKeyValuePairs(writer, message.trackProperties);
			break;
		case "PUBLISH":
			writer.varint(message.requestId);
			encodeFullTrackName(writer, message);
			writer.varint(message.trackAlias);
			encodeMessageParameters(writer, message.parameters, message.type);
			encodeKeyValuePairs(writer, message.trackProperties);
			break;
		case "PUBLISH_NAMESPACE":
			writer.varint(message.requestId);
			encodeTrackNamespace(writer, message.trackNamespace);
			encodeMessageParameters(writer, message.parameters, message.type);
			break;
		case "REQUEST_OK":
			encodeMessageParameters(writer, message.parameters, message.type);
			encodeKeyValuePairs(writer, message.trackProperties);
			break;
		case "REQUEST_ERROR":
			writer.varint(message.errorCode).varint(message.retryInterval);
			encodeReasonPhrase(writer, message.errorReason);
			if (
				(message.errorCode === REDIRECT) !==
				(message.redirect !== undefined)
			) {
				throw new RangeError(
					"Redirect is present exactly when Error Code is REDIRECT",
				);
			}
			if (message.redirect !== undefined) {
				encodeRedirect(writer, message.redirect);
			}
			break;
		case "PUBLISH_DONE":
			writer.varint(message.statusCode).varint(message.streamCount);
			encodeReasonPhrase(writer, message.errorReason);
			break;
		case "FETCH":
			writer.varint(message.requestId);
			encodeFetchRange(writer, message.range);
			encodeMessageParameters(writer, message.parameters, message.type);
			break;
		case "FETCH_OK":
			writer.uint8(message.endOfTrack ? 1 : 0);
			encodeLocation(writer, message.endLocation);
			encodeMessageParameters(writer, message.parameters, message.type);
			encodeKeyValuePairs(writer, message.trackProperties);
			break;
	}
}

export function encodeControlMessage(message: ControlMessage): Bytes {
	const payload = createByteWriter();
	encodePayload(payload, message);
	if (payload.length > MAX_MESSAGE_PAYLOAD_LENGTH) {
		throw new RangeError(
			`${message.type} payload exceeds ${MAX_MESSAGE_PAYLOAD_LENGTH} bytes`,
		);
	}
	return createByteWriter()
		.varint(MESSAGE_TYPES[message.type])
		.uint16(payload.length)
		.bytes(payload.finish())
		.finish();
}
