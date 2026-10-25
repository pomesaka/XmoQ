// What a line says, as a participant sends it on `said` and as the gatherer
// sends it on `lines` (the payloads decided on 2026-10-24). The Rust side is
// `crates/chat-tracks/src/payloads.rs`; both read the shared rows of
// `fixtures/chat-tracks/` in their tests.
//
// `said`, one Object a Group: `{"text": string, "sentAt": number}`.
// `lines`: `{"name": string, "text": string, "sentAt": number, "at": number}`,
// in that order. `sentAt` is the sender's clock and `at` the gatherer's, in
// milliseconds since the Unix epoch.

import { type JsonValue, readJson } from "@xmoq/json";

/**
 * The largest time taken: 2^53 - 1, the largest integer a JavaScript number
 * holds exactly, as on the Rust side (`MAX_SENT_AT`).
 */
export const MAX_TIME = Number.MAX_SAFE_INTEGER;

/** Why a payload of `said` or `lines` is no line: the words of `Malformed::word` on the Rust side. */
export type Malformed =
	/** Not UTF-8, not JSON, or no JSON object. */
	| "not-an-object"
	/** `text` is missing or no string. */
	| "text"
	/** `sentAt` is missing, no integer, below 0 or above `MAX_TIME`. */
	| "sent-at"
	/** Of `lines` only: `name` is missing or no string. */
	| "name"
	/** Of `lines` only: `at` is missing, no integer, below 0 or above `MAX_TIME`. */
	| "at";

/** A line as its sender said it. */
export interface Said {
	text: string;
	/** The sender's clock; only its form is checked. */
	sentAt: number;
}

/** A line of a room, as the gatherer publishes it. */
export interface Line {
	/** Its number in the room: the Group ID on `lines`. */
	number: bigint;
	/** Who said it: the last field of the sender's namespace. */
	name: string;
	text: string;
	/** The sender's clock. */
	sentAt: number;
	/** The gatherer's clock when it took the line. */
	at: number;
}

export type Read<T> = { ok: true; value: T } | { ok: false; why: Malformed };

// WHY `fatal` and `ignoreBOM`: `serde_json` reads bytes that are not UTF-8,
// and a byte order mark, as no JSON. Without `fatal` the decoder puts
// U+FFFD in place of the first, and without `ignoreBOM` it drops the second.
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const encoder = new TextEncoder();

function members(payload: Uint8Array): Map<string, JsonValue> | undefined {
	let text: string;
	try {
		text = decoder.decode(payload);
	} catch {
		return undefined;
	}
	const read = readJson(text);
	return read?.kind === "object" ? read.members : undefined;
}

function string(value: JsonValue | undefined): string | undefined {
	return value?.kind === "string" ? value.value : undefined;
}

/**
 * A time: a number written as an integer from 0 to `MAX_TIME`, with no
 * fraction, exponent or sign, as `as_u64` of `serde_json` takes it (it has
 * a float for `-0`, `1.0` and `1e3`).
 */
function time(value: JsonValue | undefined): number | undefined {
	if (value?.kind !== "number" || !/^(?:0|[1-9][0-9]*)$/.test(value.text)) {
		return undefined;
	}
	// WHY by the text's length first: a longer one would lose its last
	// digits in a number, and could come out at MAX_TIME.
	if (value.text.length > String(MAX_TIME).length) return undefined;
	const read = Number(value.text);
	return read <= MAX_TIME ? read : undefined;
}

/**
 * Reads a payload of `said`. Members other than the two are passed over, so
 * that a sender may add some without its lines being lost.
 */
export function readSaid(payload: Uint8Array): Read<Said> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const text = string(object.get("text"));
	if (text === undefined) return { ok: false, why: "text" };
	const sentAt = time(object.get("sentAt"));
	if (sentAt === undefined) return { ok: false, why: "sent-at" };
	return { ok: true, value: { text, sentAt } };
}

/**
 * Reads a payload of `lines`, the Object 0 of Group `number`. Members other
 * than the four are passed over, as in `readSaid`.
 */
export function readLine(number: bigint, payload: Uint8Array): Read<Line> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const name = string(object.get("name"));
	if (name === undefined) return { ok: false, why: "name" };
	const text = string(object.get("text"));
	if (text === undefined) return { ok: false, why: "text" };
	const sentAt = time(object.get("sentAt"));
	if (sentAt === undefined) return { ok: false, why: "sent-at" };
	const at = time(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return { ok: true, value: { number, name, text, sentAt, at } };
}

/** A UTF-16 unit of a surrogate that has no partner. */
const LONE_SURROGATE =
	/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

/**
 * Whether a string can be sent: a JavaScript string can hold a surrogate
 * without its partner, which `JSON.stringify` writes as an escape that
 * `serde_json` refuses (the row "a lone surrogate in the text").
 */
export function isWellFormed(text: string): boolean {
	return !LONE_SURROGATE.test(text);
}

function jsonString(text: string): string {
	if (!isWellFormed(text)) {
		throw new RangeError("a string with a lone surrogate cannot be sent");
	}
	// `JSON.stringify` escapes what `serde_json` escapes, in the same way:
	// `"`, `\` and the control characters, with `\u00xx` in lower case for
	// those that have no short escape; the rows "what JSON escapes" of both
	// tables are written back byte for byte.
	return JSON.stringify(text);
}

function checkTime(what: string, value: number): void {
	if (!Number.isSafeInteger(value) || value < 0) {
		throw new RangeError(
			`${what} is a whole number from 0 to 2^53 - 1, not ${value}`,
		);
	}
}

/**
 * The payload of a line on `said`, `{"text":...,"sentAt":...}`. Throws a
 * `RangeError` for a text with a lone surrogate and for a time that no
 * reader takes.
 */
export function saidPayload(said: Said): Uint8Array {
	checkTime("sentAt", said.sentAt);
	return encoder.encode(
		`{"text":${jsonString(said.text)},"sentAt":${said.sentAt}}`,
	);
}

/**
 * The payload of a line on `lines`, its members in the order of the
 * decision; the gatherer writes it, and the tests of this package write it
 * back to see that it is the gatherer's form. Throws as `saidPayload`.
 */
export function linePayload(line: Line): Uint8Array {
	checkTime("sentAt", line.sentAt);
	checkTime("at", line.at);
	return encoder.encode(
		`{"name":${jsonString(line.name)},"text":${jsonString(line.text)},"sentAt":${line.sentAt},"at":${line.at}}`,
	);
}
