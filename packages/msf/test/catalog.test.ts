import { describe, expect, spyOn, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type Catalog,
	DEFAULT_MAX_CATALOG_LENGTH,
	encodeCatalog,
	readCatalog,
} from "../src/core/catalog.ts";

const FIXTURES = join(import.meta.dir, "..", "..", "..", "fixtures", "msf-01");

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const text = (bytes: Uint8Array): string => new TextDecoder().decode(bytes);

describe("fixtures/msf-01: the catalog of moqlivemock's publisher", () => {
	const read = readCatalog(
		readFileSync(join(FIXTURES, "catalog-moqlivemock.json")),
	);

	test("is read as a catalog of 13 tracks", () => {
		if (read.type !== "catalog") throw new Error(JSON.stringify(read));
		expect(read.catalog.version).toBe("draft-01");
		expect(read.catalog.generatedAt).toBe(1791685571315);
		expect(read.catalog.isComplete).toBeUndefined();
		expect(read.catalog.tracks.map((track) => track.name)).toEqual([
			"video_400kbps_avc",
			"video_600kbps_avc",
			"video_900kbps_avc",
			"video_400kbps_hevc",
			"video_600kbps_hevc",
			"video_900kbps_hevc",
			"video_400kbps_av1",
			"video_600kbps_av1",
			"video_900kbps_av1",
			"audio_monotonic_128kbps_aac",
			"audio_scale_128kbps_aac",
			"audio_monotonic_128kbps_opus",
			"audio_scale_128kbps_opus",
		]);
	});

	// The fields this package reads, and none of the others the catalog
	// has (`lang`).
	test.each([
		{
			index: 0,
			track: {
				name: "video_400kbps_avc",
				packaging: "loc",
				isLive: true,
				role: "video",
				renderGroup: 1,
				altGroup: 1,
				codec: "avc3.4D401F",
				bitrate: 379454,
				width: 1280,
				height: 720,
				framerate: 25,
			},
		},
		{
			index: 12,
			track: {
				name: "audio_scale_128kbps_opus",
				packaging: "loc",
				isLive: true,
				role: "audio",
				renderGroup: 1,
				altGroup: 2,
				codec: "opus",
				bitrate: 135200,
				samplerate: 48000,
				channelConfig: "2",
			},
		},
	])(
		"track $index has the fields that are read, as they are",
		({ index, track }) => {
			if (read.type !== "catalog") throw new Error(JSON.stringify(read));
			expect(read.catalog.tracks[index]).toEqual(track);
		},
	);
});

describe("fixtures/msf-01/catalogs.tsv: what a payload is read as", () => {
	const table = readFileSync(join(FIXTURES, "catalogs.tsv"), "utf8");
	if (!table.endsWith("\n")) throw new Error("no newline at the end");
	const rows = table
		.slice(0, -1)
		.split("\n")
		.map((line) => {
			const [name, outcome, json, ...more] = line.split("\t");
			if (
				name === undefined ||
				outcome === undefined ||
				json === undefined ||
				more.length > 0
			) {
				throw new Error(
					`a row of catalogs.tsv is not what its README has: ${line}`,
				);
			}
			return { name, outcome, json };
		});

	test("the table has 80 rows, each with a name of its own", () => {
		expect(rows.length).toBe(80);
		expect(new Set(rows.map((row) => row.name)).size).toBe(80);
	});

	// §5.2.28 has a JSON Number and no more; `crates/msf` reads the same
	// row to the same value.
	test("a sample rate that is no whole number is kept as it is", () => {
		const row = rows.find(
			(each) => each.name === "samplerate-that-is-no-whole-number",
		);
		if (row === undefined) throw new Error("the row is gone");
		const read = readCatalog(utf8(row.json));
		if (read.type !== "catalog") throw new Error(JSON.stringify(read));
		expect(read.catalog.tracks[0]?.samplerate).toBe(44100.5);
	});

	test.each(rows)("$name: $outcome", ({ outcome, json }) => {
		expect<string>(readCatalog(utf8(json)).type).toBe(outcome);
	});
});

