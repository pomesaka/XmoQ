import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type AskAgainPauses,
	createAskAgain,
	createNextSession,
	type SessionEnding,
} from "../../src/core/again.ts";

/**
 * A table of `fixtures/ask-again`, which `crates/again` reads too. WHY an
 * empty row is an error and not passed over: a table one reader takes and
 * the other refuses holds for one of them alone.
 */
function rowsOf(file: string, columns: number): string[][] {
	const text = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"..",
			"fixtures",
			"ask-again",
			file,
		),
		"utf8",
	);
	if (!text.endsWith("\n"))
		throw new Error(`${file} does not end its last row`);
	return text
		.slice(0, -1)
		.split("\n")
		.map((row, index) => {
			const fields = row.split("\t");
			if (fields.length !== columns) {
				throw new Error(
					`${file}, row ${index + 1}: ${fields.length} columns, not ${columns}`,
				);
			}
			return fields;
		});
}

const sameSession = rowsOf("same-session.tsv", 6);
const nextSession = rowsOf("next-session.tsv", 3);

/** A number of the table, or undefined for `-`. */
const cellOf = (field: string | undefined): number | undefined =>
	field === undefined || field === "-" ? undefined : Number(field);

/** The ending a step of `next-session.tsv` names, or undefined for none. */
const endingOf = (word: string): SessionEnding | undefined => {
	switch (word) {
		case "moved":
		case "ended":
		case "failed":
			return word;
		default:
			return undefined;
	}
};

// WHY not a count of rows: a row that is added would have it changed in
// both implementations; a table that is not read is what this guards against.
test("the shared tables are read", () => {
	expect([sameSession.length > 0, nextSession.length > 0]).toEqual([
		true,
		true,
	]);
});

describe("on one session", () => {
	test.each(sameSession)(
		"%s",
		(_name, afterEnd, shortest, mostShortest, mostNamed, steps) => {
			const pauses: Partial<AskAgainPauses> = {};
			const given = [
				["afterEndMs", afterEnd],
				["shortestMs", shortest],
				["mostShortestMs", mostShortest],
				["mostNamedMs", mostNamed],
			] satisfies [keyof AskAgainPauses, string | undefined][];
			for (const [key, field] of given) {
				const value = cellOf(field);
				if (value !== undefined) pauses[key] = value;
			}
			const again = createAskAgain(pauses);
			const expected: string[] = [];
			const seen: string[] = [];
			for (const step of (steps ?? "").split(" ")) {
				expected.push(step);
				if (step === "subscribed") {
					again.subscribed();
					seen.push(step);
					continue;
				}
				const [what, ms] = step.split("=");
				if (what === "ended") {
					seen.push(`ended=${again.afterEnd()}`);
				} else if (what?.startsWith("refused:")) {
					const named = cellOf(what.slice("refused:".length));
					seen.push(`${what}=${again.afterRefusal(named)}`);
				} else {
					throw new Error(`no step ${step} (${ms})`);
				}
			}
			expect(seen).toEqual(expected);
		},
	);
});

describe("the next session", () => {
	// The first try is at 0; each step moves the clock by how long its
	// session lasted from its try (`@N`, 0 where it is not said), and then
	// by the pause before the next try.
	test.each(nextSession)("%s", (_name, againMs, steps) => {
		const next = createNextSession(Number(againMs), 0);
		let now = 0;
		const expected = (steps ?? "").split(" ");
		const seen = expected.map((step) => {
			const [what = "", lasted = "0"] = (step.split("=")[0] ?? "").split("@");
			const ending = endingOf(what);
			if (ending === undefined) throw new Error(`no ending ${step}`);
			now += Number(lasted);
			const pause = next.after(ending, now);
			now += pause;
			return `${step.split("=")[0]}=${pause}`;
		});
		expect(seen).toEqual(expected);
	});
});
