// What a line says, as a participant sends it on `said` and as the gatherer
// sends it on `lines` (the payloads decided on 2026-10-24). The Rust side is
// `crates/chat-tracks/src/payloads.rs`; both read the shared rows of
// `fixtures/chat-tracks/` in their tests.
//
// `said`, one Object a Group: `{"text": string, "sentAt": number}`.
// `lines`: `{"name": string, "text": string, "sentAt": number, "at": number}`,
// in that order. `sentAt` is the sender's clock and `at` the gatherer's, in
// milliseconds since the Unix epoch.

import {
	type JsonValue,
	membersOf,
	readJsonPayload,
	wholeNumberOf,
} from "@xmoq/json";

/**
 * The largest time taken: 2^53 - 1, the largest integer a JavaScript number
 * holds exactly, as on the Rust side (`MAX_SENT_AT`).
 */
export const MAX_TIME = Number.MAX_SAFE_INTEGER;

/** Why a payload of a track of the chat is none: the words of `Malformed::word` on the Rust side. */
export type Malformed =
	/** Not UTF-8, not JSON, or no JSON object. */
	| "not-an-object"
	/** `text` is missing or no string. */
	| "text"
	/** `sentAt` is missing, no integer, below 0 or above `MAX_TIME`. */
	| "sent-at"
	/** Of `lines` only: `name` is missing or no string. */
	| "name"
	/** Of `lines`, `reactions` and `present`: `at` is missing, no integer, below 0 or above `MAX_TIME`. */
	| "at"
	/** Of `reacted` only: `emoji` is missing, no string, or not of 1 to `MOST_EMOJI_UNITS` UTF-16 units. */
	| "emoji"
	/** Of `reactions` only: `counts` is missing, no object, empty, or has an emoji or a count no reader takes. */
	| "counts"
	/** Of `present` only: `count` is missing, no integer, below 0 or above `MAX_TIME`. */
	| "count";

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

const encoder = new TextEncoder();

const members = (payload: Uint8Array): Map<string, JsonValue> | undefined =>
	membersOf(readJsonPayload(payload));

function string(value: JsonValue | undefined): string | undefined {
	return value?.kind === "string" ? value.value : undefined;
}

/** A time: a whole number from 0 to `MAX_TIME`, as `as_u64` of `serde_json` takes it. */
const time = (value: JsonValue | undefined): number | undefined =>
	wholeNumberOf(value, MAX_TIME);

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

// The reactions and the count of a room (the payloads decided on
// 2026-10-26): a participant says a reaction on its track `reacted`, one
// Object a press, `{"emoji": string, "sentAt": number}`; the gatherer
// counts them over a while on the room's `reactions`,
// `{"counts": {emoji: number, ...}, "at": number}`, and how many it follows
// in the room on `present`, `{"count": number, "at": number}`. How fast one
// may react and how many emoji a while has are the gatherer's to bound, not
// the reader's.

/**
 * The most UTF-16 units of an emoji: one with its modifiers, and no text.
 * The form the room had for a reaction (2026-10-12), and the one of
 * `crates/chat-tracks`; what it shows is not looked at.
 */
export const MOST_EMOJI_UNITS = 8;

/** A reaction as a participant said it. */
export interface Reacted {
	emoji: string;
	/** The sender's clock; only its form is checked. */
	sentAt: number;
}

/** The reactions of a room over a while, as the gatherer publishes them. */
export interface Reactions {
	/** Each emoji with how many times it was said, in the order of the payload. */
	counts: Map<string, number>;
	/** The gatherer's clock when the while ended. */
	at: number;
}

/** How many the gatherer follows in a room. */
export interface Present {
	count: number;
	/** The gatherer's clock when it changed. */
	at: number;
}

const isEmoji = (text: string): boolean =>
	text.length >= 1 && text.length <= MOST_EMOJI_UNITS;

/** Reads a payload of `reacted`. Members other than the two are passed over. */
export function readReacted(payload: Uint8Array): Read<Reacted> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const emoji = string(object.get("emoji"));
	if (emoji === undefined || !isEmoji(emoji))
		return { ok: false, why: "emoji" };
	const sentAt = time(object.get("sentAt"));
	if (sentAt === undefined) return { ok: false, why: "sent-at" };
	return { ok: true, value: { emoji, sentAt } };
}

/**
 * Reads a payload of `reactions`. A count is a whole number from 1; an emoji
 * twice has the last count, as any member twice.
 */
export function readReactions(payload: Uint8Array): Read<Reactions> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const given = object.get("counts");
	if (given?.kind !== "object" || given.members.size === 0) {
		return { ok: false, why: "counts" };
	}
	const counts = new Map<string, number>();
	for (const [emoji, value] of given.members) {
		const count = time(value);
		if (!isEmoji(emoji) || count === undefined || count < 1) {
			return { ok: false, why: "counts" };
		}
		counts.set(emoji, count);
	}
	const at = time(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return { ok: true, value: { counts, at } };
}

/** Reads a payload of `present`. Members other than the two are passed over. */
export function readPresent(payload: Uint8Array): Read<Present> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const count = time(object.get("count"));
	if (count === undefined) return { ok: false, why: "count" };
	const at = time(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return { ok: true, value: { count, at } };
}

function checkEmoji(emoji: string): void {
	if (!isEmoji(emoji)) {
		throw new RangeError(
			`an emoji is 1 to ${MOST_EMOJI_UNITS} UTF-16 units, not ${emoji.length}`,
		);
	}
}

/**
 * The payload of a reaction on `reacted`. Throws a `RangeError` for an emoji
 * no reader takes, and as `saidPayload`.
 */
export function reactedPayload(reacted: Reacted): Uint8Array {
	checkEmoji(reacted.emoji);
	checkTime("sentAt", reacted.sentAt);
	return encoder.encode(
		`{"emoji":${jsonString(reacted.emoji)},"sentAt":${reacted.sentAt}}`,
	);
}

/**
 * The payload of the counts on `reactions`, the emoji in the order of the
 * map; the gatherer writes it, and the tests write it back to see that it
 * is the gatherer's form. Throws as `reactedPayload`, and for no emoji or a
 * count below 1.
 */
export function reactionsPayload(reactions: Reactions): Uint8Array {
	if (reactions.counts.size === 0) {
		throw new RangeError("the counts of reactions have one emoji at least");
	}
	const counts: string[] = [];
	for (const [emoji, count] of reactions.counts) {
		checkEmoji(emoji);
		checkTime("a count", count);
		if (count < 1) throw new RangeError("a count of reactions is from 1");
		counts.push(`${jsonString(emoji)}:${count}`);
	}
	checkTime("at", reactions.at);
	return encoder.encode(
		`{"counts":{${counts.join(",")}},"at":${reactions.at}}`,
	);
}

/** The payload of the count on `present`. Throws as `saidPayload`. */
export function presentPayload(present: Present): Uint8Array {
	checkTime("count", present.count);
	checkTime("at", present.at);
	return encoder.encode(`{"count":${present.count},"at":${present.at}}`);
}
