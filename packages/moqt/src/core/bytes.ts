/** Bytes this package produces. The `ArrayBuffer` parameter makes them a `BufferSource`. */
export type Bytes = Uint8Array<ArrayBuffer>;

export const MAX_VARINT = 0xffff_ffff_ffff_ffffn;

/**
 * Thrown by a `ByteReader` when the input ends inside a field. Whether that is
 * "wait for more bytes" or a malformed message depends on the caller, so the
 * reader does not decide.
 */
export class ShortBufferError extends Error {
	override readonly name = "ShortBufferError";
}

export interface ByteReader {
	readonly offset: number;
	readonly remaining: number;
	uint8(): number;
	uint16(): number;
	/**
	 * Variable-length integer of §1.4.1. This is not the QUIC varint: the number
	 * of leading 1 bits of the first byte gives the count of bytes that follow.
	 * Non-minimal encodings are valid.
	 */
	varint(): bigint;
	/** The next `length` bytes as a view into the input, not a copy. */
	take(length: number): Uint8Array;
	/** A bounded reader over the next `length` bytes. */
	sub(length: number): ByteReader;
}

export function createByteReader(bytes: Uint8Array): ByteReader {
	let offset = 0;

	const take = (length: number): Uint8Array => {
		if (length > bytes.length - offset) throw new ShortBufferError();
		const view = bytes.subarray(offset, offset + length);
		offset += length;
		return view;
	};
	const uint8 = (): number => {
		const value = bytes[offset];
		if (value === undefined) throw new ShortBufferError();
		offset += 1;
		return value;
	};

	return {
		get offset() {
			return offset;
		},
		get remaining() {
			return bytes.length - offset;
		},
		uint8,
		uint16: () => uint8() * 0x100 + uint8(),
		varint: () => {
			const first = uint8();
			let following = 0;
			while (following < 8 && (first & (0x80 >> following)) !== 0) {
				following += 1;
			}
			// 0xff starts the 9-byte form: all 8 bits of the first byte are length
			// bits, so it carries no value bits.
			let value =
				following === 8 ? 0n : BigInt(first & (0xff >> (following + 1)));
			for (const byte of take(following)) value = (value << 8n) | BigInt(byte);
			return value;
		},
		take,
		sub: (length) => createByteReader(take(length)),
	};
}

/**
 * Bytes of each §1.4.1 encoding, shortest first, with the bound its values
 * stay below. `prefix` is the leading 1 bits of the first byte.
 */
const VARINT_FORMS = [
	{ valueBytes: 1, below: 1n << 7n, prefix: 0x00 },
	{ valueBytes: 2, below: 1n << 14n, prefix: 0x80 },
	{ valueBytes: 3, below: 1n << 21n, prefix: 0xc0 },
	{ valueBytes: 4, below: 1n << 28n, prefix: 0xe0 },
	{ valueBytes: 5, below: 1n << 35n, prefix: 0xf0 },
	{ valueBytes: 6, below: 1n << 42n, prefix: 0xf8 },
	{ valueBytes: 7, below: 1n << 49n, prefix: 0xfc },
	{ valueBytes: 8, below: 1n << 56n, prefix: 0xfe },
];

function encodeVarint(value: bigint): Bytes {
	if (value < 0n || value > MAX_VARINT) {
		throw new RangeError(`${value} does not fit a varint`);
	}
	const form = VARINT_FORMS.find((candidate) => value < candidate.below);
	// The 9-byte form: 0xff, then the value in 8 bytes.
	const out = new Uint8Array(form === undefined ? 9 : form.valueBytes);
	let rest = value;
	for (let i = out.length - 1; i >= 0; i--) {
		out[i] = Number(rest & 0xffn);
		rest >>= 8n;
	}
	// The value never reaches the prefix bits of its form, so OR keeps both.
	out[0] = (out[0] ?? 0) | (form === undefined ? 0xff : form.prefix);
	return out;
}

export interface ByteWriter {
	/** Bytes written so far. */
	readonly length: number;
	uint8(value: number): ByteWriter;
	uint16(value: number): ByteWriter;
	/** Writes the shortest encoding of §1.4.1. */
	varint(value: bigint | number): ByteWriter;
	/** Appends `value` without copying it; the caller must not change it before `finish()`. */
	bytes(value: Uint8Array): ByteWriter;
	finish(): Bytes;
}

export function createByteWriter(): ByteWriter {
	const chunks: Uint8Array[] = [];
	let length = 0;

	const writer: ByteWriter = {
		get length() {
			return length;
		},
		uint8(value) {
			if (!Number.isInteger(value) || value < 0 || value > 0xff) {
				throw new RangeError(`${value} is not a uint8`);
			}
			return writer.bytes(Uint8Array.of(value));
		},
		uint16(value) {
			if (!Number.isInteger(value) || value < 0 || value > 0xffff) {
				throw new RangeError(`${value} is not a uint16`);
			}
			return writer.bytes(Uint8Array.of(value >> 8, value & 0xff));
		},
		varint: (value) => writer.bytes(encodeVarint(BigInt(value))),
		bytes(value) {
			chunks.push(value);
			length += value.length;
			return writer;
		},
		finish: () => concatBytes(chunks),
	};
	return writer;
}

export function concatBytes(parts: readonly Uint8Array[]): Bytes {
	const total = parts.reduce((sum, part) => sum + part.length, 0);
	const out = new Uint8Array(total);
	let offset = 0;
	for (const part of parts) {
		out.set(part, offset);
		offset += part.length;
	}
	return out;
}
