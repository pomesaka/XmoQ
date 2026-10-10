import {
	type ByteReader,
	type Bytes,
	concatBytes,
	createByteReader,
	createByteWriter,
	MAX_VARINT,
	ShortBufferError,
} from "./bytes.ts";
import { MoqtError, protocolViolation } from "./error.ts";
import {
	decodeKeyValuePairs,
	encodeKeyValuePairs,
	isMandatoryTrackProperty,
	type KeyValuePair,
	TooManyEntriesError,
} from "./key-value-pair.ts";
import { wireEnum } from "./wire.ts";

/** SUBGROUP_ID_MODE of §11.4.2. `Explicit` is the only mode with a Subgroup ID field. */
export type SubgroupId =
	/** 0b00: the Subgroup ID is 0. */
	| { mode: "Zero" }
	/** 0b01: the Subgroup ID is the Object ID of the first Object on the stream. */
	| { mode: "FirstObjectId" }
	/** 0b10 */
	| { mode: "Explicit"; value: bigint };

/** SUBGROUP_HEADER of §11.4.2, with the Type bits spread into fields. */
export interface SubgroupHeader {
	trackAlias: bigint;
	groupId: bigint;
	subgroupId: SubgroupId;
	/** `undefined` sets the DEFAULT_PRIORITY bit: the field is omitted from the header. */
	publisherPriority: number | undefined;
	/** PROPERTIES bit: every Object on the stream carries an Object Properties structure. */
	properties: boolean;
	/** END_OF_GROUP bit */
	endOfGroup: boolean;
	/** FIRST_OBJECT bit */
	firstObject: boolean;
}

/** Object Status of §11.2.1.1. */
export type ObjectStatus = "Normal" | "EndOfGroup" | "EndOfTrack";

/** One Object on a subgroup stream (Figure 25), with the Object ID resolved from its delta. */
export interface SubgroupObject {
	objectId: bigint;
	properties: KeyValuePair[];
	status: ObjectStatus;
	payload: Uint8Array;
}

const TYPE_PROPERTIES = 0x01;
const TYPE_SUBGROUP_ID_MODE = 0x06;
const TYPE_END_OF_GROUP = 0x08;
const TYPE_SUBGROUP_HEADER = 0x10;
const TYPE_DEFAULT_PRIORITY = 0x20;
const TYPE_FIRST_OBJECT = 0x40;

/** The value 0b11 is reserved (§11.4.2) and has no name here. */
const SUBGROUP_ID_MODES = wireEnum<SubgroupId["mode"]>({
	Zero: 0b00n,
	FirstObjectId: 0b01n,
	Explicit: 0b10n,
});

const OBJECT_STATUSES = wireEnum<ObjectStatus>({
	Normal: 0x0n,
	EndOfGroup: 0x3n,
	EndOfTrack: 0x4n,
});

/** True when a unidirectional stream type has the SUBGROUP_HEADER form 0b0XX1XXXX (§3.4). */
export function isSubgroupHeaderType(streamType: bigint): boolean {
	return (
		streamType < 0x80n && (Number(streamType) & TYPE_SUBGROUP_HEADER) !== 0
	);
}

function decodeSubgroupHeader(reader: ByteReader): SubgroupHeader {
	const typeValue = reader.varint();
	if (!isSubgroupHeaderType(typeValue)) {
		throw protocolViolation(
			`0x${typeValue.toString(16)} is not a SUBGROUP_HEADER Type`,
		);
	}
	const type = Number(typeValue);
	// Checked before the fields that follow, so an invalid Type fails without
	// waiting for them (§11.4.2).
	const mode = SUBGROUP_ID_MODES.fromWire(
		BigInt((type & TYPE_SUBGROUP_ID_MODE) >> 1),
	);
	if (mode === undefined) {
		throw protocolViolation("SUBGROUP_ID_MODE 0b11 is reserved");
	}
	const trackAlias = reader.varint();
	const groupId = reader.varint();
	const subgroupId: SubgroupId =
		mode === "Explicit" ? { mode, value: reader.varint() } : { mode };
	const publisherPriority =
		(type & TYPE_DEFAULT_PRIORITY) !== 0 ? undefined : reader.uint8();

	return {
		trackAlias,
		groupId,
		subgroupId,
		publisherPriority,
		properties: (type & TYPE_PROPERTIES) !== 0,
		endOfGroup: (type & TYPE_END_OF_GROUP) !== 0,
		firstObject: (type & TYPE_FIRST_OBJECT) !== 0,
	};
}

