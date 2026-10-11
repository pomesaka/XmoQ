import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
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

	// The rows are fixtures/published-fetch/answers.tsv, which the Rust client
	// reads too: the README there has the columns. Of its rows, a publisher
	// of a catalog can be the ones that keep the newest Group alone.
	const rows = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"fixtures",
			"published-fetch",
			"answers.tsv",
		),
		"utf8",
	)
		.split("\n")
		.filter((row) => row !== "")
		.map((row) => {
			const [
				name,
				first,
				newest,
				firstKept,
				start,
				end,
				order,
				answer,
				endLocation,
				pieces,
			] = row.split("\t");
			if (
				name === undefined ||
				first === undefined ||
				newest === undefined ||
				firstKept === undefined ||
				start === undefined ||
				end === undefined ||
				order === undefined ||
				answer === undefined ||
				endLocation === undefined ||
				pieces === undefined
			) {
				throw new Error(`a row without ten columns: ${row}`);
			}
			return {
				name,
				first,
				newest,
				firstKept,
				start,
				end,
				order,
				answer,
				endLocation,
				pieces,
			};
		});
	const ofACatalog = rows.filter((row) => row.firstKept === row.newest);

	/** "group,object" of the table as two numbers. */
	const location = (text: string): [bigint, bigint] => {
		const [group, object] = text.split(",");
		if (group === undefined || object === undefined) {
			throw new Error(`no Location: ${text}`);
		}
		return [BigInt(group), BigInt(object)];
	};

	test("the table has 47 rows, of which a publisher of a catalog can be 19", () => {
		// WHY counted: a table that is read differently, or a condition that
		// selects nothing, would leave the rows below passing for none.
		expect(rows.length).toBe(47);
		expect(ofACatalog.length).toBe(19);
		expect(ofACatalog.every((row) => row.name.startsWith("catalog-"))).toBe(
			true,
		);
	});

	test.each(ofACatalog)("$name", (row) => {
		if (row.order !== "ascending" && row.order !== "descending") {
			throw new Error(`no Group Order: ${row.order}`);
		}
		// In the table an Object is the number of its Group; the one Object a
		// publisher of a catalog has is the catalog, in the newest Group.
		const expected =
			row.answer === "invalid_range"
				? "INVALID_RANGE"
				: `end {${location(row.endLocation).join(", ")}}: ${
						row.pieces === "-"
							? ""
							: row.pieces
									.split(" ")
									.map((piece) => (piece === row.newest ? "catalog" : piece))
									.join(" ")
					}`;
		expect(
			line(
				BigInt(row.first),
				BigInt(row.newest),
				location(row.start),
				location(row.end),
				row.order === "ascending" ? "Ascending" : "Descending",
			),
		).toBe(expected);
	});
});
