// The catalog of draft-ietf-moq-msf-01 §5: a JSON document that names the
// tracks of a broadcast. Section numbers are of that draft.

/**
 * The one MSF version that is written and read. §5.1.1 has "follow the
 * convention of specifying the version as "draft-XX"". The examples of the
 * draft (§5.6) all have "1", which is not read here.
 */
export const MSF_VERSION = "draft-01";

/** The `packaging` of a track whose Objects are LOC (§4.1). */
export const LOC_PACKAGING = "loc";

/**
 * A track of a catalog (§5.2): the fields this package reads. A catalog can
 * have others, which are ignored (§5: "A parser MUST ignore fields it does
 * not understand").
 */
export interface CatalogTrack {
	/** §5.2.3. Unique per namespace within a catalog. */
	readonly name: string;
	/**
	 * §5.2.2, as the string the catalog has. Left out, the track is in the
	 * namespace of the catalog track.
	 */
	readonly namespace?: string;
	/** §5.2.4. `LOC_PACKAGING` for audio and video. */
	readonly packaging: string;
	/** §5.2.7: whether new Objects will be added to the track. */
	readonly isLive: boolean;
	/** §5.2.6. */
	readonly role?: string;
	/**
	 * §5.2.11: "An integer specifying a group of tracks which are designed
	 * to be rendered together", audio with its video.
	 */
	readonly renderGroup?: number;
	/**
	 * §5.2.12: "An integer specifying a group of tracks which are alternate
	 * versions of one-another. ... Alternate tracks MUST have matching media
	 * time sequences. A subscriber typically subscribes to one track from a
	 * set". §4.2: such tracks "MUST be time-aligned". That it is an integer
	 * is not checked: its JSON type is Number.
	 */
	readonly altGroup?: number;
	/** §5.2.18: a codec string of the WebCodecs registry. With LOC packaging it is there. */
	readonly codec?: string;
	/** §5.2.22: the maximum bitrate, in bits per second. With LOC packaging it is there. */
	readonly bitrate?: number;
	/** §5.2.26, in pixels. */
	readonly width?: number;
	/** §5.2.27, in pixels. */
	readonly height?: number;
	/** §5.2.20, in frames per second. */
	readonly framerate?: number;
	/**
	 * §5.2.28: "The number of audio frame samples per second." The draft
	 * asks for it, and for `channelConfig`, on "tracks for which audio codecs
	 * are specified"; this reads and writes them where they are there, and
	 * asks for them nowhere (`checkTrack` has why).
	 */
	readonly samplerate?: number;
	/**
	 * §5.2.29: "A string specifying the audio channel configuration", a
	 * string "to provide the flexibility to describe complex channel
	 * configurations". The draft's examples and `moqlivemock` have the
	 * number of channels in it, `"2"`; no other form is said.
	 */
	readonly channelConfig?: string;
	/**
	 * §5.2.38: the Objects of the track are encrypted with this scheme.
	 * §4.3.1: "subscribers MUST decrypt the object payload using the
	 * specified scheme before processing." Nothing here decrypts, so a track
	 * that has this cannot be played; it is read so that it can be told
	 * apart, and a catalog that has it is not written.
	 */
	readonly encryptionScheme?: string;
}

/** An independent catalog (§5.1): one that needs no earlier catalog Object to be read. */
export interface Catalog {
	readonly version: typeof MSF_VERSION;
	/** §5.1.2: when the catalog was made, in milliseconds since the Unix epoch. */
	readonly generatedAt?: number;
	/**
	 * §5.1.3: the broadcast is complete. Only ever `true`: "This field MUST
	 * NOT be included if it is FALSE."
	 */
	readonly isComplete?: true;
	readonly tracks: readonly CatalogTrack[];
}

/** What the payload of a catalog Object is. */
export type CatalogRead =
	| { type: "catalog"; catalog: Catalog }
	/**
	 * A delta update (§5.1.6, §5.3), which changes an earlier catalog of
	 * its Group. It is told apart and not applied.
	 *
	 * FIXME: apply delta updates. A publisher that sends them cannot be
	 * followed past its first one.
	 */
	| { type: "delta-update" }
	/**
	 * A catalog of another MSF version. §5.1.1: "A subscriber MUST NOT
	 * attempt to parse a catalog version which it does not understand."
	 */
	| { type: "unsupported-version"; version: string }
	/** No catalog of this version: `reason` says what is wrong, for a log. */
	| { type: "invalid"; reason: string };

