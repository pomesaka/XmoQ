import { describe, expect, test } from "bun:test";
import {
	type Acted,
	actedPayload,
	type Garden,
	gardenPayload,
	readActed,
	readGarden,
	readState,
	type State,
	statePayload,
} from "../src/core/payloads.ts";
import { petRows } from "./fixtures.ts";

/** A JSON column (a string literal, or `[x,y]`), or `-` for none. */
const jsonOf = (column: string | undefined): unknown =>
	column === "-" || column === undefined ? undefined : JSON.parse(column);
const timeOf = (column: string | undefined): number | undefined =>
	column === "-" || column === undefined ? undefined : Number(column);
/** `bubble` and `by`: `-` is null, as the payload writes it. */
const nullableOf = (column: string | undefined): unknown =>
	column === "-" ? null : jsonOf(column);
const bytes = (text: string | undefined): Uint8Array =>
	new TextEncoder().encode(text ?? "");
const text = (payload: Uint8Array): string => new TextDecoder().decode(payload);
/** WHY: the columns are strings, which `toEqual` would not take for the words of `Malformed`. */
const untyped = (read: unknown): unknown => read;

/** The members a table's columns give, with those that are `-` left out. */
function defined(members: Record<string, unknown>): Record<string, unknown> {
	return Object.fromEntries(
		Object.entries(members).filter(([, value]) => value !== undefined),
	);
}

describe("fixtures/pet-tracks", () => {
	const acted = petRows("acted.tsv", 9);
	const garden = petRows("garden.tsv", 7);
	const state = petRows("state.tsv", 14);

	test("the tables have the rows both readers count on", () => {
		expect({
			acted: acted.length,
			garden: garden.length,
			state: state.length,
		}).toEqual({
			acted: 60,
			garden: 44,
			state: 19,
		});
	});

	test.each(acted)(
		"acted: %s",
		(_case, payload, kind, to, said, thing, sentAt, malformed, written) => {
			const read = readActed(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: defined({
					kind,
					to: jsonOf(to),
					text: jsonOf(said),
					thing: timeOf(thing),
					sentAt: timeOf(sentAt),
				}),
			});
			if (!read.ok) return;
			expect(text(actedPayload(read.value)) === payload).toBe(
				written === "yes",
			);
		},
	);

	// WHY the things and events are compared as values, and not as the
	// writer writes them: the column is JSON that `JSON.parse` reads, and
	// the payload written back (column 7) pins the writer's form.
	test.each(garden)(
		"garden: %s",
		(_case, payload, things, events, at, malformed, written) => {
			const read = readGarden(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: {
					things: jsonOf(things),
					events: jsonOf(events),
					at: timeOf(at),
				},
			});
			if (!read.ok) return;
			expect(text(gardenPayload(read.value)) === payload).toBe(
				written === "yes",
			);
		},
	);

	test.each(state)(
		"state: %s",
		(_case, payload, from, to, began, arrives, facing, doing, mood, bubble, by, at, malformed, written) => {
			const read = readState(bytes(payload));
			if (malformed !== "-") {
				expect(untyped(read)).toEqual({ ok: false, why: malformed });
				return;
			}
			expect(untyped(read)).toEqual({
				ok: true,
				value: {
					from: jsonOf(from),
					to: jsonOf(to),
					began: timeOf(began),
					arrives: timeOf(arrives),
					facing,
					doing,
					mood: timeOf(mood),
					bubble: nullableOf(bubble),
					by: nullableOf(by),
					at: timeOf(at),
				},
			});
			if (!read.ok) return;
			expect(text(statePayload(read.value)) === payload).toBe(
				written === "yes",
			);
		},
	);
});

describe("what the tables, being text, cannot hold", () => {
	test.each([
		[
			"acted, not UTF-8",
			[...bytes('{"kind":"talk","text":"'), 0xff, ...bytes('","sentAt":5}')],
		],
		// `serde_json` reads no byte order mark; a TextDecoder drops it unless told not to.
		[
			"acted, a byte order mark",
			[0xef, 0xbb, 0xbf, ...bytes('{"kind":"pet","sentAt":5}')],
		],
	])("%s is no action and no garden", (_case, payload) => {
		const read = Uint8Array.from(payload);
		expect([readActed(read), readGarden(read)]).toEqual([
			{ ok: false, why: "not-an-object" },
			{ ok: false, why: "not-an-object" },
		]);
	});
});

