import { describe, expect, test } from "bun:test";
import {
	type CatalogState,
	catalogStateAfter,
	namespaceTuple,
	trackNamespaceOf,
} from "../src/core/catalog-track.ts";

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

/** The payloads the rows name. `whole-a` and `whole-b` are catalogs with one track, "a" or "b". */
const PAYLOADS: Record<string, Uint8Array> = {
	"whole-a": utf8(
		'{"version":"draft-01","tracks":[{"name":"a","packaging":"x","isLive":true}]}',
	),
	"whole-b": utf8(
		'{"version":"draft-01","tracks":[{"name":"b","packaging":"x","isLive":true}]}',
	),
	delta: utf8('{"deltaUpdate":[{"op":"remove","tracks":[{"name":"a"}]}]}'),
	"other-version": utf8('{"version":"1","tracks":[]}'),
	broken: utf8("{"),
};

/** The state in a line: the track of the catalog and where it was, then what made it outdated. */
function line(state: CatalogState): string {
	const { current, outdated } = state;
	const catalog =
		current === undefined
			? "none"
			: `${current.catalog.tracks.map((track) => track.name).join(",")} at {${current.at.group}, ${current.at.object}}`;
	return outdated === undefined
		? catalog
		: `${catalog}; outdated by {${outdated.at.group}, ${outdated.at.object}}: ${outdated.because}`;
}

/** Feeds `objects`, each "<group>,<object> <payload>", and gives the state after each that changed it; "=" for one that did not. */
function after(objects: string[]): string[] {
	let state: CatalogState = {};
	return objects.map((each) => {
		const [location, name] = each.split(" ");
		const [group, object] = (location ?? "").split(",");
		const payload = PAYLOADS[name ?? ""];
		if (group === undefined || object === undefined || payload === undefined) {
			throw new Error(`not an Object of this test: ${each}`);
		}
		const next = catalogStateAfter(
			state,
			{ group: BigInt(group), object: BigInt(object) },
			payload,
		);
		if (next === state) return "=";
		state = next;
		return line(state);
	});
}

