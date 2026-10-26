import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	linePayload,
	presentPayload,
	reactedPayload,
	reactionsPayload,
	readLine,
	readPresent,
	readReacted,
	readReactions,
	readSaid,
	saidPayload,
} from "../src/core/payloads.ts";

/** The rows of a table of `fixtures/chat-tracks/`, which `crates/chat-tracks` reads too. */
function sharedRows(file: string, fields: number): string[][] {
	const rows = readFileSync(
		join(import.meta.dir, "..", "..", "..", "fixtures", "chat-tracks", file),
		"utf8",
	)
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => row.split("\t"));
	for (const row of rows) {
		if (row.length !== fields) throw new Error(`a row of ${fields}: ${row}`);
	}
	return rows;
}

/** A text column: a JSON string literal, or `-`. */
const textOf = (literal: string | undefined): string | undefined =>
	literal === "-" || literal === undefined ? undefined : JSON.parse(literal);
const timeOf = (column: string | undefined): number | undefined =>
	column === "-" || column === undefined ? undefined : Number(column);
const bytes = (text: string | undefined): Uint8Array =>
	new TextEncoder().encode(text ?? "");
/** WHY: the columns are strings, which `toEqual` would not take for the words of `Malformed`. */
const untyped = (read: unknown): unknown => read;

describe("fixtures/chat-tracks", () => {
	const said = sharedRows("said.tsv", 6);
	const lines = sharedRows("lines.tsv", 9);
	const reacted = sharedRows("reacted.tsv", 6);
	const reactions = sharedRows("reactions.tsv", 6);
	const present = sharedRows("present.tsv", 6);

	test("the tables have the rows both readers count on", () => {
		expect({
			said: said.length,
			lines: lines.length,
			reacted: reacted.length,
			reactions: reactions.length,
			present: present.length,
		}).toEqual({
			said: 27,
			lines: 14,
			reacted: 15,
			reactions: 17,
			present: 12,
		});
	});

	test.each(reacted)(
		"reacted: %s",
		(_case, payload, emoji, sentAt, malformed, written) => {
			const read = readReacted(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: { emoji: textOf(emoji), sentAt: timeOf(sentAt) },
			});
			if (!read.ok) return;
			const back = new TextDecoder().decode(reactedPayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);

	test.each(reactions)(
		"reactions: %s",
		(_case, payload, counts, at, malformed, written) => {
			const read = readReactions(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			if (!read.ok) throw new Error(`not read: ${read.why}`);
			// The order of the members is part of what is read.
			const expected: unknown = JSON.parse(counts ?? "");
			expect(untyped([[...read.value.counts], read.value.at])).toEqual([
				Object.entries(
					typeof expected === "object" && expected !== null ? expected : {},
				),
				timeOf(at),
			]);
			const back = new TextDecoder().decode(reactionsPayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);

	test.each(present)(
		"present: %s",
		(_case, payload, count, at, malformed, written) => {
			const read = readPresent(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: { count: timeOf(count), at: timeOf(at) },
			});
			if (!read.ok) return;
			const back = new TextDecoder().decode(presentPayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);

	test.each(said)(
		"said: %s",
		(_case, payload, text, sentAt, malformed, written) => {
			const read = readSaid(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: { text: textOf(text), sentAt: timeOf(sentAt) },
			});
			if (!read.ok) return;
			const back = new TextDecoder().decode(saidPayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);

	test.each(lines)(
		"lines: %s",
		(_case, group, payload, name, text, sentAt, at, malformed, written) => {
			const number = BigInt(group ?? "");
			const read = readLine(number, bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: {
					number,
					name: textOf(name),
					text: textOf(text),
					sentAt: timeOf(sentAt),
					at: timeOf(at),
				},
			});
			if (!read.ok) return;
			const back = new TextDecoder().decode(linePayload(read.value));
			expect(back === payload).toBe(written === "yes");
		},
	);
});

describe("what the tables, being text, cannot hold", () => {
	test.each([
		// The cases of `a_payload_the_tables_cannot_hold_is_no_line` of the Rust side.
		[
			"said, not UTF-8",
			"said",
			[...bytes('{"text":"'), 0xff, ...bytes('","sentAt":5}')],
		],
		[
			"lines, not UTF-8",
			"lines",
			[
				...bytes('{"name":"'),
				0xff,
				...bytes('","text":"t","sentAt":1,"at":2}'),
			],
		],
		// `serde_json` reads no byte order mark; a TextDecoder drops it unless told not to.
		[
			"said, a byte order mark",
			"said",
			[0xef, 0xbb, 0xbf, ...bytes('{"text":"x","sentAt":5}')],
		],
		// RFC 8259 §7: control characters must be escaped in a string.
		[
			"said, a tab in the text as it is",
			"said",
			[...bytes('{"text":"a'), 0x09, ...bytes('b","sentAt":5}')],
		],
	])("%s is no line", (_case, track, payload) => {
		const read =
			track === "said"
				? readSaid(Uint8Array.from(payload))
				: readLine(0n, Uint8Array.from(payload));
		expect(read).toEqual({ ok: false, why: "not-an-object" });
	});
});

describe("what is not written", () => {
	test.each([
		["a lone surrogate in the text", "\ud800", 5],
		["a time below 0", "x", -1],
		["a time with a fraction", "x", 1.5],
		["a time beyond 2^53 - 1", "x", 2 ** 53],
	])("said: %s throws a RangeError", (_case, text, sentAt) => {
		expect(() => saidPayload({ text, sentAt })).toThrow(RangeError);
	});

	test.each([
		["an empty emoji", ""],
		["an emoji of 11 UTF-16 units", "👨‍👩‍👧‍👦"],
		["a lone surrogate", "\ud800"],
	])("reacted: %s throws a RangeError", (_case, emoji) => {
		expect(() => reactedPayload({ emoji, sentAt: 5 })).toThrow(RangeError);
	});

	test.each<[string, [string, number][]]>([
		["no emoji", []],
		["a count of 0", [["👏", 0]]],
		["a count with a fraction", [["👏", 1.5]]],
		["an empty emoji", [["", 1]]],
	])("reactions: %s throws a RangeError", (_case, counts) => {
		expect(() => reactionsPayload({ counts: new Map(counts), at: 5 })).toThrow(
			RangeError,
		);
	});

	test.each([
		["a count below 0", -1],
		["a count with a fraction", 1.5],
	])("present: %s throws a RangeError", (_case, count) => {
		expect(() => presentPayload({ count, at: 5 })).toThrow(RangeError);
	});
});
