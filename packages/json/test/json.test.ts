import { describe, expect, test } from "bun:test";
import { readJson } from "../src/json.ts";

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
