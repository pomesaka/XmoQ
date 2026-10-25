import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { StreamResetErrorCode } from "../../src/core/error.ts";
import {
	arrivalOf,
	type EarlyAlias,
	heldBytesOf,
	settleOf,
} from "../../src/core/incoming-subgroups.ts";
import type { KeyValuePair } from "../../src/core/key-value-pair.ts";
import { resolveLimits } from "../../src/core/limits.ts";
import type { SubgroupObject } from "../../src/core/subgroup.ts";

/** A table of `fixtures/early-objects/`, which the Rust implementation reads too. */
function rows(file: string): string[][] {
	return readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"..",
			"fixtures",
			"early-objects",
			file,
		),
		"utf8",
	)
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => row.split("\t"));
}

function aliasOf(name: string | undefined): EarlyAlias {
	switch (name) {
		case "established":
		case "draining":
		case "retired":
		case "unknown":
			return name;
		default:
			throw new Error(`an alias this test knows: ${name}`);
	}
}

/** The outcome of a row as the rule says it: `deliver`, `hold`, `release`, `keep`, or `stop-` and the code. */
function outcomeOf(said: { type: string; code?: number }): string {
	if (said.type !== "stop") return said.type;
	switch (said.code) {
		case StreamResetErrorCode.CANCELLED:
			return "stop-cancelled";
		case StreamResetErrorCode.INTERNAL_ERROR:
			return "stop-internal-error";
		default:
			throw new Error(`a code this test knows: ${said.code}`);
	}
}

const MOST_HELD = resolveLimits().maxHeldSubgroupBytes;

/** A property as `held-bytes.tsv` writes it: `v` with a varint value, `bN` with N bytes. */
function propertyOf(text: string): KeyValuePair {
	if (!text.startsWith("b")) return { type: 2n, value: 0n };
	return { type: 1n, value: new Uint8Array(Number(text.slice(1))) };
}

describe("Objects whose Track Alias is not known yet", () => {
	test("the limit is the one the table counts to", () => {
		expect(MOST_HELD).toBe(1_048_576);
	});

	const arrivals = rows("arrival.tsv");
	test("the arrival table has its rows", () => {
		expect(arrivals.length).toBe(7);
	});
	test.each(arrivals)("%s", (_name, alias, pending, held, bytes, outcome) => {
		const said = arrivalOf(
			aliasOf(alias),
			pending === "yes",
			Number(held),
			Number(bytes),
			MOST_HELD,
		);
		expect(outcomeOf(said)).toBe(outcome ?? "");
	});

	const settles = rows("settle.tsv");
	test("the settle table has its rows", () => {
		expect(settles.length).toBe(5);
	});
	test.each(settles)("%s", (_name, alias, pending, outcome) => {
		expect(outcomeOf(settleOf(aliasOf(alias), pending === "yes"))).toBe(
			outcome ?? "",
		);
	});

	const counted = rows("held-bytes.tsv");
	test("the held-bytes table has its rows", () => {
		expect(counted.length).toBe(5);
	});
	test.each(counted)("%s", (_name, payload, properties, bytes) => {
		const listed =
			properties === "-" || properties === undefined
				? []
				: properties.split(",");
		const object: SubgroupObject = {
			objectId: 0n,
			properties: listed.map(propertyOf),
			status: "Normal",
			payload: new Uint8Array(Number(payload)),
		};
		expect(heldBytesOf(object)).toBe(Number(bytes));
	});
});
