import { describe, expect, test } from "bun:test";
import type { Catalog } from "../src/core/catalog.ts";
import {
	answerCatalogFetch,
	type CatalogPublication,
	firstPublication,
	nextPublication,
} from "../src/core/catalog-publication.ts";

const text = (bytes: Uint8Array): string => new TextDecoder().decode(bytes);
const utf8 = (value: string): Uint8Array => new TextEncoder().encode(value);
/** The namespace of the catalog track in these tests. */
const NAMESPACE = [utf8("xmoq"), utf8("demo")];

const VIDEO = {
	name: "video",
	packaging: "loc",
	isLive: true,
	codec: "avc3.42E01F",
	bitrate: 1000000,
};
const catalogOf = (...tracks: Catalog["tracks"]): Catalog => ({
	version: "draft-01",
	tracks,
});

describe("firstPublication", () => {
	test("the catalog is Object 0 of the Group it is given, with its payload", () => {
		const publication = firstPublication(
			catalogOf(VIDEO),
			1760140800000n,
			NAMESPACE,
		);
		expect(publication.at).toEqual({ group: 1760140800000n, object: 0n });
		expect(publication.firstGroup).toBe(1760140800000n);
		expect(text(publication.payload)).toBe(
			'{"version":"draft-01","tracks":[{"name":"video","packaging":"loc","isLive":true,"codec":"avc3.42E01F","bitrate":1000000}]}',
		);
	});

	test.each([
		{ name: "a negative number", group: -1n },
		{ name: "2^64", group: 18446744073709551616n },
	])("$name is no Group ID", ({ group }) => {
		expect(() => firstPublication(catalogOf(), group, NAMESPACE)).toThrow(
			RangeError,
		);
	});

	test("a catalog that is not written is refused", () => {
		expect(() =>
			firstPublication(
				catalogOf({ name: "video", packaging: "loc", isLive: true }),
				0n,
				NAMESPACE,
			),
		).toThrow(new RangeError('track 0: "codec" is missing'));
	});
});