export function encodeSubgroupHeader(header: SubgroupHeader): Bytes {
	const type =
		TYPE_SUBGROUP_HEADER |
		(header.properties ? TYPE_PROPERTIES : 0) |
		(Number(SUBGROUP_ID_MODES.toWire(header.subgroupId.mode)) << 1) |
		(header.endOfGroup ? TYPE_END_OF_GROUP : 0) |
		(header.publisherPriority === undefined ? TYPE_DEFAULT_PRIORITY : 0) |
		(header.firstObject ? TYPE_FIRST_OBJECT : 0);
	const writer = createByteWriter()
		.varint(type)
		.varint(header.trackAlias)
		.varint(header.groupId);
	if (header.subgroupId.mode === "Explicit") {
		writer.varint(header.subgroupId.value);
	}
	if (header.publisherPriority !== undefined) {
		writer.uint8(header.publisherPriority);
	}
	return writer.finish();
}

/**
 * Throws, for an encoder, when the Object Properties would make a receiver
 * take the Track for malformed: a Mandatory Track Property (§2.5.1), and a
 * Prior Group ID Gap or Prior Object ID Gap that is there twice or is larger
 * than the ID it counts back from (§12.8, §12.9).
 *
 * WHY the same check as the decoder's: what this endpoint's decoder refuses,
 * its encoder does not write. The Rust implementation holds its encoders to
 * the check of its decoders as well.
 */
export function refuseObjectPropertiesThatBreakTheTrack(
	properties: readonly KeyValuePair[],
	groupId: bigint,
	objectId: bigint,
): void {
	try {
		checkObjectProperties(properties, groupId, objectId);
	} catch (error) {
		if (error instanceof DecodeStop && error.failure.scope === "track") {
			throw new RangeError(`${error.failure.reason} (§2.5.1, §12.8, §12.9)`);
		}
		throw error;
	}
}

/**
 * Encodes the Subgroup Object Fields of Figure 25. `previousObjectId` is the
 * Object ID of the Object written before this one on the same stream.
 */
export function encodeSubgroupObject(
	header: Pick<SubgroupHeader, "properties" | "groupId">,
	previousObjectId: bigint | undefined,
	object: SubgroupObject,
): Bytes {
	if (previousObjectId !== undefined && object.objectId <= previousObjectId) {
		throw new RangeError("Object IDs on a subgroup stream must ascend (§2.2)");
	}
	if (object.status !== "Normal" && object.payload.length > 0) {
		throw new RangeError(
			"an Object with a status other than Normal has no payload (§11.2.1.1)",
		);
	}
	if (object.status !== "Normal" && object.properties.length > 0) {
		throw new RangeError(
			"only an Object with status Normal has properties (§11.2.1.2)",
		);
	}
	if (!header.properties && object.properties.length > 0) {
		throw new RangeError("the SUBGROUP_HEADER has no PROPERTIES bit");
	}
	refuseObjectPropertiesThatBreakTheTrack(
		object.properties,
		header.groupId,
		object.objectId,
	);

	// §11.4.2: the delta is from the previous Object ID plus one, or the ID itself for the first Object.
	const delta =
		previousObjectId === undefined
			? object.objectId
			: object.objectId - previousObjectId - 1n;
	const writer = createByteWriter().varint(delta);
	if (header.properties) {
		const properties = createByteWriter();
		encodeKeyValuePairs(properties, object.properties);
		writer.varint(properties.length).bytes(properties.finish());
	}
	writer.varint(object.payload.length);
	// §11.4.2: Object Status is sent only when Object Payload Length is zero.
	if (object.payload.length === 0) {
		writer.varint(OBJECT_STATUSES.toWire(object.status));
	}
	return writer.bytes(object.payload).finish();
}