describe("catalogStateAfter: which catalog a reader has after each Object", () => {
	test.each<{ name: string; objects: string[]; states: string[] }>([
		{
			name: "the first Object of a Group is the catalog",
			objects: ["5,0 whole-a"],
			states: ["a at {5, 0}"],
		},
		{
			name: "the same Object a second time changes nothing (a FETCH and its subscription overlap)",
			objects: ["5,0 whole-a", "5,0 whole-a"],
			states: ["a at {5, 0}", "="],
		},
		{
			name: "a later Group replaces the catalog",
			objects: ["5,0 whole-a", "6,0 whole-b"],
			states: ["a at {5, 0}", "b at {6, 0}"],
		},
		{
			name: "what comes of a Group before the latest is ignored",
			objects: ["6,0 whole-b", "5,0 whole-a", "5,1 delta"],
			states: ["b at {6, 0}", "=", "="],
		},
		{
			name: "a delta update is not applied: the catalog may be outdated",
			objects: ["5,0 whole-a", "5,1 delta"],
			states: ["a at {5, 0}", "a at {5, 0}; outdated by {5, 1}: delta-update"],
		},
		{
			name: "the same delta update a second time changes nothing",
			objects: ["5,0 whole-a", "5,1 delta", "5,1 delta"],
			states: [
				"a at {5, 0}",
				"a at {5, 0}; outdated by {5, 1}: delta-update",
				"=",
			],
		},
		{
			name: "of the Objects of a Group that cannot be used, the one with the largest ID is kept, in whatever order and however often they come",
			objects: [
				"5,0 whole-a",
				"5,1 delta",
				"5,2 delta",
				"5,1 delta",
				"5,2 delta",
			],
			states: [
				"a at {5, 0}",
				"a at {5, 0}; outdated by {5, 1}: delta-update",
				"a at {5, 0}; outdated by {5, 2}: delta-update",
				"=",
				"=",
			],
		},
		{
			name: "a first Object that could not be read and comes again as a catalog is the catalog, and nothing is outdated",
			objects: ["5,0 broken", "5,0 whole-a"],
			states: ["none; outdated by {5, 0}: unreadable", "a at {5, 0}"],
		},
		{
			name: "a first Object that was a delta update and comes again as a catalog is the catalog",
			objects: ["5,0 delta", "5,0 whole-a"],
			states: ["none; outdated by {5, 0}: out-of-place", "a at {5, 0}"],
		},
		{
			name: "the same place with another catalog is not looked at (a publisher that starts again within a subscription)",
			objects: ["0,0 whole-a", "0,0 whole-b"],
			states: ["a at {0, 0}", "="],
		},
		{
			name: "a whole catalog of a later Group ends that",
			objects: ["5,0 whole-a", "5,1 delta", "6,0 whole-b"],
			states: [
				"a at {5, 0}",
				"a at {5, 0}; outdated by {5, 1}: delta-update",
				"b at {6, 0}",
			],
		},
		{
			name: "a later Group whose first Object cannot be read leaves the catalog, as outdated",
			objects: ["5,0 whole-a", "6,0 broken"],
			states: ["a at {5, 0}", "a at {5, 0}; outdated by {6, 0}: unreadable"],
		},
		{
			name: "a catalog of another version is not read",
			objects: ["5,0 whole-a", "6,0 other-version"],
			states: ["a at {5, 0}", "a at {5, 0}; outdated by {6, 0}: unreadable"],
		},
		{
			name: "a delta update as the first Object of a Group is out of place",
			objects: ["5,0 whole-a", "6,0 delta"],
			states: ["a at {5, 0}", "a at {5, 0}; outdated by {6, 0}: out-of-place"],
		},
		{
			name: "a whole catalog that is not the first Object of its Group is out of place",
			objects: ["5,0 whole-a", "5,1 whole-b"],
			states: ["a at {5, 0}", "a at {5, 0}; outdated by {5, 1}: out-of-place"],
		},
		{
			name: "nothing was read yet and the first Object cannot be",
			objects: ["5,0 broken"],
			states: ["none; outdated by {5, 0}: unreadable"],
		},
		{
			name: "a delta update that comes before the first Object of its Group stays unapplied",
			objects: ["5,0 whole-a", "6,1 delta", "6,0 whole-b"],
			states: [
				"a at {5, 0}",
				"a at {5, 0}; outdated by {6, 1}: delta-update",
				"b at {6, 0}; outdated by {6, 1}: delta-update",
			],
		},
		{
			name: "after an Object of a later Group, the catalog of the Group before is not taken",
			objects: ["6,1 delta", "5,0 whole-a"],
			states: ["none; outdated by {6, 1}: delta-update", "="],
		},
	])("$name", ({ objects, states }) => {
		expect(after(objects)).toEqual(states);
	});

	test("the reason an Object could not be read is kept for a log", () => {
		const state = catalogStateAfter({}, { group: 5n, object: 0n }, utf8("{"));
		expect(state.outdated).toEqual({
			at: { group: 5n, object: 0n },
			because: "unreadable",
			detail: "the payload is not JSON in UTF-8",
		});
	});

	test("the state keeps a copy of where the Object was, not the caller's object", () => {
		const payload = PAYLOADS["whole-a"];
		if (payload === undefined) throw new Error("no payload");
		const location = { group: 5n, object: 0n };
		const state = catalogStateAfter({}, location, payload);
		location.group = 9n;
		expect(state.current?.at).toEqual({ group: 5n, object: 0n });
	});

	test("a payload over the length that is read is not a catalog", () => {
		const payload = PAYLOADS["whole-a"];
		if (payload === undefined) throw new Error("no payload");
		const at = { group: 5n, object: 0n };
		expect(
			catalogStateAfter({}, at, payload, payload.length).current,
		).toBeDefined();
		expect(
			catalogStateAfter({}, at, payload, payload.length - 1).outdated?.because,
		).toBe("unreadable");
	});
});

describe("namespaceTuple: the namespace a catalog names with a string", () => {
	const fields = (namespace: string): string[] =>
		namespaceTuple(namespace).map((field) => new TextDecoder().decode(field));

	test.each<{ namespace: string; tuple: string[] }>([
		{ namespace: "mlm/msf/clear", tuple: ["mlm", "msf", "clear"] },
		{
			namespace: "conference.example.com/conference123/alice",
			tuple: ["conference.example.com", "conference123", "alice"],
		},
		{ namespace: "one", tuple: ["one"] },
		{ namespace: "/a//b/", tuple: ["a", "b"] },
		{ namespace: "", tuple: [] },
		{ namespace: "é/日本", tuple: ["é", "日本"] },
	])('"$namespace"', ({ namespace, tuple }) => {
		expect(fields(namespace)).toEqual(tuple);
	});

	test("a track without a namespace is in that of the catalog track, and one with a namespace in its own", () => {
		const ofTheCatalog = namespaceTuple("a/b");
		expect(trackNamespaceOf({}, ofTheCatalog)).toBe(ofTheCatalog);
		expect(trackNamespaceOf({ namespace: "c" }, ofTheCatalog)).toEqual(
			namespaceTuple("c"),
		);
	});
});