/** A value of the type while the check builds it. */
type Building<T> = { -readonly [Key in keyof T]: T[Key] };

/** Thrown inside the check, and turned into `invalid` at its one exit. */
class Refused extends Error {}

type JsonObject = Record<string, unknown>;

const isObject = (value: unknown): value is JsonObject =>
	typeof value === "object" && value !== null && !Array.isArray(value);
// A string with half of a surrogate pair is no text: it cannot be written as
// UTF-8, so a track name like that would be subscribed to under another
// name than the catalog has. JSON can spell one ("\ud800"), and
// `JSON.parse` takes it.
const HALF_A_PAIR =
	/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;
const isString = (value: unknown): value is string =>
	typeof value === "string" && !HALF_A_PAIR.test(value);
// WHY finite: `JSON.parse` reads a number too large for a double, as 1e999
// is, as Infinity, which JSON has no number for and `encodeCatalog` could
// not write back.
const isNumber = (value: unknown): value is number =>
	typeof value === "number" && Number.isFinite(value);
const isBoolean = (value: unknown): value is boolean =>
	typeof value === "boolean";

/** A field that is of the JSON type the draft gives it, or absent. */
function optional<T>(
	object: JsonObject,
	where: string,
	key: string,
	is: (value: unknown) => value is T,
	type: string,
): T | undefined {
	const value = object[key];
	if (value === undefined) return undefined;
	if (!is(value)) throw new Refused(`${where}: "${key}" is not a ${type}`);
	return value;
}

/** A field with "Required: Yes", or one a MUST asks for here. */
function required<T>(
	object: JsonObject,
	where: string,
	key: string,
	is: (value: unknown) => value is T,
	type: string,
): T {
	const value = optional(object, where, key, is, type);
	if (value === undefined) throw new Refused(`${where}: "${key}" is missing`);
	return value;
}

function checkTrack(value: unknown, where: string): CatalogTrack {
	if (!isObject(value)) throw new Refused(`${where} is not an object`);
	const track: Building<CatalogTrack> = {
		name: required(value, where, "name", isString, "string"),
		packaging: required(value, where, "packaging", isString, "string"),
		isLive: required(value, where, "isLive", isBoolean, "boolean"),
	};
	// WHY LOC packaging is taken for "an audio or a video track": §5.2.18
	// asks for a codec on "tracks which have an inherent codec associated
	// with them (e.g., audio and video tracks)" and §5.2.22 for a bitrate on
	// "audio and video tracks", and no field says which tracks those are
	// (`role` is optional). A LOC track holds one encoded audio or video
	// sample an Object (§4.1), so it is one.
	// WHY NOT `samplerate` and `channelConfig` (§5.2.28, §5.2.29) as well:
	// they are for "tracks for which audio codecs are specified", and
	// telling an audio codec string from a video one takes the registry.
	// They are read where they are there, with the types the draft gives.
	const media = track.packaging === LOC_PACKAGING ? required : optional;
	const fields = {
		namespace: optional(value, where, "namespace", isString, "string"),
		role: optional(value, where, "role", isString, "string"),
		renderGroup: optional(value, where, "renderGroup", isNumber, "number"),
		altGroup: optional(value, where, "altGroup", isNumber, "number"),
		codec: media(value, where, "codec", isString, "string"),
		bitrate: media(value, where, "bitrate", isNumber, "number"),
		width: optional(value, where, "width", isNumber, "number"),
		height: optional(value, where, "height", isNumber, "number"),
		framerate: optional(value, where, "framerate", isNumber, "number"),
		samplerate: optional(value, where, "samplerate", isNumber, "number"),
		channelConfig: optional(value, where, "channelConfig", isString, "string"),
		encryptionScheme: optional(
			value,
			where,
			"encryptionScheme",
			isString,
			"string",
		),
	};
	if (fields.namespace !== undefined) track.namespace = fields.namespace;
	if (fields.role !== undefined) track.role = fields.role;
	if (fields.renderGroup !== undefined) track.renderGroup = fields.renderGroup;
	if (fields.altGroup !== undefined) track.altGroup = fields.altGroup;
	if (fields.codec !== undefined) track.codec = fields.codec;
	if (fields.bitrate !== undefined) track.bitrate = fields.bitrate;
	if (fields.width !== undefined) track.width = fields.width;
	if (fields.height !== undefined) track.height = fields.height;
	if (fields.framerate !== undefined) track.framerate = fields.framerate;
	if (fields.samplerate !== undefined) track.samplerate = fields.samplerate;
	if (fields.channelConfig !== undefined) {
		track.channelConfig = fields.channelConfig;
	}
	if (fields.encryptionScheme !== undefined) {
		track.encryptionScheme = fields.encryptionScheme;
	}
	return track;
}

