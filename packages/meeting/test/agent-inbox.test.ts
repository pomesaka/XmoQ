import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type InboxLine,
	newInboxLines,
	readInboxLine,
} from "../src/core/agent-inbox.ts";

/** `fixtures/agent-lines/inbox.tsv`, which `crates/agent/src/input.rs` reads too: a name, the line as a string of JSON, and what it asks for. */
const inboxRows: [string, string, string][] = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"agent-lines",
		"inbox.tsv",
	),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => {
		const [name = "", line = "", asks = ""] = row.split("\t");
		return [name, line, asks];
	});

/** What a line asks for, as the third column writes it: its type, a space, and the rest as it is. */
const written = (read: InboxLine): string => {
	switch (read.type) {
		case "say":
			return `say ${read.text}`;
		case "frame":
			return `frame ${read.id ?? "-"}`;
		case "unknown":
			return `unknown ${read.name}`;
		case "nothing":
			return "nothing";
	}
};

test("the shared table of the inbox is read", () => {
	expect(inboxRows.length > 0).toBe(true);
});

test.each(inboxRows)("%s", (_name, line, asks) => {
	const text: unknown = JSON.parse(line);
	if (typeof text !== "string") throw new Error(`no string of JSON: ${line}`);
	expect(written(readInboxLine(text))).toBe(asks);
});

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

/** `fixtures/agent-lines/inbox-reads.tsv`, which `crates/agent/src/inbox.rs` reads too: a name, the file, the offset and the head of the read before, the lines read (`null` for one too long), and the next offset. */
const readRows: [string, string, number, string, (string | null)[], number][] =
	readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"fixtures",
			"agent-lines",
			"inbox-reads.tsv",
		),
		"utf8",
	)
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => {
			const [
				name = "",
				file = "",
				offset = "",
				head = "",
				lines = "",
				next = "",
			] = row.split("\t");
			const text = (json: string): string => {
				const value: unknown = JSON.parse(json);
				if (typeof value !== "string") throw new Error(`no string: ${json}`);
				return value;
			};
			const read: unknown = JSON.parse(lines);
			if (!Array.isArray(read)) throw new Error(`no list: ${lines}`);
			return [
				name,
				text(file),
				Number(offset),
				text(head),
				read.map((line: unknown) => (typeof line === "string" ? line : null)),
				Number(next),
			];
		});

test("the shared table of the reads of the inbox is read", () => {
	expect(readRows.length > 0).toBe(true);
});

test.each(readRows)("%s", (_name, file, offset, head, lines, next) => {
	const read = newInboxLines(utf8(file), offset, utf8(head));
	expect({
		lines: read.lines.map((line) => line ?? null),
		offset: read.offset,
	}).toEqual({ lines, offset: next });
});