export interface SubgroupStreamLimits {
	/** Largest Object Payload Length accepted. The draft sets no maximum. */
	maxObjectPayloadLength: number;
	/** Largest Object Properties Length accepted. The draft sets no maximum. */
	maxObjectPropertiesLength: number;
}

export type SubgroupStreamItem =
	| { type: "header"; header: SubgroupHeader }
	| { type: "object"; object: SubgroupObject };

/**
 * Why a stream of Objects, a subgroup stream or a fetch stream, cannot be
 * decoded any further, by how far the damage reaches.
 */
export type ObjectStreamFailure =
	/** The peer broke a rule of the draft: the session ends. */
	| { scope: "session"; error: MoqtError }
	/** The Track is malformed (§2.4.2): its subscription is cancelled. */
	| { scope: "track"; reason: string }
	/** A local limit was exceeded, with no rule broken: only this stream ends. */
	| { scope: "stream"; reason: string };

export interface SubgroupStreamResult {
	items: SubgroupStreamItem[];
	/** Set when decoding stopped. Items before it are valid. */
	failure?: ObjectStreamFailure;
}

/** Carries a failure that is not a `MoqtError` out of the decoding steps. */
export class DecodeStop extends Error {
	readonly failure: ObjectStreamFailure;

	constructor(failure: ObjectStreamFailure) {
		super(failure.scope);
		this.failure = failure;
	}
}

interface PendingObject {
	objectId: bigint;
	properties: KeyValuePair[];
	status: ObjectStatus;
	payloadLength: number;
}

/** The chunks as one array, without a second copy when there is only one. */
export function joinChunks(chunks: readonly Uint8Array[]): Uint8Array {
	const [only] = chunks;
	return chunks.length === 1 && only !== undefined ? only : concatBytes(chunks);
}

function decodeObjectStatus(value: bigint): ObjectStatus {
	const status = OBJECT_STATUSES.fromWire(value);
	if (status === undefined) {
		throw protocolViolation(`unknown Object Status ${value}`);
	}
	return status;
}

/** Property types of §12.8 and §12.9. Both hold a varint. */
const PRIOR_GROUP_ID_GAP = 0x3cn;
const PRIOR_OBJECT_ID_GAP = 0x3en;

/**
 * The value of a gap property. More than one instance in an Object makes the
 * Track malformed (§12.8, §12.9).
 */
function gapOf(
	properties: readonly KeyValuePair[],
	type: bigint,
	name: string,
): bigint | undefined {
	const instances = properties.filter((property) => property.type === type);
	if (instances.length > 1) {
		throw new DecodeStop({
			scope: "track",
			reason: `more than one ${name} on an Object`,
		});
	}
	const value = instances[0]?.value;
	return typeof value === "bigint" ? value : undefined;
}

/**
 * Stops decoding when the properties of one Object show its Track to be
 * malformed (§2.4.2). Only what a single Object can show is checked here; the
 * conditions of §12.8 and §12.9 that compare Objects are not.
 */
