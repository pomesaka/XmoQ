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
import { createSession } from "../../src/core/session.ts";
import type { SubgroupObject } from "../../src/core/subgroup.ts";
import { hex, utf8 } from "../helpers.ts";

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

	const reused = rows("reused-alias.tsv");
	test("the reused-alias table has its rows", () => {
		expect(reused.length).toBe(4);
	});
	test.each(reused)("%s", (_name, end, retired, outcome) => {
		// The alias is written as one byte below: a varint of 0 to 63.
		expect(Number(retired)).toBeLessThan(64);
		const session = createSession();
		session.start();
		// Request ID 0 on stream 1, Request ID 2 on stream 2.
		session.subscribe({ trackNamespace: [utf8("t")], trackName: utf8("a") });
		session.subscribe({ trackNamespace: [utf8("t")], trackName: utf8("b") });
		const outputs = [
			// One Object of Track Alias 7.
			...session.receivePeerUniStreamData(
				0,
				hex("14 07 00 00 00 00 01 61"),
				end === "fin",
			),
			// With the Mandatory Track Property 0x4000.
			...session.receiveRequestStreamData(
				1,
				hex(`04 00 06 ${retired?.padStart(2, "0")} 00 c0 40 00 01`),
				false,
			),
			...session.receiveRequestStreamData(2, hex("04 00 02 07 00"), false),
		];
		// To any subscription, as the Rust test counts what is released: the
		// first one, which is over, must not get it either.
		const given = outputs.some(
			(output) => output.type === "event" && output.event.type === "object",
		);
		expect(given ? "given" : "none").toBe(outcome ?? "");
	});
});
