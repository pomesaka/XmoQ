// The unidirectional stream that carries the Objects of a FETCH (§11.4.4):
// a FETCH_HEADER, then Objects and End of Range indicators.
//
// An Object on such a stream says less than one on a subgroup stream where
// it can: each field may be left out and taken from the Object before it. So
// the stream is decoded and encoded in order, with that Object in mind.
//
// The names and the readings follow the Rust implementation
// (crates/moqt/src/fetch.rs); where the draft leaves a choice, the two make
// the same one (fixtures/moqt-18-verdicts/fetch-stream.tsv and
// fixtures/moqt-18-by-hand/, which both read; what is provisional is under
// "Other observations about the draft" in docs/moqt-implementation-notes.md).
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
import { encodeKeyValuePairs, type KeyValuePair } from "./key-value-pair.ts";
import {
	decodeLocation,
	encodeLocation,
	type GroupOrder,
	type Location,
} from "./parameters.ts";
import {
	checkObjectProperties,
	DecodeStop,
	decodePropertiesBlock,
	joinChunks,
	type ObjectStreamFailure,
	readObjectFieldLength,
	refuseObjectPropertiesThatBreakTheTrack,
	type SubgroupStreamLimits,
} from "./subgroup.ts";
import { wireEnum } from "./wire.ts";

/** Stream Type of a fetch stream (§3.4, Table 3). */
export const FETCH_HEADER_STREAM_TYPE = 0x05n;

/** §11.4.4.1, Table 8: how the Subgroup ID is given. */
const SUBGROUP_MODE_MASK = 0x03n;
const SUBGROUP_ZERO = 0x00n;
const SUBGROUP_OF_THE_PRIOR = 0x01n;
const SUBGROUP_OF_THE_PRIOR_PLUS_ONE = 0x02n;
const SUBGROUP_PRESENT = 0x03n;
/** §11.4.4.1, Table 9. */
const OBJECT_ID_PRESENT = 0x04n;
const GROUP_ID_PRESENT = 0x08n;
const PRIORITY_PRESENT = 0x10n;
const PROPERTIES_PRESENT = 0x20n;
const DATAGRAM = 0x40n;
/** §11.4.4: "When less than 128, the bits represent flags". */
const FIRST_VALUE_THAT_IS_NO_FLAGS = 0x80n;

/** FETCH_HEADER (§11.4.4). */
export interface FetchHeader {
	/** The Request ID of the FETCH the stream answers. */
	requestId: bigint;
}

/** One Object as a fetch stream gives it. A fetch stream has no Object Status field. */
export interface FetchObject {
	groupId: bigint;
	/**
	 * `undefined` for an Object whose Forwarding Preference is Datagram, which
	 * has no Subgroup ID (§11.4.4.1).
	 */
	subgroupId: bigint | undefined;
	objectId: bigint;
	publisherPriority: number;
	properties: KeyValuePair[];
	payload: Uint8Array;
}

/**
 * What an End of Range indicator says of the Objects up to its Location
 * (§11.4.4.2): 0x8C, they do not exist; 0x10C, the sender does not know.
 */
export type RangeEnd = "NonExistent" | "Unknown";

const RANGE_ENDS = wireEnum<RangeEnd>({
	NonExistent: 0x8cn,
	Unknown: 0x10cn,
});

/** What a fetch stream yields, in order: its header once, then Objects and End of Range indicators. */
export type FetchStreamItem =
	| { type: "header"; header: FetchHeader }
	| { type: "object"; object: FetchObject }
	/**
	 * "All Objects with Locations between the last serialized Object, if any,
	 * and this Location, inclusive, either do not exist [...] or are unknown"
	 * (§11.4.4.2).
	 */
	| { type: "end-of-range"; kind: RangeEnd; location: Location };

export interface FetchStreamResult {
	items: FetchStreamItem[];
	/** Set when decoding stopped. Items before it are valid. */
	failure?: ObjectStreamFailure;
}