describe("readCatalog", () => {
	test("says which version a catalog of another version has", () => {
		expect(readCatalog(utf8('{"version":"1","tracks":[]}'))).toEqual({
			type: "unsupported-version",
			version: "1",
		});
	});

	test.each([
		{
			name: "a field that is missing",
			json: '{"version":"draft-01","tracks":[{"name":"v","packaging":"loc","isLive":true,"bitrate":1}]}',
			reason: 'track 0: "codec" is missing',
		},
		{
			name: "a field of another type",
			json: '{"version":"draft-01","tracks":[{"name":"v","packaging":"x","isLive":true},{"name":7,"packaging":"x","isLive":true}]}',
			reason: 'track 1: "name" is not a string',
		},
		{
			name: "a name that comes twice",
			json: '{"version":"draft-01","tracks":[{"name":"v","packaging":"x","isLive":true},{"name":"v","packaging":"x","isLive":true}]}',
			reason: 'track 1: the name "v" comes twice in its namespace',
		},
	])("gives the reason for $name", ({ json, reason }) => {
		expect(readCatalog(utf8(json))).toEqual({ type: "invalid", reason });
	});

	test.each([
		{ name: "bytes that are no UTF-8", bytes: [0x7b, 0xff, 0x7d] },
		// The first bytes of a GZIP stream (RFC 1952): a compressed catalog
		// (§12.1) is not read.
		{ name: "nothing", bytes: [] },
	])("$name is no catalog", ({ bytes }) => {
		expect(readCatalog(new Uint8Array(bytes))).toEqual({
			type: "invalid",
			reason: "the payload is no JSON in UTF-8 that serde_json reads",
		});
	});

	test("bytes that are no UTF-8 inside a string are no catalog, also where the rest is one", () => {
		const bytes = utf8('{"version":"draft-01","tracks":[],"label":"?"}');
		bytes[bytes.indexOf(0x3f)] = 0xff;
		expect(readCatalog(bytes)).toEqual({
			type: "invalid",
			reason: "the payload is no JSON in UTF-8 that serde_json reads",
		});
	});

	test("a catalog that says the broadcast is complete is read as that", () => {
		expect(
			readCatalog(utf8('{"version":"draft-01","isComplete":true,"tracks":[]}')),
		).toEqual({
			type: "catalog",
			catalog: { version: "draft-01", isComplete: true, tracks: [] },
		});
	});

	// One reason for each: the JSON reader says no more than that it reads
	// no JSON there, as `serde_json` does not.
	test.each<{ name: string; payload: Uint8Array }>([
		{
			name: "a number that is too large for a double",
			payload: utf8('{"version":"draft-01","generatedAt":1e999,"tracks":[]}'),
		},
		{
			name: "a byte order mark before the document",
			payload: new Uint8Array([
				0xef,
				0xbb,
				0xbf,
				...utf8('{"version":"draft-01","tracks":[]}'),
			]),
		},
		{
			name: "half of a surrogate pair in a name",
			payload: utf8(
				'{"version":"draft-01","tracks":[{"name":"\\udc00x","packaging":"x","isLive":true}]}',
			),
		},
	])("$name makes the document no catalog", ({ payload }) => {
		expect(readCatalog(payload)).toEqual({
			type: "invalid",
			reason: "the payload is no JSON in UTF-8 that serde_json reads",
		});
	});

	test.each([
		{
			name: "that is no array",
			json: '{"deltaUpdate":null}',
			reason: 'the catalog: "deltaUpdate" is not an array',
		},
		{
			name: "without an operation",
			json: '{"deltaUpdate":[]}',
			reason: 'the catalog: "deltaUpdate" has no operation',
		},
		{
			name: "in a document with tracks",
			json: '{"deltaUpdate":[{"op":"add","tracks":[]}],"tracks":[]}',
			reason: 'the catalog: a delta update has "tracks" or "version"',
		},
		{
			name: "in a document with a version",
			json: '{"deltaUpdate":[{"op":"add","tracks":[]}],"version":"draft-01"}',
			reason: 'the catalog: a delta update has "tracks" or "version"',
		},
	])("gives the reason for a `deltaUpdate` $name", ({ json, reason }) => {
		expect(readCatalog(utf8(json))).toEqual({ type: "invalid", reason });
	});

	test("a whole surrogate pair in a name is text", () => {
		expect(
			readCatalog(
				utf8(
					'{"version":"draft-01","tracks":[{"name":"\\ud83d\\ude00","packaging":"x","isLive":true}]}',
				),
			).type,
		).toBe("catalog");
	});

	// 34 bytes.
	const SMALLEST = utf8('{"version":"draft-01","tracks":[]}');

	test.each([
		{ name: "as many bytes as are read", maxLength: 34, type: "catalog" },
		{ name: "one byte more than is read", maxLength: 33, type: "invalid" },
	])("a payload of $name is read as: $type", ({ maxLength, type }) => {
		expect(SMALLEST.length).toBe(34);
		expect<string>(readCatalog(SMALLEST, maxLength).type).toBe(type);
	});

	test("a payload that is too long is refused before it is decoded or parsed", () => {
		const decode = spyOn(TextDecoder.prototype, "decode");
		const parse = spyOn(JSON, "parse");
		try {
			expect(readCatalog(SMALLEST, 10).type).toBe("invalid");
			expect(decode).toHaveBeenCalledTimes(0);
			expect(parse).toHaveBeenCalledTimes(0);
			// The same payload within the length is.
			expect(readCatalog(SMALLEST, 34).type).toBe("catalog");
			expect(decode).toHaveBeenCalledTimes(1);
		} finally {
			decode.mockRestore();
			parse.mockRestore();
		}
	});

	test.each([
		{ name: "NaN", maxLength: Number.NaN },
		{ name: "Infinity", maxLength: Number.POSITIVE_INFINITY },
		{ name: "a negative number", maxLength: -1 },
		{ name: "a number that is no integer", maxLength: 1.5 },
	])(
		"$name is no length to read at most: a RangeError, whatever the payload",
		({ maxLength }) => {
			expect(() => readCatalog(SMALLEST, maxLength)).toThrow(RangeError);
			expect(() => readCatalog(new Uint8Array(0), maxLength)).toThrow(
				RangeError,
			);
		},
	);

	test("the reason for a payload that is too long has both numbers", () => {
		expect(readCatalog(SMALLEST, 10)).toEqual({
			type: "invalid",
			reason: "the payload has 34 bytes, more than the 10 that are read",
		});
	});

	test("without a number of its own, a payload of 1 MiB is read and one byte more is not", () => {
		expect(DEFAULT_MAX_CATALOG_LENGTH).toBe(1048576);
		// Spaces, which JSON allows around a document, up to the length.
		const padded = (length: number): Uint8Array => {
			const bytes = new Uint8Array(length).fill(0x20);
			bytes.set(SMALLEST);
			return bytes;
		};
		expect(readCatalog(padded(1048576)).type).toBe("catalog");
		expect(readCatalog(padded(1048577))).toEqual({
			type: "invalid",
			reason:
				"the payload has 1048577 bytes, more than the 1048576 that are read",
		});
	});

	// The first bytes of a GZIP stream (RFC 1952): a compressed catalog
	// (§12.1) is not read, and the reason says what it looks like.
	test("a payload that begins like a GZIP stream is no catalog, with a reason of its own", () => {
		expect(readCatalog(new Uint8Array([0x1f, 0x8b, 0x08, 0x00]))).toEqual({
			type: "invalid",
			reason:
				"the payload begins like a GZIP stream; compressed catalogs are not read",
		});
	});

	test("a track keeps its encryption scheme, so that it can be told apart", () => {
		const read = readCatalog(
			utf8(
				'{"version":"draft-01","tracks":[{"name":"v","packaging":"x","isLive":true,"encryptionScheme":"moq-secure-objects"}]}',
			),
		);
		if (read.type !== "catalog") throw new Error(JSON.stringify(read));
		expect(read.catalog.tracks[0]?.encryptionScheme).toBe("moq-secure-objects");
	});

	test("a track keeps a namespace it has", () => {
		const read = readCatalog(
			utf8(
				'{"version":"draft-01","tracks":[{"name":"v","namespace":"a/b","packaging":"x","isLive":false}]}',
			),
		);
		expect(read).toEqual({
			type: "catalog",
			catalog: {
				version: "draft-01",
				tracks: [
					{ name: "v", namespace: "a/b", packaging: "x", isLive: false },
				],
			},
		});
	});
});