/**
 * What a parsed JSON value is as a catalog. The one place the rules of the
 * draft are applied: what is read and what is written both pass it.
 */
/**
 * §5.4.1: "Variables are denoted by enclosing the variable name in percent
 * characters (%). Variable names MUST consist of alphanumeric characters,
 * hyphens, and underscores", and "The percent character (%) MUST NOT appear
 * in catalog field values except as part of a variable reference". A string
 * of nothing but other characters and such references. WHY "alphanumeric"
 * is ASCII: the draft says no more, and a reference is made to be put in
 * place of. WHY a name of no characters (`%%`) is none: "consist of" names
 * no empty one. `crates/msf` reads the same.
 */
const OF_VARIABLES = /^(?:[^%]|%[A-Za-z0-9_-]+%)*$/;

/**
 * The first string value of the document that holds a `%` outside a
 * variable reference (`OF_VARIABLES`), or undefined. Every value is looked at,
 * those of fields that are not read among them; keys are no field values.
 */
function stringWithPercent(value: unknown): string | undefined {
	if (typeof value === "string") {
		return OF_VARIABLES.test(value) ? undefined : value;
	}
	const inside = Array.isArray(value)
		? value
		: isObject(value)
			? Object.values(value)
			: [];
	for (const each of inside) {
		const found = stringWithPercent(each);
		if (found !== undefined) return found;
	}
	return undefined;
}