export function checkObjectProperties(
	properties: readonly KeyValuePair[],
	groupId: bigint,
	objectId: bigint,
): void {
	// §2.5.1: "An Object received with a Mandatory Track Property as an Object
	// Property is malformed."
	const mandatory = properties.find(({ type }) =>
		isMandatoryTrackProperty(type),
	);
	if (mandatory !== undefined) {
		throw new DecodeStop({
			scope: "track",
			reason: `Mandatory Track Property 0x${mandatory.type.toString(16)} on an Object`,
		});
	}
	const groupGap = gapOf(properties, PRIOR_GROUP_ID_GAP, "Prior Group ID Gap");
	if (groupGap !== undefined && groupGap > groupId) {
		throw new DecodeStop({
			scope: "track",
			reason: `Prior Group ID Gap ${groupGap} is larger than Group ID ${groupId}`,
		});
	}
	const objectGap = gapOf(
		properties,
		PRIOR_OBJECT_ID_GAP,
		"Prior Object ID Gap",
	);
	if (objectGap !== undefined && objectGap > objectId) {
		throw new DecodeStop({
			scope: "track",
			reason: `Prior Object ID Gap ${objectGap} is larger than Object ID ${objectId}`,
		});
	}
}

/**
 * Reads the Object Properties Length or the Object Payload Length of an
 * Object. One over the local limit is refused when it is read: only the
 * stream ends, since the peer broke no rule.
 */
export function readObjectFieldLength(
	reader: ByteReader,
	limit: number,
	name: "Object Properties Length" | "Object Payload Length",
): number {
	const length = reader.varint();
	if (length > BigInt(limit)) {
		throw new DecodeStop({
			scope: "stream",
			reason: `${name} ${length} exceeds the limit ${limit}`,
		});
	}
	return Number(length);
}

function decodeObjectProperties(
	reader: ByteReader,
	limits: SubgroupStreamLimits,
): KeyValuePair[] {
	const length = readObjectFieldLength(
		reader,
		limits.maxObjectPropertiesLength,
		"Object Properties Length",
	);
	return decodePropertiesBlock(reader.sub(length));
}

/** Decodes Object Properties from `block`, which holds all of them and nothing else. */
export function decodePropertiesBlock(block: ByteReader): KeyValuePair[] {
	try {
		return decodeKeyValuePairs(block);
	} catch (error) {
		// The block is complete, so running out of bytes inside it is malformed input.
		if (error instanceof ShortBufferError) {
			throw protocolViolation("Object Properties end inside a Key-Value-Pair");
		}
		if (error instanceof TooManyEntriesError) {
			throw new DecodeStop({ scope: "stream", reason: error.message });
		}
		throw error;
	}
}

export interface SubgroupStreamDecoder {
	push(data: Uint8Array): SubgroupStreamResult;
	/**
	 * Call when the stream ends with a FIN. §11.4: a FIN in the middle of a
	 * serialized Object is a PROTOCOL_VIOLATION.
	 */
	finish(): MoqtError | undefined;
}

/**
 * Incremental decoder for one subgroup stream: feed it the stream bytes as
 * they arrive, starting with the SUBGROUP_HEADER Type.
 *
 * Memory is bounded by the limits: outside an Object payload it holds at most
 * one incomplete header or Object prefix, and a payload is collected only up
 * to `maxObjectPayloadLength`.
 */
