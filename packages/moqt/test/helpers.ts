import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

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

const FIXTURE_DIR = join(
	import.meta.dir,
	"..",
	"..",
	"..",
	"fixtures",
	"moqt-18",
);

/** The names `fixture` accepts: every `.hex` file of `fixtures/moqt-18/`. */
export function fixtureNames(): string[] {
	return readdirSync(FIXTURE_DIR)
		.filter((file) => file.endsWith(".hex"))
		.map((file) => file.slice(0, -".hex".length))
		.sort();
}

/**
 * The bytes of `fixtures/moqt-18/<name>.hex`, captured from moqtail, in the
 * form `hex` reads. The directory is shared with the Rust implementation; its
 * README records where each file comes from and what it decodes to.
 */
export function fixture(name: string): string {
	const path = join(FIXTURE_DIR, `${name}.hex`);
	const digits = readFileSync(path, "utf8").replace(/\s+/g, "");
	const pairs = digits.match(/../g);
	if (pairs === null || digits.length % 2 !== 0) {
		throw new Error(`${path} does not hold whole bytes`);
	}
	return pairs.join(" ");
}

/** SETUP of the moqtail relay 0.15.0. */
export const MOQTAIL_RELAY_SETUP = fixture("setup-moqtail-relay");