function check(value: unknown): CatalogRead {
	if (!isObject(value)) {
		return { type: "invalid", reason: "the catalog is not a JSON object" };
	}
	// Before anything else of the document, so that both readers turn down
	// the same documents for it (`crates/msf`). Variables are not put in place
	// of: a reference is read as it is.
	const percent = stringWithPercent(value);
	if (percent !== undefined) {
		return {
			type: "invalid",
			reason: `the catalog: ${JSON.stringify(percent.slice(0, 40))} has a "%" outside a variable reference (§5.4.1)`,
		};
	}
	// §5.1.6: "If this field is present, the catalog represents a delta (or
	// partial) update", and its JSON type is an Array. Before the version,
	// which a delta update does not have.
	const { deltaUpdate } = value;
	if (deltaUpdate !== undefined) {
		// WHY `null` is no absent field, here as in every other field: a
		// whole catalog that is read as a delta update could not be used,
		// and a reader that took `null` for "absent" would read the same
		// document in another way than one that does not.
		if (!Array.isArray(deltaUpdate)) {
			return {
				type: "invalid",
				reason: 'the catalog: "deltaUpdate" is not an array',
			};
		}
		// §5.3: "A delta update MUST include the Delta Update ... field with
		// at least one operation. It MUST NOT contain an instance of a Tracks
		// ... field or an MSF version ... field." The operations themselves
		// are not looked at.
		if (deltaUpdate.length === 0) {
			return {
				type: "invalid",
				reason: 'the catalog: "deltaUpdate" has no operation',
			};
		}
		if (value.tracks !== undefined || value.version !== undefined) {
			return {
				type: "invalid",
				reason: 'the catalog: a delta update has "tracks" or "version"',
			};
		}
		return { type: "delta-update" };
	}
	try {
		const version = required(
			value,
			"the catalog",
			"version",
			isString,
			"string",
		);
		if (version !== MSF_VERSION)
			return { type: "unsupported-version", version };
		const read: CatalogTrack[] = [];
		const catalog: Building<Catalog> = { version: MSF_VERSION, tracks: read };
		const generatedAt = optional(
			value,
			"the catalog",
			"generatedAt",
			isNumber,
			"number",
		);
		if (generatedAt !== undefined) catalog.generatedAt = generatedAt;
		const isComplete = optional(
			value,
			"the catalog",
			"isComplete",
			isBoolean,
			"boolean",
		);
		// §5.1.3: "This field MUST NOT be included if it is FALSE."
		if (isComplete === false) {
			throw new Refused('the catalog: "isComplete" is false');
		}
		if (isComplete) catalog.isComplete = true;
		const { tracks } = value;
		if (!Array.isArray(tracks)) {
			throw new Refused(
				tracks === undefined
					? 'the catalog: "tracks" is missing'
					: 'the catalog: "tracks" is not an array',
			);
		}
		// §5.2.3: "Within the catalog, track names MUST be unique per
		// namespace." A track without a namespace is in the catalog's own,
		// which is not known here: two tracks of one name, one with that
		// namespace spelled out and one without, are not found.
		const names = new Set<string>();
		for (const [index, each] of tracks.entries()) {
			const track = checkTrack(each, `track ${index}`);
			const key = JSON.stringify([track.namespace ?? null, track.name]);
			if (names.has(key)) {
				throw new Refused(
					`track ${index}: the name "${track.name}" comes twice in its namespace`,
				);
			}
			names.add(key);
			read.push(track);
		}
		return { type: "catalog", catalog };
	} catch (error) {
		if (error instanceof Refused)
			return { type: "invalid", reason: error.message };
		throw error;
	}
}

// WHY fatal: a catalog is JSON (§5), which is UTF-8 between systems
// (RFC 8259 §8.1). Bytes that are not would be read as other text.
// WHY the byte order mark is kept, so that `JSON.parse` refuses it: the same
// section has "MUST NOT add a byte order mark" for a writer and only "MAY
// ignore" for a parser, and parsers differ (serde_json of Rust refuses one).
// A document that one implementation of ours reads and the other does not
// is worse than one both refuse.
const utf8Decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const utf8Encoder = new TextEncoder();

/**
 * The most bytes of a payload `readCatalog` reads when it is given no other
 * number. Provisional.
 *
 * WHY a limit: the payload is parsed whole, and the parsed document takes a
 * multiple of its bytes, so a reader has to say how much it takes from a
 * publisher it does not know. An Object of MoQ Transport can be far larger
 * than any catalog.
 *
 * WHY 1 MiB: the catalog of `moqlivemock`'s publisher is 2,715 bytes for 13
 * tracks (`fixtures/msf-01/`), about 200 bytes a track, so this is room
 * for some 5,000 tracks of that kind, or for a few with initialization
 * data (§5.1.7) of tens of kilobytes each. Not measured against a catalog
 * that has such data.
 */
export const DEFAULT_MAX_CATALOG_LENGTH = 1024 * 1024;

/**
 * Throws a `RangeError` for a number that is no length to read at most: one
 * that is not an integer, or below 0. WHY not taken as it is: every length is
 * "not more than" NaN and Infinity, which would be no limit at all, and the
 * number is the caller's, not the peer's.
 */
export function checkMaxLength(maxLength: number): void {
	if (!Number.isSafeInteger(maxLength) || maxLength < 0) {
		throw new RangeError(
			`the most bytes of a catalog to read is ${maxLength}, which is no length`,
		);
	}
}

/**
 * Reads the payload of a catalog Object. Never throws for a payload: one
 * that is no catalog comes from the peer. One of more than `maxLength` bytes
 * is `invalid`, and not parsed. Throws a `RangeError` for a `maxLength` that
 * is no length (`checkMaxLength`).
 *
 * `null` is a value of another type in every field, and never "absent". A
 * key that comes twice is not noticed: `JSON.parse` keeps the later one.
 *
 * Not looked at: compression (§12.1; a compressed payload is `invalid`, as
 * bytes that are no JSON), and where `initDataList` stands in the document
 * (§5.1.7).
 */
