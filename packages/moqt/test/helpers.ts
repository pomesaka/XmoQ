/** Parses hex written as in the moqtail relay log: byte pairs separated by spaces. */
export function hex(text: string): Uint8Array<ArrayBuffer> {
	const pairs = text.split(/\s+/).filter((pair) => pair.length > 0);
	return Uint8Array.from(pairs, (pair) => {
		if (!/^[0-9a-f]{2}$/.test(pair))
			throw new Error(`"${pair}" is not a hex byte`);
		return Number.parseInt(pair, 16);
	});
}

export function toHex(bytes: Uint8Array): string {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		" ",
	);
}

export function utf8(text: string): Uint8Array<ArrayBuffer> {
	return new TextEncoder().encode(text);
}

/** SETUP of the moqtail relay 0.15.0, one of the bytes in moqtail-fixtures.test.ts. */
export const MOQTAIL_RELAY_SETUP =
	"af 00 00 18 07 16 6d 6f 71 74 61 69 6c 2f 30 2e 31 35 2e 30 2b 64 64 38 36 32 33 64";
