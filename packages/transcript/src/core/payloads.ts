// What was said in one piece of a voice, as the transcriber publishes it on
// `speech` (decided on 2026-10-25). The Rust side is
// `crates/transcript-tracks/src/payloads.rs`; both read the shared rows of
// `fixtures/transcript-tracks/` in their tests.
//
// One piece a Group, numbered without a gap; its Object 0 is
// `{"from": [string, ...], "text": string, "startUs": number, "endUs": number, "at": number}`,
// in that order. `from` is the namespace of the broadcast, `startUs` and
// `endUs` the Timestamps of LOC of its audio track where the piece begins
// and ends, and `at` the transcriber's clock when it had read the piece, in
// milliseconds since the Unix epoch.

import {
	type JsonValue,
	membersOf,
	readJsonPayload,
	wholeNumberOf,
} from "@xmoq/json";

/** The largest number taken: 2^53 - 1, as on the Rust side (`MAX_NUMBER`). */
export const MAX_NUMBER = Number.MAX_SAFE_INTEGER;

/** Why a payload of `speech` is no piece: the words of `Malformed::word` on the Rust side. */
export type Malformed =
	/** Not UTF-8, not JSON, or no JSON object. */
	| "not-an-object"
	/** `from` is missing, no array, empty, or has a member that is no string or is empty. */
	| "from"
	/** `text` is missing or no string. */
	| "text"
	/** `startUs` is missing, no integer, below 0 or above `MAX_NUMBER`. */
	| "start-us"
	/** As `startUs`, or below `startUs`. */
	| "end-us"
	/** As `startUs`. */
	| "at";

/** A piece of speech. */
export interface Speech {
	/** Its number in the transcript: the Group ID on `speech`. */
	number: bigint;
	/** The namespace of the broadcast whose voice it was, every field. */
	from: string[];
	text: string;
	startUs: number;
	endUs: number;
	at: number;
}

export type Read<T> = { ok: true; value: T } | { ok: false; why: Malformed };

const encoder = new TextEncoder();

const members = (payload: Uint8Array): Map<string, JsonValue> | undefined =>
	membersOf(readJsonPayload(payload));

/** A whole number from 0 to `MAX_NUMBER`, as `as_u64` of `serde_json` takes it. */
const integer = (value: JsonValue | undefined): number | undefined =>
	wholeNumberOf(value, MAX_NUMBER);

function fields(value: JsonValue | undefined): string[] | undefined {
	if (value?.kind !== "array" || value.items.length === 0) return undefined;
	const read: string[] = [];
	for (const item of value.items) {
		if (item.kind !== "string" || item.value === "") return undefined;
		read.push(item.value);
	}
	return read;
}

/**
 * Reads a payload of `speech`, the Object 0 of Group `number`. Members other
 * than the five are passed over, so that a transcriber may add some.
 */
export function readSpeech(number: bigint, payload: Uint8Array): Read<Speech> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const from = fields(object.get("from"));
	if (from === undefined) return { ok: false, why: "from" };
	const text = object.get("text");
	if (text?.kind !== "string") return { ok: false, why: "text" };
	const startUs = integer(object.get("startUs"));
	if (startUs === undefined) return { ok: false, why: "start-us" };
	const endUs = integer(object.get("endUs"));
	if (endUs === undefined || endUs < startUs)
		return { ok: false, why: "end-us" };
	const at = integer(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return {
		ok: true,
		value: { number, from, text: text.value, startUs, endUs, at },
	};
}

/**
 * The payload of a piece on `speech`, its members in the order of the
 * decision; the transcriber writes it, and the tests of this package write
 * it back to see that it is the transcriber's form.
 */
export function speechPayload(speech: Speech): Uint8Array {
	const from = speech.from.map((field) => JSON.stringify(field)).join(",");
	return encoder.encode(
		`{"from":[${from}],"text":${JSON.stringify(speech.text)},"startUs":${speech.startUs},"endUs":${speech.endUs},"at":${speech.at}}`,
	);
}