export function readCatalog(
	payload: Uint8Array,
	maxLength: number = DEFAULT_MAX_CATALOG_LENGTH,
): CatalogRead {
	checkMaxLength(maxLength);
	if (payload.length > maxLength) {
		return {
			type: "invalid",
			reason: `the payload has ${payload.length} bytes, more than the ${maxLength} that are read`,
		};
	}
	// RFC 1952 §2.3.1: a GZIP member begins with 1f 8b. No JSON text does,
	// so this is only another reason for the same outcome: whether a payload
	// is compressed is said by a property (§12.1) whose number the draft
	// does not give, and is not known here.
	if (payload[0] === 0x1f && payload[1] === 0x8b) {
		return {
			type: "invalid",
			reason:
				"the payload begins like a GZIP stream; compressed catalogs are not read",
		};
	}
	let value: unknown;
	try {
		value = JSON.parse(utf8Decoder.decode(payload), refuseWhatParsersDiffOn);
	} catch (error) {
		if (error instanceof Refused) {
			return { type: "invalid", reason: error.message };
		}
		return { type: "invalid", reason: "the payload is not JSON in UTF-8" };
	}
	return check(value);
}

/**
 * A reviver of `JSON.parse` that refuses, anywhere in the document, a key or
 * a string with half of a surrogate pair and a number too large for a
 * double. WHY: RFC 8259 §8.2 has "the behavior of software that receives
 * JSON texts containing such values is unpredictable" of a string like
 * that, and §6 a number beyond a double as one that does not interoperate:
 * parsers differ on both, and serde_json, which `crates/msf` reads with,
 * refuses the whole document for either. A document that one reader of
 * ours reads and the other does not is worse than one both refuse. WHY NOT
 * read such a document in Rust as `JSON.parse` does: that would take a
 * JSON parser of our own there.
 */
function refuseWhatParsersDiffOn(key: string, value: unknown): unknown {
	if (HALF_A_PAIR.test(key)) {
		throw new Refused("a key has half of a surrogate pair");
	}
	if (typeof value === "string" && HALF_A_PAIR.test(value)) {
		throw new Refused("a string has half of a surrogate pair");
	}
	if (typeof value === "number" && !Number.isFinite(value)) {
		throw new Refused("a number is too large for a double");
	}
	return value;
}

/**
 * The payload of a catalog Object for an independent catalog. Throws a
 * `RangeError` for a catalog the draft does not allow, with the reason
 * `readCatalog` would give for it.
 *
 * A value that is no `Catalog` by its type and that `JSON.stringify`
 * refuses (one that contains itself, a BigInt) makes that throw its
 * `TypeError`. The length of what is written is not limited.
 */
export function encodeCatalog(catalog: Catalog): Uint8Array {
	// Through JSON, so that what is checked is what a reader gets: a field
	// that is `undefined` is left out, and one that is no JSON is not there.
	const text = JSON.stringify(catalog);
	const read = check(JSON.parse(text));
	switch (read.type) {
		case "catalog": {
			// §5.2.39 asks for a cipher suite with an encryption scheme, and
			// the keys have fields of their own; none of it is implemented.
			const encrypted = read.catalog.tracks.find(
				(track) => track.encryptionScheme !== undefined,
			);
			if (encrypted !== undefined) {
				throw new RangeError(
					`the track "${encrypted.name}" has an encryption scheme, which is not written`,
				);
			}
			// What was checked is written, and nothing else: a field of the
			// argument that this package does not read is not passed on.
			return utf8Encoder.encode(JSON.stringify(read.catalog));
		}
		case "invalid":
			throw new RangeError(read.reason);
		case "unsupported-version":
			throw new RangeError(
				`the version "${read.version}" is not ${MSF_VERSION}`,
			);
		case "delta-update":
			throw new RangeError("a delta update is not written");
	}
}