describe("nextPublication", () => {
	const first = firstPublication(catalogOf(VIDEO), 7n, NAMESPACE);

	test("the next catalog is Object 0 of the next Group, and the first Group stays known", () => {
		const next = nextPublication(
			first,
			catalogOf(VIDEO, { ...VIDEO, name: "b" }),
		);
		expect(next.at).toEqual({ group: 8n, object: 0n });
		expect(next.firstGroup).toBe(7n);
		expect(nextPublication(next, catalogOf(VIDEO)).at).toEqual({
			group: 9n,
			object: 0n,
		});
	});

	test.each<{ name: string; before: Catalog; after: Catalog }>([
		{
			name: "a track is added",
			before: catalogOf(VIDEO),
			after: catalogOf(VIDEO, { ...VIDEO, name: "b" }),
		},
		{
			name: "a track is removed",
			before: catalogOf(VIDEO),
			after: catalogOf(),
		},
		{
			name: "a track that was live is not live any more",
			before: catalogOf(VIDEO),
			after: catalogOf({ ...VIDEO, isLive: false }),
		},
		{
			name: "the broadcast is complete",
			before: catalogOf(VIDEO),
			after: { ...catalogOf(), isComplete: true },
		},
		{
			name: "the broadcast stays complete",
			before: { ...catalogOf(), isComplete: true },
			after: { ...catalogOf(), isComplete: true },
		},
		{
			name: "a track of the same name in another namespace has other attributes",
			before: catalogOf(VIDEO),
			after: catalogOf(VIDEO, { ...VIDEO, namespace: "other", bitrate: 5 }),
		},
		{
			name: "the same track with its fields in another order",
			before: catalogOf(VIDEO),
			after: catalogOf({
				bitrate: 1000000,
				codec: "avc3.42E01F",
				isLive: true,
				packaging: "loc",
				name: "video",
			}),
		},
		{
			name: "the same track with the namespace of the catalog track spelled out",
			before: catalogOf(VIDEO),
			after: catalogOf({ ...VIDEO, namespace: "xmoq/demo" }),
		},
		{
			name: "the same track with that namespace no longer spelled out",
			before: catalogOf({ ...VIDEO, namespace: "xmoq/demo" }),
			after: catalogOf(VIDEO),
		},
		{
			name: "a complete broadcast loses a track",
			before: { ...catalogOf(VIDEO), isComplete: true },
			after: { ...catalogOf(), isComplete: true },
		},
		{
			name: "only the time the catalog was made differs",
			before: { ...catalogOf(VIDEO), generatedAt: 1 },
			after: { ...catalogOf(VIDEO), generatedAt: 2 },
		},
	])("may follow: $name", ({ before, after }) => {
		const from = firstPublication(before, 0n, NAMESPACE);
		expect(nextPublication(from, after).at.group).toBe(1n);
	});

	test.each<{ name: string; before: Catalog; after: Catalog; reason: string }>([
		{
			name: "the broadcast was complete and is not any more",
			before: { ...catalogOf(), isComplete: true },
			after: catalogOf(),
			reason:
				"the catalog before said the broadcast is complete, and this one does not",
		},
		{
			name: "a track that was not live is live again",
			before: catalogOf({ ...VIDEO, isLive: false }),
			after: catalogOf(VIDEO),
			reason: 'the track "video" was not live, and is live in this catalog',
		},
		{
			name: "a track has another codec",
			before: catalogOf(VIDEO),
			after: catalogOf({ ...VIDEO, codec: "avc3.640028" }),
			reason:
				'the track "video" has other attributes than it was declared with',
		},
		{
			name: "a track has a field more",
			before: catalogOf(VIDEO),
			after: catalogOf({ ...VIDEO, width: 1280 }),
			reason:
				'the track "video" has other attributes than it was declared with',
		},
		{
			name: "a track has a field less",
			before: catalogOf({ ...VIDEO, width: 1280 }),
			after: catalogOf(VIDEO),
			reason:
				'the track "video" has other attributes than it was declared with',
		},
		{
			name: "a track has another codec, and the namespace of the catalog track spelled out",
			before: catalogOf(VIDEO),
			after: catalogOf({
				...VIDEO,
				namespace: "xmoq/demo",
				codec: "avc3.640028",
			}),
			reason:
				'the track "video" has other attributes than it was declared with',
		},
		{
			name: "a complete broadcast gets a track",
			before: { ...catalogOf(), isComplete: true },
			after: { ...catalogOf(VIDEO), isComplete: true },
			reason:
				'the catalog before said the broadcast is complete, and the track "video" is new',
		},
		{
			name: "one name twice in the namespace of the catalog track, once with it spelled out",
			before: catalogOf(),
			after: catalogOf(VIDEO, { ...VIDEO, namespace: "xmoq/demo" }),
			reason:
				'the name "video" comes twice in the namespace of the catalog track, once with that namespace spelled out',
		},
		{
			name: "the catalog is not written",
			before: catalogOf(VIDEO),
			after: catalogOf(VIDEO, VIDEO),
			reason: 'track 1: the name "video" comes twice in its namespace',
		},
	])("may not follow: $name", ({ before, after, reason }) => {
		const from = firstPublication(before, 0n, NAMESPACE);
		expect(() => nextPublication(from, after)).toThrow(new RangeError(reason));
	});
});