/** What the fields that an Object leaves out are taken from. */
interface Prior {
	/**
	 * Of the Object or the End of Range indicator before: "Prior Group ID and
	 * prior Object ID: The values from the End of Range indicator"
	 * (§11.4.4.2).
	 */
	location: Location;
	/**
	 * The Subgroup ID and the Publisher Priority of the last Object, which an
	 * indicator has none of. `undefined` before the first Object.
	 */
	object:
		| { subgroupId: bigint | undefined; publisherPriority: number }
		| undefined;
}

function afterObject(object: FetchObject): Prior {
	return {
		location: { group: object.groupId, object: object.objectId },
		object: {
			subgroupId: object.subgroupId,
			publisherPriority: object.publisherPriority,
		},
	};
}

/** `before` is what was prior to the indicator. */
function afterRangeEnd(before: Prior | undefined, location: Location): Prior {
	return { location, object: before?.object };
}

/** The Group ID a Group ID Delta stands for after `prior` (§11.4.4.1), or `undefined` past either end. */
function groupAfter(
	prior: bigint,
	delta: bigint,
	groupOrder: GroupOrder,
): bigint | undefined {
	const group =
		groupOrder === "Ascending" ? prior + delta + 1n : prior - delta - 1n;
	return group < 0n || group > MAX_VARINT ? undefined : group;
}

/** What is known of an Object before its payload has arrived. */
interface PendingObject {
	object: Omit<FetchObject, "payload" | "properties">;
	/** The Object Properties as they came; empty where the Object has none. */
	encodedProperties: Uint8Array;
	payloadLength: number;
}

export interface FetchStreamDecoder {
	push(data: Uint8Array): FetchStreamResult;
	/**
	 * Call when the stream ends with a FIN. A FIN in the middle of a
	 * serialized Object is a PROTOCOL_VIOLATION, as for a subgroup stream
	 * (§11.4).
	 */
	finish(): MoqtError | undefined;
}

/**
 * Incremental decoder for one fetch stream: feed it the stream bytes as they
 * arrive, starting with the Stream Type.
 *
 * `groupOrder` is the one the FETCH asked for, Ascending where it named none
 * (§10.2.8): a Group ID Delta counts upwards or downwards by it.
 *
 * Memory is bounded by the limits, as for a subgroup stream.
 *
 * A failure of scope "track" (Properties that break the track) or "stream"
 * (an Object over a limit) ends this stream and leaves the session open.
 * Anything else is a protocol error that closes the session: Serialization
 * Flags the draft does not define, a field taken from an Object that was not
 * there, an ID past 64 bits.
 */
