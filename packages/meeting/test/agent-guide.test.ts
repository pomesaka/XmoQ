import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { AGENT_GUIDE, callLine } from "../src/core/agent-guide.ts";

const fixture = (name: string): string =>
	readFileSync(
		join(import.meta.dir, "..", "..", "..", "fixtures", "agent-lines", name),
		"utf8",
	);

test("the guide is the shared text, which xmoq-agent holds its own to", () => {
	expect(AGENT_GUIDE).toBe(fixture("AGENT.md"));
});

/** `fixtures/agent-lines/call.tsv`, which `crates/agent/src/guide.rs` reads too: a name, the folder, and the line to paste. */
const callRows: [string, string, string][] = fixture("call.tsv")
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => {
		const [name = "", folder = "", line = ""] = row.split("\t");
		return [name, folder, line];
	});

test("the shared table of the line to paste is read", () => {
	expect(callRows.length > 0).toBe(true);
});

test.each(callRows)("%s", (_name, folder, line) => {
	expect(callLine(folder)).toBe(line);
});
