import type { ByteReader, ByteWriter } from "./bytes.ts";
import { protocolViolation } from "./error.ts";
import { readLength } from "./wire.ts";

/** Track Namespace of §2.4.1: an ordered list of Track Namespace Fields. */
export type TrackNamespace = readonly Uint8Array[];

/**
 * Full Track Name of §2.4.1. Messages carry its two parts as separate fields,
 * as the message formats of the draft do.
 */
export interface FullTrackName {
	trackNamespace: TrackNamespace;
	trackName: Uint8Array;
}

const MAX_TRACK_NAMESPACE_FIELDS = 32;
/** §2.4.1: maximum length of a Track Namespace and of a Full Track Name. */
const MAX_FULL_TRACK_NAME_LENGTH = 4096;

/** Returns the namespace and the sum of its Track Namespace Field Lengths. */
function decodeNamespaceWithLength(reader: ByteReader): {
	trackNamespace: TrackNamespace;
	length: number;
} {
	const count = readLength(
		reader,
		MAX_TRACK_NAMESPACE_FIELDS,
		"Number of Track Namespace Fields",
	);
	const trackNamespace: Uint8Array[] = [];
	let length = 0;
	for (let i = 0; i < count; i++) {
		const fieldLength = readLength(
			reader,
			MAX_FULL_TRACK_NAME_LENGTH - length,
			"Track Namespace length",
		);
		if (fieldLength === 0) {
			throw protocolViolation("Track Namespace Field Length is 0");
		}
		length += fieldLength;
		trackNamespace.push(reader.take(fieldLength).slice());
	}
	return { trackNamespace, length };
}

export function decodeTrackNamespace(reader: ByteReader): TrackNamespace {
	return decodeNamespaceWithLength(reader).trackNamespace;
}

/** Decodes Track Namespace, Track Name Length and Track Name, as SUBSCRIBE and PUBLISH carry them. */
export function decodeFullTrackName(reader: ByteReader): FullTrackName {
	const { trackNamespace, length } = decodeNamespaceWithLength(reader);
	const nameLength = readLength(
		reader,
		MAX_FULL_TRACK_NAME_LENGTH - length,
		"Full Track Name length",
	);
	return { trackNamespace, trackName: reader.take(nameLength).slice() };
}

function checkedNamespaceLength(trackNamespace: TrackNamespace): number {
	if (trackNamespace.length > MAX_TRACK_NAMESPACE_FIELDS) {
		throw new RangeError(
			`Track Namespace has more than ${MAX_TRACK_NAMESPACE_FIELDS} fields`,
		);
	}
	let length = 0;
	for (const field of trackNamespace) {
		if (field.length === 0) {
			throw new RangeError("Track Namespace Field is empty");
		}
		length += field.length;
	}
	if (length > MAX_FULL_TRACK_NAME_LENGTH) {
		throw new RangeError(
			`Track Namespace exceeds ${MAX_FULL_TRACK_NAME_LENGTH} bytes`,
		);
	}
	return length;
}

function writeNamespace(
	writer: ByteWriter,
	trackNamespace: TrackNamespace,
): void {
	writer.varint(trackNamespace.length);
	for (const field of trackNamespace) writer.varint(field.length).bytes(field);
}

export function encodeTrackNamespace(
	writer: ByteWriter,
	trackNamespace: TrackNamespace,
): void {
	checkedNamespaceLength(trackNamespace);
	writeNamespace(writer, trackNamespace);
}

export function encodeFullTrackName(
	writer: ByteWriter,
	{ trackNamespace, trackName }: FullTrackName,
): void {
	const length = checkedNamespaceLength(trackNamespace) + trackName.length;
	if (length > MAX_FULL_TRACK_NAME_LENGTH) {
		throw new RangeError(
			`Full Track Name exceeds ${MAX_FULL_TRACK_NAME_LENGTH} bytes`,
		);
	}
	writeNamespace(writer, trackNamespace);
	writer.varint(trackName.length).bytes(trackName);
}