const standing: State = {
	from: [1, 1],
	to: [1, 1],
	began: 1,
	arrives: 1,
	facing: "left",
	doing: "sit",
	mood: 2,
	bubble: null,
	by: null,
	at: 1,
};

describe("what is not written", () => {
	test.each<[string, Acted]>([
		["a throw beyond the floor", { kind: "throw", to: [1001, 0], sentAt: 5 }],
		["a feed beyond the floor", { kind: "feed", to: [0, 1001], sentAt: 5 }],
		["an eat of a thing below 0", { kind: "eat", thing: -1, sentAt: 5 }],
		[
			"a take of a thing with a fraction",
			{ kind: "take", thing: 1.5, sentAt: 5 },
		],
		[
			"a take of a thing beyond 2^53 - 1",
			{ kind: "take", thing: 2 ** 53, sentAt: 5 },
		],
		["a throw to a fraction", { kind: "throw", to: [0.5, 0], sentAt: 5 }],
		["an empty talk", { kind: "talk", text: "", sentAt: 5 }],
		[
			"a talk over 120 bytes",
			{ kind: "talk", text: "a".repeat(121), sentAt: 5 },
		],
		[
			"a talk with a lone surrogate",
			{ kind: "talk", text: "\ud800", sentAt: 5 },
		],
		["a sentAt below 0", { kind: "pet", sentAt: -1 }],
		["a sentAt beyond 2^53 - 1", { kind: "pet", sentAt: 2 ** 53 }],
	])("acted: %s throws a RangeError", (_case, acted) => {
		expect(() => actedPayload(acted)).toThrow(RangeError);
	});

	test.each<[string, Partial<State>]>([
		["from beyond the floor", { from: [0, 1001] }],
		["arrives before it began", { began: 5, arrives: 4 }],
		["a mood over 4", { mood: 5 }],
		["an empty bubble", { bubble: "" }],
		["a bubble over 64 bytes", { bubble: "a".repeat(65) }],
		["by an empty name", { by: "" }],
		["an at with a fraction", { at: 1.5 }],
	])("state: %s throws a RangeError", (_case, change) => {
		expect(() => statePayload({ ...standing, ...change })).toThrow(RangeError);
	});
});

const bowl = {
	id: 1,
	kind: "bowl",
	at: [1, 1],
	by: "alice",
} satisfies Garden["things"][number];
const call = {
	id: 1,
	kind: "call",
	by: "alice",
	text: null,
	at: 5,
} satisfies Garden["events"][number];

describe("a garden that is not written", () => {
	test.each<[string, Garden]>([
		[
			"one thing over the most",
			{ things: Array(9).fill(bowl), events: [], at: 5 },
		],
		[
			"one event over the most",
			{ things: [], events: Array(17).fill(call), at: 5 },
		],
		[
			"a thing beyond the floor",
			{ things: [{ ...bowl, at: [1001, 0] }], events: [], at: 5 },
		],
		[
			"a thing whose id has a fraction",
			{ things: [{ ...bowl, id: 1.5 }], events: [], at: 5 },
		],
		["a thing by nobody", { things: [{ ...bowl, by: "" }], events: [], at: 5 }],
		[
			"an event by nobody",
			{ things: [], events: [{ ...call, by: "" }], at: 5 },
		],
		[
			"a text on a call",
			{ things: [], events: [{ ...call, text: "hi" }], at: 5 },
		],
		[
			"a talk without a text",
			{ things: [], events: [{ ...call, kind: "talk" }], at: 5 },
		],
		[
			"a talk over 120 bytes",
			{
				things: [],
				events: [{ ...call, kind: "talk", text: "a".repeat(121) }],
				at: 5,
			},
		],
		["an at below 0", { things: [], events: [], at: -1 }],
	])("garden: %s throws a RangeError", (_case, garden) => {
		expect(() => gardenPayload(garden)).toThrow(RangeError);
	});
});
