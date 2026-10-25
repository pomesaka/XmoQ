import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readSpeech, speechPayload } from "../src/core/payloads.ts";

/** The rows of `fixtures/transcript-tracks/speech.tsv`, which `crates/transcript-tracks` reads too. */
const rows = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"transcript-tracks",
		"speech.tsv",
	),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => row.split("\t"));

const bytes = (text: string | undefined): Uint8Array =>
	new TextEncoder().encode(text ?? "");
/** WHY: the columns are strings, which `toEqual` would not take for the words of `Malformed`. */
const untyped = (read: unknown): unknown => read;

describe("fixtures/transcript-tracks", () => {
	test("the table has the rows both readers count on", () => {
		expect(rows.map((row) => row.length)).toEqual(rows.map(() => 10));
		expect(rows.length).toBe(21);
	});

	test.each(rows)(
		"%s",
		(_case, group, payload, from, text, startUs, endUs, at, malformed, written) => {
			const number = BigInt(group ?? "0");
			const read = readSpeech(number, bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: {
					number,
					from: JSON.parse(from ?? ""),
					text: JSON.parse(text ?? ""),
					startUs: Number(startUs),
					endUs: Number(endUs),
					at: Number(at),
				},
			});
			if (!read.ok) return;
			const back = new TextDecoder().decode(speechPayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);
});

// Bytes that are not UTF-8, which the table, being text, cannot hold; the
// Rust test has the same case.
test("a payload that is not UTF-8 is no piece", () => {
	const payload = new Uint8Array([
		...bytes('{"from":["a"],"text":"'),
		0xff,
		...bytes('","startUs":1,"endUs":2,"at":3}'),
	]);
	expect(untyped(readSpeech(0n, payload))).toEqual({
		ok: false,
		why: "not-an-object",
	});
});
