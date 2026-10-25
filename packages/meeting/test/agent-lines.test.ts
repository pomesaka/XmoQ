import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type AgentEvent,
	agentLine,
	framePath,
} from "../src/core/agent-lines.ts";

/** `fixtures/agent-lines/lines.tsv`: the name of a row, and its line. */
const rows: [string, string][] = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"agent-lines",
		"lines.tsv",
	),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => {
		const [name = "", line = ""] = row.split("\t");
		return [name, line];
	});

/** The event of each row of the shared table, by its name. */
const EVENTS: Record<string, AgentEvent> = {
	"a participant came": { type: "participant", state: "joined", id: "p1" },
	"a participant left": { type: "participant", state: "left", id: "p1" },
	"a line of the chat": {
		type: "chat",
		number: 7n,
		from: "alice",
		text: "hello",
		sentAt: 990n,
		gatheredAt: 995n,
	},
	"a line with what a terminal would act on, and a line break": {
		type: "chat",
		number: 8n,
		from: 'b"ob',
		text: "a\u001b[2J\nb",
		sentAt: 0n,
		gatheredAt: 0n,
	},
	"a piece of speech": {
		type: "speech",
		number: 3n,
		from: ["xmoq", "meet", "demo", "p1"],
		text: "こんにちは",
		startUs: 10n,
		endUs: 20n,
		readAt: 30n,
	},
	"a picture of a participant, at a path from the folder": {
		type: "frame",
		path: framePath(1_700_000_000_000, 1, 12n, 7n),
		group: 12n,
		object: 7n,
		timestampUs: 1_700_000_000_000_000n,
		width: 320,
		height: 180,
		asked: "command",
		id: "p1",
	},
	"an error": {
		type: "error",
		message:
			"no command /nothing: /frame is one, and // begins a line that is said",
	},
};

test("every row of the shared table has an event here, and every event a row", () => {
	expect(rows.map(([name]) => name).sort()).toEqual(Object.keys(EVENTS).sort());
});

test.each(rows)("%s", (name, line) => {
	const event = EVENTS[name];
	if (event === undefined) throw new Error(`no event for ${name}`);
	expect(agentLine(event, 1000)).toBe(line);
});
