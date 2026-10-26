import { type ByteReader, type ByteWriter, MAX_VARINT } from "./bytes.ts";
import { protocolViolation } from "./error.ts";
import { readLength } from "./wire.ts";

/**
 * Key-Value-Pair of §1.4.3 with the Type already resolved from its delta.
 * `value` is a varint when Type is even and a byte sequence when Type is odd.
 */
export interface KeyValuePair {
	type: bigint;
	value: bigint | Uint8Array;
}

/** §1.4.3: "The maximum length of a value is 2^16-1 bytes." */
export const MAX_VALUE_LENGTH = 0xffff;

/**
 * Most pairs accepted in one sequence. Provisional: the draft sets no
 * maximum. A pair can arrive in two bytes and becomes an object many times
 * that size, so a byte limit alone does not bound what a sequence costs. The
 * Rust implementation uses the same number, so that neither accepts input the
 * other rejects.
 */
export const MAX_KEY_VALUE_PAIRS = 64;

/**
 * A list has more entries than this implementation accepts. The peer broke no
 * rule of the draft, so the caller decides how far the damage reaches.
 */
export class TooManyEntriesError extends Error {
	override readonly name = "TooManyEntriesError";

	constructor(what: string) {
		super(`more than ${MAX_KEY_VALUE_PAIRS} ${what}`);
	}
}

/** §1.4.3: an even Type carries a varint, an odd Type a length and bytes. */
function hasVarintValue(type: bigint): boolean {
	return type % 2n === 0n;
}

function compareBigint(a: bigint, b: bigint): number {
	if (a < b) return -1;
	if (a > b) return 1;
	return 0;
}

/**
 * Decodes Key-Value-Pairs until `reader` is exhausted. The caller bounds the
 * reader, because the byte length of the sequence comes from the context
 * (§1.4.3).
 */
export function decodeKeyValuePairs(reader: ByteReader): KeyValuePair[] {
	const pairs: KeyValuePair[] = [];
	let type = 0n;
	while (reader.remaining > 0) {
		if (pairs.length === MAX_KEY_VALUE_PAIRS) {
			throw new TooManyEntriesError("Key-Value-Pairs in one sequence");
		}
		type += reader.varint();
		if (type > MAX_VARINT) {
			throw protocolViolation("Key-Value-Pair Type exceeds 2^64 - 1");
		}
		if (hasVarintValue(type)) {
			pairs.push({ type, value: reader.varint() });
		} else {
			const length = readLength(
				reader,
				MAX_VALUE_LENGTH,
				"Key-Value-Pair Length",
			);
			pairs.push({ type, value: reader.take(length).slice() });
		}
	}
	return pairs;
}

/** Writes the pairs in ascending Type order, which the delta encoding requires. */
export function encodeKeyValuePairs(
	writer: ByteWriter,
	pairs: readonly KeyValuePair[],
): void {
	// Array.prototype.sort is stable, so pairs of the same Type keep their order.
	const sorted = [...pairs].sort((a, b) => compareBigint(a.type, b.type));
	let previous = 0n;
	for (const { type, value } of sorted) {
		writer.varint(type - previous);
		previous = type;
		if (typeof value === "bigint") {
			if (!hasVarintValue(type)) {
				throw new RangeError(`odd Type ${type} needs a byte value`);
			}
			writer.varint(value);
		} else {
			if (hasVarintValue(type)) {
				throw new RangeError(`even Type ${type} needs a varint value`);
			}
			if (value.length > MAX_VALUE_LENGTH) {
				throw new RangeError(
					`value of Type ${type} exceeds ${MAX_VALUE_LENGTH} bytes`,
				);
			}
			writer.varint(value.length).bytes(value);
		}
	}
}

/** §2.5.1: Property types in this range are Mandatory Track Properties. */
export function isMandatoryTrackProperty(type: bigint): boolean {
	return type >= 0x4000n && type <= 0x7fffn;
}
