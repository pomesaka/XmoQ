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

const FIXTURES = join(import.meta.dir, "..", "..", "..", "fixtures");
const FIXTURE_DIR = join(FIXTURES, "moqt-18");
const BY_HAND_DIR = join(FIXTURES, "moqt-18-by-hand");

function hexFileNames(directory: string): string[] {
	return readdirSync(directory)
		.filter((file) => file.endsWith(".hex"))
		.map((file) => file.slice(0, -".hex".length))
		.sort();
}

function hexFile(directory: string, name: string): string {
	const path = join(directory, `${name}.hex`);
	const digits = readFileSync(path, "utf8").replace(/\s+/g, "");
	const pairs = digits.match(/../g);
	if (pairs === null || digits.length % 2 !== 0) {
		throw new Error(`${path} does not hold whole bytes`);
	}
	return pairs.join(" ");
}

const FROM_LIBRARIES_DIR = join(FIXTURES, "moqt-18-from-libraries");

/** Every `.hex` file of `fixtures/moqt-18-from-libraries/`. */
export function fromALibraryNames(): string[] {
	return hexFileNames(FROM_LIBRARIES_DIR);
}

/**
 * The bytes of `fixtures/moqt-18-from-libraries/<name>.hex`: what a peer's
 * library serialized in a script, and no wire carried. The Rust
 * implementation reads the same files.
 */
export function fromALibrary(name: string): string {
	return hexFile(FROM_LIBRARIES_DIR, name);
}

/** The names `fixture` accepts: every `.hex` file of `fixtures/moqt-18/`. */
export function fixtureNames(): string[] {
	return hexFileNames(FIXTURE_DIR);
}

/**
 * The files of `fixtures/moqt-18-by-hand/` that are one control message.
 * The others are fetch streams, which test/core/fetch.test.ts has a table
 * of.
 */
export const BY_HAND_CONTROL_MESSAGES = [
	"request-update-request-id-3",
	"request-update-request-id-5",
] as const;

/** The names `byHand` accepts: every `.hex` file of `fixtures/moqt-18-by-hand/`. */
export function byHandNames(): string[] {
	return hexFileNames(BY_HAND_DIR);
}

/**
 * The bytes of `fixtures/moqt-18-by-hand/<name>.hex`, in the form `hex`
 * reads. Nobody's implementation wrote them: they were written by hand from
 * the draft, and the Rust implementation reads the same files. They check
 * that the two read the same bytes as the same values, and nothing against
 * another implementation (the README there).
 */
export function byHand(name: string): string {
	return hexFile(BY_HAND_DIR, name);
}

/**
 * The bytes of `fixtures/moqt-18/<name>.hex`, captured from another
 * implementation (moqtail, and one file from moqlivemock), in the form `hex`
 * reads. The directory is shared with the Rust implementation; its
 * README records where each file comes from and what it decodes to.
 */
export function fixture(name: string): string {
	return hexFile(FIXTURE_DIR, name);
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

/** One row of `fixtures/first-bytes/said.tsv`. */
export interface FirstBytesFixture {
	name: string;
	kind: "uni" | "bidi";
	/** Every byte the stream has sent when its time runs out, in the form `hex` reads. */
	sentHex: string;
	/** Whether the stream has said what it is with those bytes. */
	said: boolean;
}

/**
 * The rows of `fixtures/first-bytes/said.tsv`: what a stream of the peer has
 * sent when its time runs out, and whether it has said what it is. The file
 * is shared with the Rust implementation; its README has what the rows hold
 * a reader to and what a test has to set up.
 */
export function firstBytesFixtures(): FirstBytesFixture[] {
	const path = join(FIXTURES, "first-bytes", "said.tsv");
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n")) {
		throw new Error(`${path}: the last row does not end with a line feed`);
	}
	const rows = table
		.slice(0, -1)
		.split("\n")
		.map((line): FirstBytesFixture => {
			const refuse = (what: string): never => {
				throw new Error(`a row of the first bytes fixture ${what}: ${line}`);
			};
			const columns = line.split("\t");
			const [name, kind, sent, said] = columns;
			if (columns.length !== 4 || name === undefined || sent === undefined) {
				return refuse("does not have four columns");
			}
			if (name === "") return refuse("has no name");
			if (kind !== "uni" && kind !== "bidi") {
				return refuse("has a kind that is neither uni nor bidi");
			}
			if (!HEX_BYTES.test(sent)) {
				return refuse("has bytes that are not whole bytes in hexadecimal");
			}
			if (said !== "said" && said !== "not-yet") {
				return refuse("says neither said nor not-yet");
			}
			return { name, kind, sentHex: spaced(sent), said: said === "said" };
		});
	// WHY: `test.each` passes with no rows, so an emptied file would look
	// like a fixture that holds.
	if (rows.length === 0) throw new Error(`${path} has no rows`);
	return rows;
}
