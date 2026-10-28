import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { DEFAULT_HIDE_AFTER_MS } from "../src/core/frame-queue.ts";

/**
 * The rows of `fixtures/shared-values/values.tsv` that name this package,
 * by name: values that other parts hold as constants of their own.
 */
function sharedValues(): Record<string, number> {
	const table = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"fixtures",
			"shared-values",
			"values.tsv",
		),
		"utf8",
	);
	const held: Record<string, number> = {};
	for (const row of table.split("\n").filter((row) => row !== "")) {
		const [name, value, holders, ...rest] = row.split("\t");
		if (
			name === undefined ||
			value === undefined ||
			holders === undefined ||
			rest.length > 0
		) {
			throw new Error(`a row of other than three columns: ${row}`);
		}
		if (holders.split(",").includes("@xmoq/media")) held[name] = Number(value);
	}
	return held;
}

test("the values held alike with other parts are as the shared table says", () => {
	expect(sharedValues()).toEqual({
		"video-hide-after-ms": DEFAULT_HIDE_AFTER_MS,
	});
});
