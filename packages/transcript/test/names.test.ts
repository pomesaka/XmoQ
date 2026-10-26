import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { SPEECH_TRACK, transcriptNamespace } from "../src/core/names.ts";

/**
 * The rows of `fixtures/transcript-tracks/names.tsv`, which
 * `xmoq-transcript-tracks` reads too; this side makes the names of the rows
 * of a transcript and reads none.
 */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"transcript-tracks",
		"names.tsv",
	);
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n"))
		throw new Error(`${path}: no line feed at its end`);
	const read = table
		.slice(0, -1)
		.split("\n")
		.map((row) => row.split("\t"));
	if (read.some((row) => row.length !== 4)) {
		throw new Error(`${path}: a row of other than four columns`);
	}
	return read;
}

const transcripts = rows().filter(([, , , listened]) => listened !== "none");

test("the table has its rows of a transcript", () => {
	expect(transcripts.length).toBe(3);
});

test.each(transcripts)("%s", (_case, fields, name, listened) => {
	const decoder = new TextDecoder();
	expect({
		namespace: transcriptNamespace(JSON.parse(listened ?? "")).map((field) =>
			decoder.decode(field),
		),
		name: SPEECH_TRACK,
	}).toEqual({ namespace: JSON.parse(fields ?? ""), name });
});
