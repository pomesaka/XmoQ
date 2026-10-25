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

test.each<{
	name: string;
	file: string;
	offset: number;
	/** The first bytes of the file at the read before. */
	head: string;
	lines: (string | undefined)[];
	next: number;
}>([
	{
		name: "the whole lines from the beginning",
		file: "a\nbc\n",
		offset: 0,
		head: "",
		lines: ["a", "bc"],
		next: 5,
	},
	{
		name: "only what came after the last read",
		file: "a\nbc\nd\n",
		offset: 5,
		head: "",
		lines: ["d"],
		next: 7,
	},
	{
		name: "a last line without its line feed is left for the next read",
		file: "a\nb",
		offset: 0,
		head: "",
		lines: ["a"],
		next: 2,
	},
	{
		name: "a file shorter than the last read was written anew, and is read from its beginning",
		file: "x\n",
		offset: 5,
		head: "",
		lines: ["x"],
		next: 2,
	},
	{
		name: "a character of more than one byte is read whole",
		file: "こんにちは\n",
		offset: 0,
		head: "",
		lines: ["こんにちは"],
		next: 16,
	},
	{
		name: "a line over 4,096 bytes is not read",
		file: `${"x".repeat(4097)}\nok\n`,
		offset: 0,
		head: "",
		lines: [undefined, "ok"],
		next: 4101,
	},
	{
		name: "a file written anew, longer than before, is read from its beginning",
		file: "new\nlonger line\n",
		offset: 4,
		head: "old\n",
		lines: ["new", "longer line"],
		next: 16,
	},
	{
		name: "a file added to, that begins as it did, is read from where the last read ended",
		file: "old\nnew\n",
		offset: 4,
		head: "old\n",
		lines: ["new"],
		next: 8,
	},
])("$name", ({ file, offset, head, lines, next }) => {
	const read = newInboxLines(utf8(file), offset, utf8(head));
	expect({ lines: read.lines, offset: read.offset }).toEqual({
		lines,
		offset: next,
	});
});