describe("nextPublication compares what is published, not what it was given", () => {
	test("a field that is not written makes no difference", () => {
		const from = firstPublication(
			{ version: "draft-01", tracks: [{ ...VIDEO, ...{ trackDuration: 1 } }] },
			0n,
			NAMESPACE,
		);
		const after: Catalog = {
			version: "draft-01",
			tracks: [{ ...VIDEO, ...{ trackDuration: 2 } }],
		};
		expect(nextPublication(from, after).at.group).toBe(1n);
	});

	test("the catalog a publication has is a copy: changing what was given changes nothing, and the rules still hold", () => {
		const track = { ...VIDEO };
		const given = { version: "draft-01" as const, tracks: [track] };
		const from = firstPublication(given, 0n, NAMESPACE);
		track.codec = "avc3.640028";
		expect(from.catalog.tracks[0]?.codec).toBe("avc3.42E01F");
		// The same object, with another codec now: refused.
		expect(() => nextPublication(from, given)).toThrow(
			new RangeError(
				'the track "video" has other attributes than it was declared with',
			),
		);
	});

	test("what a publication has can be given back as the next catalog", () => {
		const from = firstPublication(catalogOf(VIDEO), 0n, NAMESPACE);
		expect(nextPublication(from, from.catalog).at.group).toBe(1n);
	});

	test("a catalog over the length a reader takes is not published", () => {
		const long: Catalog = {
			version: "draft-01",
			tracks: [{ ...VIDEO, name: "v".repeat(1048576) }],
		};
		expect(() => firstPublication(long, 0n, NAMESPACE)).toThrow(RangeError);
		const from = firstPublication(catalogOf(VIDEO), 0n, NAMESPACE);
		expect(() => nextPublication(from, long)).toThrow(RangeError);
	});

	test("the Group after the last one there is, is no Group", () => {
		const last = firstPublication(
			catalogOf(VIDEO),
			18446744073709551615n,
			NAMESPACE,
		);
		expect(() => nextPublication(last, catalogOf(VIDEO))).toThrow(
			new RangeError("18446744073709551616 is no Group ID"),
		);
	});
});

