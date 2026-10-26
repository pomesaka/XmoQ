import { afterEach, beforeEach, expect, jest, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { RequestErrorCode } from "../../src/core/error.ts";
import type { GoAwayNotice } from "../../src/webtransport/connection.ts";
import { RequestFailedError } from "../../src/webtransport/endings.ts";
import { goAwayBehind } from "../../src/webtransport/keep-moving.ts";
import { pass } from "./clock.ts";

/**
 * The rows of `fixtures/ask-again/go-away-behind.tsv`, which
 * `client::go_away_behind` of `xmoq-moqt-io` reads too: (case, the code of
 * the refusal, what comes after it, the wait, what the wait gives).
 */
function rows(): [string, string, string, string, string][] {
	const file = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"..",
		"fixtures",
		"ask-again",
		"go-away-behind.tsv",
	);
	const text = readFileSync(file, "utf8");
	if (!text.endsWith("\n"))
		throw new Error(`${file} does not end its last row`);
	return text
		.slice(0, -1)
		.split("\n")
		.map((row) => {
			const [name, code, comes, waitMs, gives, ...more] = row.split("\t");
			if (
				name === undefined ||
				code === undefined ||
				comes === undefined ||
				waitMs === undefined ||
				gives === undefined ||
				more.length > 0
			) {
				throw new Error(`a row of five columns: ${row}`);
			}
			return [name, code, comes, waitMs, gives];
		});
}

const table = rows();

/** The code of the refusal by its name in the table. */
const CODES: Record<string, bigint> = {
	GOING_AWAY: RequestErrorCode.GOING_AWAY,
	NOT_SUPPORTED: RequestErrorCode.NOT_SUPPORTED,
};

beforeEach(() => {
	jest.useFakeTimers();
});

afterEach(() => {
	jest.useRealTimers();
});

test("the table has its rows", () => {
	// A table that lost rows would pass.
	expect(table.length).toBe(6);
});

test.each(table)("%s", async (_name, codeName, comes, waitText, gives) => {
	const errorCode = CODES[codeName];
	if (errorCode === undefined)
		throw new Error(`a code the test does not know: ${codeName}`);
	const goAway = Promise.withResolvers<GoAwayNotice>();
	const closed = Promise.withResolvers<{
		type: "local";
		code: number;
		reason: string;
	}>();
	let last = 0;
	for (const step of comes === "-" ? [] : comes.split(" ")) {
		const [what, at] = step.split("@");
		const atMs = Number(at);
		last = Math.max(last, atMs);
		switch (what) {
			case "goaway":
				setTimeout(
					() => goAway.resolve({ newSessionUri: "https://b/", timeoutMs: 0 }),
					atMs,
				);
				break;
			case "closed":
				setTimeout(
					() => closed.resolve({ type: "local", code: 0, reason: "" }),
					atMs,
				);
				break;
			// A data stream is read apart from the GOAWAY: nothing to keep.
			case "object":
				break;
			default:
				throw new Error(`a step the test does not know: ${step}`);
		}
	}
	const waitMs = Number(waitText);
	const refusal = new RequestFailedError({
		errorCode,
		retryInterval: 0n,
		errorReason: "",
	});
	const got = goAwayBehind(
		{ goAway: goAway.promise, closed: closed.promise },
		refusal,
		waitMs,
	);
	await pass(Math.max(waitMs, last) + 1);
	expect((await got) === undefined ? "none" : "goaway").toBe(gives);
});