export function createSubgroupStreamDecoder(
	limits: SubgroupStreamLimits,
): SubgroupStreamDecoder {
	/** Bytes of an incomplete header or Object prefix. */
	let pending: Uint8Array = new Uint8Array(0);
	let header: SubgroupHeader | undefined;
	let previousObjectId: bigint | undefined;
	let object: PendingObject | undefined;
	let payloadChunks: Uint8Array[] = [];
	let payloadReceived = 0;
	/** An End of Group or End of Track Object arrived. */
	let ended = false;
	let failed = false;

	/** Decodes the Subgroup Object Fields of Figure 25 up to, not including, Object Payload. */
	const decodeObjectPrefix = (
		reader: ByteReader,
		streamHeader: SubgroupHeader,
	): PendingObject => {
		const delta = reader.varint();
		const objectId =
			previousObjectId === undefined ? delta : previousObjectId + delta + 1n;
		if (objectId > MAX_VARINT) {
			throw protocolViolation("Object ID exceeds 2^64 - 1");
		}
		const properties = streamHeader.properties
			? decodeObjectProperties(reader, limits)
			: [];
		// The fields of an Object are judged in the order they arrive, so what
		// its properties show about the Track is decided here, before the Object
		// Payload Length and the Object Status that follow them, and without
		// waiting for a payload that would be dropped anyway.
		checkObjectProperties(properties, streamHeader.groupId, objectId);

		const payloadLength = readObjectFieldLength(
			reader,
			limits.maxObjectPayloadLength,
			"Object Payload Length",
		);
		// §11.4.2: Object Status is present only when Object Payload Length is zero.
		const status =
			payloadLength === 0 ? decodeObjectStatus(reader.varint()) : "Normal";
		if (status !== "Normal" && properties.length > 0) {
			throw protocolViolation(
				"properties on an Object whose status is not Normal",
			);
		}
		return {
			objectId,
			properties,
			status,
			payloadLength,
		};
	};

	/** Collects payload bytes; emits the Object once it is whole. Returns false when it needs more input. */
	const collectPayload = (
		current: PendingObject,
		items: SubgroupStreamItem[],
	): boolean => {
		const take = Math.min(
			current.payloadLength - payloadReceived,
			pending.length,
		);
		if (take > 0) {
			// Copied: `pending` can be a view of a buffer the caller reuses.
			payloadChunks.push(pending.slice(0, take));
			payloadReceived += take;
			pending = pending.subarray(take);
		}
		if (payloadReceived < current.payloadLength) return false;
		items.push({
			type: "object",
			object: {
				objectId: current.objectId,
				properties: current.properties,
				status: current.status,
				payload: joinChunks(payloadChunks),
			},
		});
		previousObjectId = current.objectId;
		ended = current.status !== "Normal";
		object = undefined;
		payloadChunks = [];
		payloadReceived = 0;
		return true;
	};

	/** Decodes the header or one Object prefix. Returns false when it needs more input. */
	const decodeFields = (items: SubgroupStreamItem[]): boolean => {
		if (pending.length === 0) return false;
		// §2.4.2 (conditions 4 and 5): an Object after the final Object of its
		// Group or Track makes the Track malformed. §11.4.3 calls the same thing
		// "a protocol violation" in passing; the list of §2.4.2 is taken as the
		// rule, so the subscription ends and the session does not. Its first
		// byte is enough: nothing of the Object is read or waited for.
		if (ended) {
			throw new DecodeStop({
				scope: "track",
				reason: "an Object follows the end of its Group or Track",
			});
		}
		const reader = createByteReader(pending);
		try {
			if (header === undefined) {
				header = decodeSubgroupHeader(reader);
				items.push({ type: "header", header });
			} else {
				object = decodeObjectPrefix(reader, header);
			}
		} catch (error) {
			if (error instanceof ShortBufferError) return false;
			throw error;
		}
		pending = pending.subarray(reader.offset);
		return true;
	};

	return {
		push(data) {
			const items: SubgroupStreamItem[] = [];
			if (failed) return { items };
			pending = pending.length === 0 ? data : concatBytes([pending, data]);
			try {
				// Each step consumes input or completes an Object, so this ends.
				for (;;) {
					const progressed =
						object === undefined
							? decodeFields(items)
							: collectPayload(object, items);
					if (!progressed) break;
				}
			} catch (error) {
				failed = true;
				pending = new Uint8Array(0);
				payloadChunks = [];
				if (error instanceof DecodeStop) {
					return { items, failure: error.failure };
				}
				if (error instanceof MoqtError) {
					return { items, failure: { scope: "session", error } };
				}
				throw error;
			}
			// Keeps the incomplete prefix, not the caller's buffer it is a view of.
			pending = pending.slice();
			return { items };
		},
		finish() {
			if (failed) return undefined;
			if (header === undefined) {
				return protocolViolation("FIN inside a SUBGROUP_HEADER");
			}
			if (object !== undefined || pending.length > 0) {
				return protocolViolation("FIN in the middle of a serialized Object");
			}
			return undefined;
		},
	};
}
