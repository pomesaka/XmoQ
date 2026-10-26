import { describe, expect, test } from "bun:test";
import {
	type JsonValue,
	membersOf,
	plainOf,
	readJson,
	readJsonPayload,
	wholeNumberOf,
} from "../src/json.ts";

// The rest of what this reader does is read through the payloads of the
// tracks, by the shared tables (`packages/chat/test/payloads.test.ts`).
describe("an array", () => {
	test.each([
		["no items", "[]", []],
		["strings", '["xmoq", "demo"]', ["string:xmoq", "string:demo"]],
		["a number keeps its text", "[1.0, -0]", ["number:1.0", "number:-0"]],
	])("%s", (_, text, items) => {
		const read = readJson(text);
		expect(read?.kind).toBe("array");
		const kept =
			read?.kind === "array"
				? read.items.map((item) =>
						item.kind === "string"
							? `string:${item.value}`
							: item.kind === "number"
								? `number:${item.text}`
								: item.kind,
					)
				: [];
		expect(kept).toEqual(items);
	});
});

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

describe("a payload", () => {
	test.each<[string, Uint8Array, JsonValue["kind"] | undefined]>([
		["JSON in UTF-8", bytes('{"a":1}'), "object"],
		["bytes that are not UTF-8", Uint8Array.of(0x22, 0xff, 0x22), undefined],
		[
			"a byte order mark, which serde_json does not take",
			Uint8Array.of(0xef, 0xbb, 0xbf, ...bytes("1")),
			undefined,
		],
		["no JSON", bytes("{"), undefined],
	])("%s", (_, payload, kind) => {
		expect(readJsonPayload(payload)?.kind).toBe(kind);
	});
});

describe("a whole number, as as_u64 of serde_json takes it", () => {
	test.each<[string, string, number | undefined, number | undefined]>([
		["0", "0", undefined, 0],
		[
			"the largest a number holds exactly",
			"9007199254740991",
			undefined,
			9007199254740991,
		],
		["one more", "9007199254740992", undefined, undefined],
		["a longer one", "90071992547409910", undefined, undefined],
		["at the most given", "10", 10, 10],
		["over the most given", "11", 10, undefined],
		["a fraction of 0", "1.0", undefined, undefined],
		["an exponent", "1e3", undefined, undefined],
		["minus 0", "-0", undefined, undefined],
		["below 0", "-1", undefined, undefined],
	])("%s", (_, text, most, read) => {
		expect(wholeNumberOf(readJson(text), most)).toBe(read);
	});

	test("a string of digits is none", () => {
		expect(wholeNumberOf(readJson('"5"'))).toBeUndefined();
	});
});

describe("a plain value, as JSON.parse gives it", () => {
	test.each<[string, string]>([
		["literals", "[true, false, null]"],
		["numbers by their text", "[1e3, 1.0, -0, 0.5]"],
		["an object and its members", '{"a":{"b":[1,"c"]}}'],
		["a member twice: the last", '{"a":1,"a":2}'],
		["a member named __proto__", '{"__proto__":1}'],
	])("%s", (_, text) => {
		const read = readJson(text);
		if (read === undefined) throw new Error(`no JSON: ${text}`);
		expect(plainOf(read)).toEqual(JSON.parse(text));
	});
});

test("the members of an object, and of nothing else", () => {
	expect([
		membersOf(readJson('{"a":1}'))?.size,
		membersOf(readJson("[1]")),
		membersOf(undefined),
	]).toEqual([1, undefined, undefined]);
});
