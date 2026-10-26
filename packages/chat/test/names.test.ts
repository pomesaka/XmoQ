import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type ChatRoom,
	LINES_TRACK,
	personNamespace,
	roomNamespace,
} from "../src/core/names.ts";

/**
 * The rows of `fixtures/chat-tracks/names.tsv`, which `xmoq-chat-tracks`
 * reads too. WHY not filter empty lines away: every row ends with one line
 * feed, as its README says; an empty row is an error of the table.
 */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"chat-tracks",
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

const decoder = new TextDecoder();
const textOf = (namespace: Uint8Array[]): string[] =>
	namespace.map((field) => decoder.decode(field));

/** `<kind>/<room>` of the table, as a room. */
function roomOf(text: string): ChatRoom {
	const [kind, name, ...rest] = text.split("/");
	if ((kind !== "broadcast" && kind !== "meeting") || name === undefined) {
		throw new Error(`no room ${text}`);
	}
	if (rest.length > 0) throw new Error(`no room ${text}`);
	return { kind, name };
}

const table = rows();

test("the table has its rows", () => {
	expect(table.length).toBe(14);
});

// This side makes the names and reads none: the rows of a participant and
// of a room's lines are what it makes; the rows of nobody's are the
// reader's, the gatherer's (`crates/chat-tracks`).
test.each(table)("%s", (_case, fields, track, read) => {
	const [what, room, name, ...rest] = (read ?? "").split(" ");
	const parsed: unknown = JSON.parse(fields ?? "");
	if (!Array.isArray(parsed)) throw new Error(`no fields ${fields}`);
	const namespace = parsed.map(String);
	switch (what) {
		case "participant":
			expect(rest).toEqual([]);
			expect(track).toBe("-");
			expect(textOf(personNamespace(roomOf(room ?? ""), name ?? ""))).toEqual(
				namespace,
			);
			break;
		case "lines":
			expect(name).toBeUndefined();
			expect(track).toBe(LINES_TRACK);
			expect(textOf(roomNamespace(roomOf(room ?? "")))).toEqual(namespace);
			break;
		case "none":
			expect(room).toBeUndefined();
			break;
		default:
			throw new Error(`no reading ${read}`);
	}
});