export function createFetchStreamDecoder(
	limits: SubgroupStreamLimits,
	groupOrder: GroupOrder,
): FetchStreamDecoder {
	/** Bytes of an incomplete header, Object prefix or indicator. */
	let pending: Uint8Array = new Uint8Array(0);
	let header: FetchHeader | undefined;
	let prior: Prior | undefined;
	let object: PendingObject | undefined;
	let payloadChunks: Uint8Array[] = [];
	let payloadReceived = 0;
	let failed = false;

	const decodeHeader = (reader: ByteReader): FetchHeader => {
		if (reader.varint() !== FETCH_HEADER_STREAM_TYPE) {
			throw protocolViolation("not a fetch stream");
		}
		return { requestId: reader.varint() };
	};

	/** The fields of Figure 27 up to, not including, Object Payload. */
	const decodeObjectPrefix = (
		flags: bigint,
		reader: ByteReader,
	): PendingObject => {
		// §11.4.4.1: "If the first Object in the FETCH response uses a flag
		// that references fields in the prior Object, the Subscriber MUST close
		// the session with a PROTOCOL_VIOLATION."
		const groupPresent = (flags & GROUP_ID_PRESENT) !== 0n;

		let groupId: bigint;
		if (groupPresent) {
			const value = reader.varint();
			// The first Object gives the Group ID itself. After an End of Range
			// indicator the value is a delta from the indicator's Group, as
			// after an Object (§11.4.4.2), also for the first Object.
			const group =
				prior === undefined
					? value
					: groupAfter(prior.location.group, value, groupOrder);
			if (group === undefined) throw protocolViolation("Group ID out of range");
			groupId = group;
		} else {
			if (prior === undefined) throw protocolViolation("no prior Group ID");
			groupId = prior.location.group;
		}

		const priorSubgroup = (): bigint => {
			const subgroupId = prior?.object?.subgroupId;
			// WHY also where the Object before was a datagram, which has no
			// Subgroup ID to take: the draft gives no value for it, and the Rust
			// implementation closes the session here as well.
			if (subgroupId === undefined) {
				throw protocolViolation("no prior Subgroup ID");
			}
			return subgroupId;
		};
		// §11.4.4.1: with 0x40 "the subscriber MUST ignore the bits".
		let subgroupId: bigint | undefined;
		if ((flags & DATAGRAM) === 0n) {
			switch (flags & SUBGROUP_MODE_MASK) {
				case SUBGROUP_ZERO:
					subgroupId = 0n;
					break;
				case SUBGROUP_OF_THE_PRIOR:
					subgroupId = priorSubgroup();
					break;
				case SUBGROUP_OF_THE_PRIOR_PLUS_ONE:
					subgroupId = priorSubgroup() + 1n;
					if (subgroupId > MAX_VARINT) {
						throw protocolViolation("Subgroup ID exceeds 2^64 - 1");
					}
					break;
				default:
					subgroupId = reader.varint();
			}
		}

		let objectId: bigint;
		if ((flags & OBJECT_ID_PRESENT) !== 0n) {
			const value = reader.varint();
			// "When the Group ID Delta field is present, the Object ID is the
			// value of Object ID Delta if present."
			objectId =
				prior === undefined || groupPresent
					? value
					: prior.location.object + value;
		} else {
			if (prior === undefined) throw protocolViolation("no prior Object ID");
			// "the prior Object's ID plus one, regardless of which group it
			// belongs to".
			objectId = prior.location.object + 1n;
		}
		if (objectId > MAX_VARINT) {
			throw protocolViolation("Object ID exceeds 2^64 - 1");
		}

		let publisherPriority: number;
		if ((flags & PRIORITY_PRESENT) !== 0n) {
			publisherPriority = reader.uint8();
		} else {
			if (prior?.object === undefined) {
				throw protocolViolation("no prior Publisher Priority");
			}
			publisherPriority = prior.object.publisherPriority;
		}

		// The Properties are taken as bytes here and decoded when the payload
		// has arrived, as the Rust implementation does: what is wrong with them
		// is then reported at the same point of the stream by both, and an
		// Object Payload Length over the limit, which follows them, is refused
		// before them.
		let encodedProperties: Uint8Array = new Uint8Array(0);
		if ((flags & PROPERTIES_PRESENT) !== 0n) {
			const length = readObjectFieldLength(
				reader,
				limits.maxObjectPropertiesLength,
				"Object Properties Length",
			);
			// Copied: the reader is over a buffer the caller can reuse.
			encodedProperties = reader.take(length).slice();
		}

		const payloadLength = readObjectFieldLength(
			reader,
			limits.maxObjectPayloadLength,
			"Object Payload Length",
		);
		return {
			object: { groupId, subgroupId, objectId, publisherPriority },
			encodedProperties,
			payloadLength,
		};
	};

	/** Collects payload bytes; emits the Object once it is whole. Returns false when it needs more input. */
	const collectPayload = (
		current: PendingObject,
		items: FetchStreamItem[],
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
		const properties = decodePropertiesBlock(
			createByteReader(current.encodedProperties),
		);
		checkObjectProperties(
			properties,
			current.object.groupId,
			current.object.objectId,
		);
		const whole = {
			...current.object,
			properties,
			payload: joinChunks(payloadChunks),
		};
		items.push({ type: "object", object: whole });
		prior = afterObject(whole);
		object = undefined;
		payloadChunks = [];
		payloadReceived = 0;
		return true;
	};

	/** Decodes the header, one indicator or one Object prefix. Returns false when it needs more input. */
	const decodeFields = (items: FetchStreamItem[]): boolean => {
		if (pending.length === 0) return false;
		const reader = createByteReader(pending);
		try {
			if (header === undefined) {
				header = decodeHeader(reader);
				items.push({ type: "header", header });
			} else {
				const flags = reader.varint();
				if (flags >= FIRST_VALUE_THAT_IS_NO_FLAGS) {
					const kind = RANGE_ENDS.fromWire(flags);
					// §11.4.4: "Any other value is a PROTOCOL_VIOLATION."
					if (kind === undefined) {
						throw protocolViolation(
							`unknown Serialization Flags 0x${flags.toString(16)}`,
						);
					}
					// WHY read as a Location and not as deltas, though the figure
					// of §11.4.4 names the fields Group ID Delta and Object ID
					// Delta: §11.4.4.2 has "the Group ID and Object ID fields are
					// present" and calls what they give "this Location".
					// WHY nothing is read after them, though the figure has an
					// Object Payload Length for every entry: an indicator stands
					// for Objects that are not there, and neither the Rust
					// implementation nor the moqtail crate reads one.
					const location = decodeLocation(reader);
					items.push({ type: "end-of-range", kind, location });
					prior = afterRangeEnd(prior, location);
				} else {
					object = decodeObjectPrefix(flags, reader);
				}
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
			const items: FetchStreamItem[] = [];
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
			// A fetch without Objects still has "the FETCH_HEADER" before the
			// FIN (§10.12.3).
			if (header === undefined) {
				return protocolViolation("FIN before the FETCH_HEADER");
			}
			if (object !== undefined || pending.length > 0) {
				return protocolViolation("FIN in the middle of a serialized Object");
			}
			return undefined;
		},
	};
}

export interface FetchStreamEncoder {
	/** The Stream Type and the FETCH_HEADER: the first bytes of the stream. */
	header: Bytes;
	/**
	 * The bytes of the next Object. Throws a `RangeError`, encoding nothing,
	 * when the Object does not come after what was encoded before it (an
	 * earlier Group by the Group Order, or the same Group and no larger
	 * Object ID), and when its properties break the track (§2.5.1, §12.8,
	 * §12.9), as the decoder would find.
	 */
	object(object: FetchObject): Bytes;
	/**
	 * The bytes of an End of Range indicator: the Objects after what was
	 * encoded before, up to and with `location`, do not exist or are not
	 * known. Throws a `RangeError`, encoding nothing, for a Location that
	 * goes back: an earlier Group by the Group Order, or the same Group and
	 * no larger Object ID.
	 */
	endOfRange(kind: RangeEnd, location: Location): Bytes;
}

/**
 * Encodes one fetch stream: the header, then Objects and End of Range
 * indicators in the order of the fetch.
 *
 * A field is left out where it can be: the Group ID is there for the first
 * Object and for a new Group; the Object ID unless it is the prior one plus
 * one in the same Group; the priority when it changes; the Subgroup ID when
 * it is neither 0 nor the prior one nor that plus one. WHY these flags and
 * no others, where several encodings give the same Object: the moqtail crate
 * 0.15.0 and the Rust implementation leave a field out in exactly these
 * places, so the bytes they wrote can be compared with what this writes.
 *
 * An End of Range indicator is refused like an Object where it certainly
 * goes back: a Group against the Group Order, or the Group of what came
 * before it and no larger Object ID. WHY refused: the fields of the next
 * Object are written against the indicator's Location, so an indicator that
 * went back would take the encoder back with it, and an Object that goes
 * back would be written after it. WHY NOT more than that: §11.4.4.2 has an
 * indicator cover what lies "between the last serialized Object, if any,
 * and this Location" and does not say which Location of a Group that comes
 * later in a Descending fetch an indicator is to name, so within such a
 * Group any Object ID is taken.
 */
export function createFetchStreamEncoder(
	header: FetchHeader,
	groupOrder: GroupOrder,
): FetchStreamEncoder {
	let prior: Prior | undefined;

	/** The Group ID Delta for a Group that comes after the prior one, or `undefined`. */
	const groupDelta = (
		priorGroup: bigint,
		groupId: bigint,
	): bigint | undefined => {
		const apart =
			groupOrder === "Ascending" ? groupId - priorGroup : priorGroup - groupId;
		return apart >= 1n ? apart - 1n : undefined;
	};

	return {
		header: createByteWriter()
			.varint(FETCH_HEADER_STREAM_TYPE)
			.varint(header.requestId)
			.finish(),
		object(object) {
			refuseObjectPropertiesThatBreakTheTrack(
				object.properties,
				object.groupId,
				object.objectId,
			);
			let flags = 0n;
			const fields = createByteWriter();

			const sameGroup = prior?.location.group === object.groupId;
			if (prior === undefined) {
				flags |= GROUP_ID_PRESENT;
				fields.varint(object.groupId);
			} else if (!sameGroup) {
				const delta = groupDelta(prior.location.group, object.groupId);
				if (delta === undefined) {
					throw new RangeError("Groups must follow the Group Order");
				}
				flags |= GROUP_ID_PRESENT;
				fields.varint(delta);
			}

			const priorSubgroup = prior?.object?.subgroupId;
			if (object.subgroupId === undefined) {
				flags |= DATAGRAM;
			} else if (object.subgroupId === 0n) {
				flags |= SUBGROUP_ZERO;
			} else if (object.subgroupId === priorSubgroup) {
				flags |= SUBGROUP_OF_THE_PRIOR;
			} else if (
				priorSubgroup !== undefined &&
				object.subgroupId === priorSubgroup + 1n
			) {
				flags |= SUBGROUP_OF_THE_PRIOR_PLUS_ONE;
			} else {
				flags |= SUBGROUP_PRESENT;
				fields.varint(object.subgroupId);
			}

			if (prior !== undefined && sameGroup) {
				const ahead = object.objectId - prior.location.object;
				if (ahead < 1n) {
					throw new RangeError("Object IDs must increase within a Group");
				}
				if (ahead > 1n) {
					flags |= OBJECT_ID_PRESENT;
					fields.varint(ahead);
				}
			} else {
				flags |= OBJECT_ID_PRESENT;
				fields.varint(object.objectId);
			}

			if (prior?.object?.publisherPriority !== object.publisherPriority) {
				flags |= PRIORITY_PRESENT;
				fields.uint8(object.publisherPriority);
			}
			if (object.properties.length > 0) {
				flags |= PROPERTIES_PRESENT;
				const properties = createByteWriter();
				encodeKeyValuePairs(properties, object.properties);
				fields.varint(properties.length).bytes(properties.finish());
			}
			fields.varint(object.payload.length).bytes(object.payload);

			prior = afterObject(object);
			return createByteWriter().varint(flags).bytes(fields.finish()).finish();
		},
		endOfRange(kind, location) {
			if (prior !== undefined) {
				if (prior.location.group === location.group) {
					if (location.object <= prior.location.object) {
						throw new RangeError("Object IDs must increase within a Group");
					}
				} else if (
					groupDelta(prior.location.group, location.group) === undefined
				) {
					throw new RangeError("Groups must follow the Group Order");
				}
			}
			const writer = createByteWriter().varint(RANGE_ENDS.toWire(kind));
			encodeLocation(writer, location);
			prior = afterRangeEnd(prior, location);
			return writer.finish();
		},
	};
}
