import type { ByteReader } from "./bytes.ts";
import { protocolViolation } from "./error.ts";

/**
 * Reads a varint used as a byte length or an element count. `max` bounds what
 * the caller will allocate; exceeding it is a PROTOCOL_VIOLATION.
 */
export function readLength(
	reader: ByteReader,
	max: number,
	what: string,
): number {
	const value = reader.varint();
	if (value > BigInt(max)) {
		throw protocolViolation(`${what} ${value} exceeds ${max}`);
	}
	return Number(value);
}

/** A set of names that each have one value on the wire. */
export interface WireEnum<Name extends string> {
	toWire(name: Name): bigint;
	fromWire(value: bigint): Name | undefined;
}

export function wireEnum<Name extends string>(
	values: Record<Name, bigint>,
): WireEnum<Name> {
	const names = new Map<bigint, Name>();
	for (const name in values) names.set(values[name], name);
	return {
		toWire: (name) => values[name],
		fromWire: (value) => names.get(value),
	};
}
