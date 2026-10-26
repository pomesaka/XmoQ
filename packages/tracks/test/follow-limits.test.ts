import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	FOLLOW_FETCH_TIMEOUT_MS,
	FOLLOW_RENDEZVOUS_MS,
} from "../src/follow.ts";

/**
 * The rows of `fixtures/followed-track/limits.tsv` whose TypeScript value
 * is here; those of a track's payload and of the numbers remembered are of
 * `@xmoq/chat` and `@xmoq/transcript`.
 */
const OURS: Record<string, number | undefined> = {
	"rendezvous-ms": FOLLOW_RENDEZVOUS_MS,
	"fetch-timeout-ms": FOLLOW_FETCH_TIMEOUT_MS,
};

/** The table's rows: every row ends with one line feed, as its README says. */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"followed-track",
		"limits.tsv",
	);
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n"))
		throw new Error(`${path}: no line feed at its end`);
	return table
		.slice(0, -1)
		.split("\n")
		.map((row) => row.split("\t"));
}

const limits = rows().filter(([name]) => name !== undefined && name in OURS);

test("the table has the rows whose value is here", () => {
	expect(limits.map(([name]) => name).sort()).toEqual(Object.keys(OURS).sort());
});

test.each(limits)("%s", (name, _rust, typescript) => {
	expect(name === undefined ? undefined : OURS[name]).toEqual(
		typescript === "-" ? undefined : Number(typescript),
	);
});
