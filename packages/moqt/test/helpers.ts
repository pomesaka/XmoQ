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

/** One row of `fixtures/webtransport-close/reasons.tsv`. */
export interface CloseReasonFixture {
	name: string;
	/** The reason an application gives. */
	reason: string;
	/** The reason as Chromium sent it, in the form `hex` reads. */
	sentHex: string;
}

function spaced(digits: string): string {
	return (digits.match(/../g) ?? []).join(" ");
}

/** Whole bytes in lower-case hexadecimal digits, or nothing at all. */
const HEX_BYTES = /^(?:[0-9a-f]{2})*$/;

/**
 * One row of `reasons.tsv`. Throws for a row that is not what the README of
 * the fixture describes: a row read in another way than it was written would
 * pin nothing.
 */
export function parseCloseReasonFixture(line: string): CloseReasonFixture {
	const refuse = (what: string): never => {
		throw new Error(`a row of the close reason fixture ${what}: ${line}`);
	};
	const columns = line.split("\t");
	const [name, encoding, inputHex, sentHex] = columns;
	if (
		columns.length !== 4 ||
		name === undefined ||
		inputHex === undefined ||
		sentHex === undefined
	) {
		return refuse("does not have four columns");
	}
	if (name === "") return refuse("has no name");
	if (encoding !== "utf8" && encoding !== "utf16") {
		return refuse("has an encoding that is neither utf8 nor utf16");
	}
	if (!HEX_BYTES.test(inputHex) || !HEX_BYTES.test(sentHex)) {
		return refuse("has a column that is not whole bytes in hexadecimal");
	}
	const bytes = hex(spaced(inputHex));
	if (encoding === "utf8") {
		return {
			name,
			reason: new TextDecoder("utf-8", {
				ignoreBOM: true,
				fatal: true,
			}).decode(bytes),
			sentHex: spaced(sentHex),
		};
	}
	if (bytes.length % 2 !== 0) {
		return refuse("has half of a UTF-16 code unit");
	}
	// UTF-16 code units, high byte first: a reason that is not valid text.
	const units = Array.from(
		{ length: bytes.length / 2 },
		(_unit, index) =>
			((bytes[index * 2] ?? 0) << 8) | (bytes[index * 2 + 1] ?? 0),
	);
	return {
		name,
		reason: String.fromCharCode(...units),
		sentHex: spaced(sentHex),
	};
}

/**
 * The rows of `fixtures/webtransport-close/reasons.tsv`: reasons of a session
 * close and the bytes Chromium sent for each. The file is shared with the
 * Rust implementation; its README records where the bytes come from.
 */
export function closeReasonFixtures(): CloseReasonFixture[] {
	const path = join(FIXTURE_DIR, "..", "webtransport-close", "reasons.tsv");
	const table = readFileSync(path, "utf8");
	// WHY not filter empty lines away: every row ends with one line feed, the
	// last one too, and the Rust side reads the table the same way. A table
	// that one side reads and the other refuses would be pinned by one of
	// them only.
	if (!table.endsWith("\n")) {
		throw new Error(`${path}: the last row does not end with a line feed`);
	}
	const rows = table.slice(0, -1).split("\n").map(parseCloseReasonFixture);
	// WHY: `test.each` passes with no rows, so an emptied file, or another
	// file at this path, would look like a fixture that holds.
	if (rows.length === 0) throw new Error(`${path} has no rows`);
	return rows;
}
