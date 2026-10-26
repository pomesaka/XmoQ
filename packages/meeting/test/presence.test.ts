import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	meetingPrefix,
	participantNamespace,
	participantOf,
	seatSubscribeNamespace,
} from "../src/core/names.ts";
import {
	createPresence,
	type PresenceChange,
	watchedOf,
} from "../src/core/presence.ts";

/** The rows of a table of `fixtures/meeting-presence/`, which `crates/viewer` and `crates/meeting-tracks` read too. */
function sharedRows(file: string): string[][] {
	const table = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"fixtures",
			"meeting-presence",
			file,
		),
		"utf8",
	);
	return table
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => row.split("\t"));
}

const written = (change: PresenceChange): string =>
	`${change.type}:${change.id}`;

describe("who comes and who leaves: fixtures/meeting-presence", () => {
	test.each(sharedRows("who-is-there.tsv"))(
		"%s",
		(_name, most, steps, expected) => {
			const presence = createPresence(Number(most));
			const changes: PresenceChange[] = [];
			for (const step of (steps ?? "").split(" ")) {
				const [what, id] = step.split(":");
				if (what === "named" && id !== undefined) {
					const change = presence.named(id);
					if (change !== undefined) changes.push(change);
				} else if (what === "done" && id !== undefined) {
					const change = presence.done(id);
					if (change !== undefined) changes.push(change);
				} else if (step === "session-ended") {
					presence.sessionEnded();
				} else if (step === "settled") {
					changes.push(...presence.settled());
				} else {
					throw new Error(`no such step: ${step}`);
				}
			}
			expect(changes.map(written)).toEqual(
				expected === "-" ? [] : (expected ?? "").split(" "),
			);
		},
	);

	test("the table has its 10 rows of 4 columns", () => {
		expect(sharedRows("who-is-there.tsv").map((row) => row.length)).toEqual([
			4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
		]);
	});
});

describe("the participant of a namespace: fixtures/meeting-presence", () => {
	test.each(sharedRows("participant-of.tsv"))(
		"%s",
		(_name, room, namespace, expected) => {
			expect(participantOf(room ?? "", (namespace ?? "").split(" "))).toBe(
				expected === "-" ? undefined : expected,
			);
		},
	);

	test("the table has its 6 rows of 4 columns", () => {
		expect(sharedRows("participant-of.tsv").map((row) => row.length)).toEqual([
			4, 4, 4, 4, 4, 4,
		]);
	});
});

describe("the namespaces of a meeting: fixtures/meeting-presence", () => {
	const namespaceOf = (room: string, id: string, what: string): string[] => {
		switch (what) {
			case "prefix":
				return meetingPrefix(room);
			case "participant":
				return participantNamespace(room, id);
			case "seat-subscribe":
				return seatSubscribeNamespace(room);
			default:
				throw new Error(`no such namespace: ${what}`);
		}
	};

	test.each(sharedRows("namespaces.tsv"))(
		"%s",
		(_name, room, id, what, namespace) => {
			expect(namespaceOf(room ?? "", id ?? "", what ?? "").join(" ")).toBe(
				namespace ?? "",
			);
		},
	);

	test("the table has its 6 rows of 5 columns", () => {
		expect(sharedRows("namespaces.tsv").map((row) => row.length)).toEqual([
			5, 5, 5, 5, 5, 5,
		]);
	});
});

describe("who is there, in the order they came", () => {
	test.each([
		["as they came", ["c", "a", "b"], [], ["c", "a", "b"]],
		["one who left and came again comes last", ["a", "b"], ["a"], ["b", "a"]],
	])("%s", (_case, came, cameAgain, ids) => {
		const presence = createPresence(48);
		for (const id of came) presence.named(id);
		for (const id of cameAgain) {
			presence.done(id);
			presence.named(id);
		}
		expect(presence.ids()).toEqual(ids);
	});
});

describe("whose video is received", () => {
	test.each([
		["fewer than the most", ["a", "b"], 8, ["a", "b"]],
		["the first who came", ["a", "b", "c"], 2, ["a", "b"]],
		["none", ["a"], 0, []],
	])("%s", (_case, ids, most, watched) => {
		expect(watchedOf(ids, most)).toEqual(watched);
	});
});
