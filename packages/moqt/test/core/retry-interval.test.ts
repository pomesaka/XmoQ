import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	madeUpRetryInterval,
	retryAfterMs,
	retryWaitMs,
} from "../../src/core/error.ts";
import { RequestFailedError } from "../../src/webtransport/endings.ts";

/** A table of `fixtures/retry-interval`, which the Rust implementation reads too. */
function rowsOf(file: string): string[][] {
	return readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"..",
			"fixtures",
			"retry-interval",
			file,
		),
		"utf8",
	)
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => row.split("\t"));
}

const reading = rowsOf("reading.tsv");
const madeUp = rowsOf("made-up.tsv");

describe("a Retry Interval", () => {
	test("the shared tables have the rows this test expects", () => {
		expect([reading.length, madeUp.length]).toEqual([7, 11]);
	});

	test.each(reading)(
		"%s: %s is read as the shared table says",
		(_name, value = "", retried, after = "", wait = "") => {
			const interval = BigInt(value);
			expect(retryAfterMs(interval) !== undefined).toBe(retried === "yes");
			expect(retryAfterMs(interval)).toBe(
				after === "-" ? undefined : BigInt(after),
			);
			expect(retryWaitMs(interval)).toBe(
				wait === "-" ? undefined : Number(wait),
			);
			// What the application is handed with the error.
			const error = new RequestFailedError({
				errorCode: 0x9n,
				retryInterval: interval,
				errorReason: "",
			});
			expect(error.retryWaitMs).toBe(wait === "-" ? undefined : Number(wait));
		},
	);

	test.each(madeUp)(
		"a REQUEST_ERROR this endpoint makes up with %s (%s) has the one of the shared table",
		(_name, code = "", value = "") => {
			expect(madeUpRetryInterval(BigInt(code))).toBe(BigInt(value));
		},
	);
});