describe("encodeCatalog", () => {
	const VIDEO: Catalog = {
		version: "draft-01",
		generatedAt: 1791685571315,
		tracks: [
			{
				name: "video",
				packaging: "loc",
				isLive: true,
				role: "video",
				codec: "avc3.42E01F",
				bitrate: 1000000,
				width: 1280,
				height: 720,
				framerate: 30,
			},
		],
	};

	test("writes the JSON of the catalog", () => {
		expect(text(encodeCatalog(VIDEO))).toBe(
			'{"version":"draft-01","tracks":[{"name":"video","packaging":"loc","isLive":true,"role":"video","codec":"avc3.42E01F","bitrate":1000000,"width":1280,"height":720,"framerate":30}],"generatedAt":1791685571315}',
		);
	});

	test("what it writes is read as the catalog it was given", () => {
		expect(readCatalog(encodeCatalog(VIDEO))).toEqual({
			type: "catalog",
			catalog: VIDEO,
		});
	});

	test("does not pass on a field this package does not read", () => {
		const withMore = { ...VIDEO, tracks: [], secret: "x" };
		expect(text(encodeCatalog(withMore))).toBe(
			'{"version":"draft-01","tracks":[],"generatedAt":1791685571315}',
		);
	});

	test.each<{ name: string; catalog: Catalog; reason: string }>([
		{
			name: "a LOC track without a codec",
			catalog: {
				version: "draft-01",
				tracks: [{ name: "video", packaging: "loc", isLive: true, bitrate: 1 }],
			},
			reason: 'track 0: "codec" is missing',
		},
		{
			name: "a name that comes twice",
			catalog: {
				version: "draft-01",
				tracks: [
					{ name: "a", packaging: "x", isLive: true },
					{ name: "a", packaging: "x", isLive: true },
				],
			},
			reason: 'track 1: the name "a" comes twice in its namespace',
		},
		{
			name: "a name with a percent outside a variable reference (§5.4.1)",
			catalog: {
				version: "draft-01",
				tracks: [{ name: "100%", packaging: "x", isLive: true }],
			},
			reason:
				'the catalog: "100%" has a "%" outside a variable reference (§5.4.1)',
		},
		{
			name: "a bitrate that JSON has no number for",
			catalog: {
				version: "draft-01",
				tracks: [
					{
						name: "video",
						packaging: "loc",
						isLive: true,
						codec: "avc3.42E01F",
						bitrate: Number.NaN,
					},
				],
			},
			reason: 'track 0: "bitrate" is not a number',
		},
		{
			name: "a track with an encryption scheme",
			catalog: {
				version: "draft-01",
				tracks: [
					{
						name: "video",
						packaging: "x",
						isLive: true,
						encryptionScheme: "moq-secure-objects",
					},
				],
			},
			reason:
				'the track "video" has an encryption scheme, which is not written',
		},
	])("refuses $name with a RangeError", ({ catalog, reason }) => {
		expect(() => encodeCatalog(catalog)).toThrow(new RangeError(reason));
	});
});