describe("answerCatalogFetch: what a FETCH gets from a publisher that keeps the newest catalog alone", () => {
	/** A publisher whose first catalog was in Group `first` and whose newest is in Group `newest`. */
	function publisher(first: bigint, newest: bigint): CatalogPublication {
		let publication = firstPublication(catalogOf(VIDEO), first, NAMESPACE);
		while (publication.at.group < newest) {
			publication = nextPublication(publication, catalogOf(VIDEO));
		}
		return publication;
	}

	/** The answer in a line: the End Location, then the pieces; `u<group>,<object>` for an End of Unknown Range. */
	function line(
		first: bigint,
		newest: bigint,
		start: [bigint, bigint],
		end: [bigint, bigint],
		groupOrder: "Ascending" | "Descending",
	): string {
		const answer = answerCatalogFetch(publisher(first, newest), {
			startLocation: { group: start[0], object: start[1] },
			endLocation: { group: end[0], object: end[1] },
			groupOrder,
		});
		if (answer.type === "invalid-range") return "INVALID_RANGE";
		const pieces = answer.pieces.map((piece) =>
			piece === "catalog"
				? "catalog"
				: `u${piece.unknownThrough.group},${piece.unknownThrough.object}`,
		);
		return `end {${answer.endLocation.group}, ${answer.endLocation.object}}: ${pieces.join(" ")}`;
	}

	// `first` and `newest` are the Groups of the first and the newest catalog.
	test.each<{
		name: string;
		first: bigint;
		newest: bigint;
		start: [bigint, bigint];
		end: [bigint, bigint];
		order: "Ascending" | "Descending";
		answer: string;
	}>([
		{
			name: "the range a Joining FETCH with offset 0 makes: the newest Group, up to its one Object",
			first: 5n,
			newest: 5n,
			start: [5n, 0n],
			end: [5n, 1n],
			order: "Ascending",
			answer: "end {5, 1}: catalog",
		},
		{
			name: "the whole newest Group: the End Location is the Largest Object plus 1",
			first: 5n,
			newest: 5n,
			start: [5n, 0n],
			end: [5n, 0n],
			order: "Ascending",
			answer: "end {5, 1}: catalog",
		},
		{
			name: "a range that goes on beyond the newest Group",
			first: 5n,
			newest: 5n,
			start: [5n, 0n],
			end: [9n, 0n],
			order: "Ascending",
			answer: "end {5, 1}: catalog",
		},
		{
			name: "a range that starts before the first Group: nothing was published there, and nothing is said of it",
			first: 5n,
			newest: 5n,
			start: [0n, 0n],
			end: [5n, 1n],
			order: "Ascending",
			answer: "end {5, 1}: catalog",
		},
		{
			name: "a range that ends before the first Group: an answer with nothing on the stream",
			first: 5n,
			newest: 5n,
			start: [0n, 0n],
			end: [3n, 0n],
			order: "Ascending",
			answer: "end {3, 0}: ",
		},
		{
			name: "a range that starts after the catalog Object",
			first: 5n,
			newest: 5n,
			start: [5n, 1n],
			end: [6n, 0n],
			order: "Ascending",
			answer: "INVALID_RANGE",
		},
		{
			name: "a range that starts in a later Group",
			first: 5n,
			newest: 5n,
			start: [6n, 0n],
			end: [7n, 0n],
			order: "Ascending",
			answer: "INVALID_RANGE",
		},
		{
			name: "Ascending, a range that starts before the first Group: the indicator covers that as well",
			first: 5n,
			newest: 8n,
			start: [0n, 0n],
			end: [8n, 1n],
			order: "Ascending",
			answer: "end {8, 1}: u7,0 catalog",
		},
		{
			name: "catalogs that were replaced are unknown: the indicator stands before the catalog, at Object 0 of the last of them",
			first: 5n,
			newest: 8n,
			start: [5n, 0n],
			end: [8n, 1n],
			order: "Ascending",
			answer: "end {8, 1}: u7,0 catalog",
		},
		{
			name: "a range within what was replaced: the indicator ends where the range does",
			first: 5n,
			newest: 8n,
			start: [5n, 0n],
			end: [5n, 1n],
			order: "Ascending",
			answer: "end {5, 1}: u5,0",
		},
		{
			name: "a whole Group of what was replaced",
			first: 5n,
			newest: 8n,
			start: [6n, 0n],
			end: [6n, 0n],
			order: "Ascending",
			// Object 0 of the Group, also where the whole Group was asked for.
			answer: "end {6, 0}: u6,0",
		},
		{
			name: "a range among the Groups that were replaced that has none of their Objects: no indicator",
			first: 5n,
			newest: 8n,
			start: [6n, 3n],
			end: [6n, 4n],
			order: "Ascending",
			answer: "end {6, 4}: ",
		},
		{
			name: "a range that ends before the first Group, with catalogs replaced: no indicator",
			first: 5n,
			newest: 8n,
			start: [0n, 0n],
			end: [3n, 0n],
			order: "Ascending",
			answer: "end {3, 0}: ",
		},
		{
			name: "a range that starts after the last catalog that was replaced",
			first: 5n,
			newest: 8n,
			start: [7n, 1n],
			end: [8n, 1n],
			order: "Ascending",
			answer: "end {8, 1}: catalog",
		},
		{
			name: "Descending: the catalog first, and the indicator after it, at Object 0 of the first Group that was replaced",
			first: 5n,
			newest: 8n,
			start: [5n, 0n],
			end: [8n, 1n],
			order: "Descending",
			answer: "end {8, 1}: catalog u5,0",
		},
		{
			name: "Descending, from within the first Group: the indicator names the next one",
			first: 5n,
			newest: 8n,
			start: [5n, 1n],
			end: [8n, 1n],
			order: "Descending",
			answer: "end {8, 1}: catalog u6,0",
		},
		{
			name: "Descending, a range that starts before the first Group: the indicator does not go below it",
			first: 5n,
			newest: 8n,
			start: [0n, 0n],
			end: [8n, 1n],
			order: "Descending",
			answer: "end {8, 1}: catalog u5,0",
		},
		{
			name: "Descending, with nothing replaced",
			first: 5n,
			newest: 5n,
			start: [0n, 0n],
			end: [5n, 1n],
			order: "Descending",
			answer: "end {5, 1}: catalog",
		},
		{
			name: "Descending, within what was replaced",
			first: 5n,
			newest: 8n,
			start: [5n, 0n],
			end: [6n, 0n],
			order: "Descending",
			answer: "end {6, 0}: u5,0",
		},
	])("$name", ({ first, newest, start, end, order, answer }) => {
		expect(line(first, newest, start, end, order)).toBe(answer);
	});
});
