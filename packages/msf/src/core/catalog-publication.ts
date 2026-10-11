// What a publisher of a catalog track holds and answers with
// (draft-ietf-moq-msf-01 §5, §6.1): the newest Group, which is one Object.
// No I/O and no clock; `src/moqt` writes the streams. Section numbers are
// of that draft unless they say "transport", which is
// draft-ietf-moq-transport-18.

import {
	type Catalog,
	type CatalogTrack,
	DEFAULT_MAX_CATALOG_LENGTH,
	encodeCatalog,
	readCatalog,
} from "./catalog.ts";
import { type CatalogLocation, trackNamespaceOf } from "./catalog-track.ts";

/** The catalog a publisher offers: Object 0 of the newest Group. */
export interface CatalogPublication {
	/**
	 * The catalog as it is published: what a reader of `payload` gets, and
	 * not the object the caller gave, which can have other fields and can
	 * change.
	 */
	readonly catalog: Catalog;
	/** Where it is: Object 0 of its Group. */
	readonly at: CatalogLocation;
	/** The payload of that Object. */
	readonly payload: Uint8Array;
	/** The namespace of the catalog track, which a track without `namespace` is in (§5.2.2). */
	readonly namespace: readonly Uint8Array[];
	/**
	 * The Group of the first catalog of this publisher. The catalogs from
	 * there to the Group before `at` were published and are not kept.
	 */
	readonly firstGroup: bigint;
}

/** The largest number MoQ Transport can carry (transport §1.4.1). */
const MAX_GROUP = 2n ** 64n - 1n;

/** The key a track is known by: the namespace it is in, field by field, and its name. */
const trackKey = (
	track: CatalogTrack,
	catalogNamespace: readonly Uint8Array[],
): string =>
	JSON.stringify([
		trackNamespaceOf(track, catalogNamespace).map((field) => Array.from(field)),
		track.name,
	]);

/**
 * The catalog as Object 0 of `group`, checked. Throws a `RangeError` for a
 * catalog `encodeCatalog` does not write, for a Group that is no Group ID,
 * and for two tracks of one name in one namespace where `encodeCatalog`
 * cannot tell: one that names the namespace of the catalog track and one
 * that names none (§5.2.3: "track names MUST be unique per namespace").
 */
function publication(
	catalog: Catalog,
	group: bigint,
	namespace: readonly Uint8Array[],
	firstGroup: bigint,
): CatalogPublication {
	if (group < 0n || group > MAX_GROUP) {
		throw new RangeError(`${group} is no Group ID`);
	}
	const payload = encodeCatalog(catalog);
	// WHY the limit of a reader: a catalog over it is one no reader of this
	// package takes, and it would be published all the same.
	if (payload.length > DEFAULT_MAX_CATALOG_LENGTH) {
		throw new RangeError(
			`the catalog is ${payload.length} bytes, and ${DEFAULT_MAX_CATALOG_LENGTH} are read`,
		);
	}
	// What was written, read: every comparison is of what a reader gets.
	const read = readCatalog(payload, payload.length);
	if (read.type !== "catalog") {
		throw new Error(`a catalog that was written is read as ${read.type}`);
	}
	const keys = new Set<string>();
	for (const track of read.catalog.tracks) {
		const key = trackKey(track, namespace);
		if (keys.has(key)) {
			throw new RangeError(
				`the name "${track.name}" comes twice in the namespace of the catalog track, once with that namespace spelled out`,
			);
		}
		keys.add(key);
	}
	return {
		catalog: read.catalog,
		at: { group, object: 0n },
		payload,
		namespace,
		firstGroup,
	};
}

/**
 * The first catalog of a publisher, as Object 0 of `group`. Throws a
 * `RangeError` for a catalog that is not published (`publication`).
 *
 * §6.1: "When a publisher restarts ... it MUST ensure the new starting
 * Group ID is greater than any previously published Group ID for that
 * track. One approach is to use the current wall clock time in milliseconds
 * since the Unix epoch as the starting Group ID." The caller gives the
 * Group; nothing here knows of an earlier run.
 */
export function firstPublication(
	catalog: Catalog,
	group: bigint,
	namespace: readonly Uint8Array[],
): CatalogPublication {
	return publication(catalog, group, namespace, group);
}

/** Everything of a track but `isLive`, in one string. The tracks are ones that were read, whose fields are in one order. */
const attributesOf = (track: CatalogTrack): string =>
	JSON.stringify({ ...track, isLive: undefined });

/**
 * The catalog that replaces `before`: Object 0 of the next Group (§5: "As
 * soon as an independent update is produced, it MUST be placed at the start
 * of a new Group"; §6.1: "each subsequent Group ID SHOULD increase by 1").
 *
 * Throws a `RangeError`, and nothing is published, for a catalog that is
 * not published (`publication`), and for one that may not follow `before`.
 * The two are compared as a reader gets them: the order of the fields, and
 * fields that are not written, make no difference.
 *
 * - §5.1.3, `isComplete`: "This field MUST NOT be removed from a catalog
 *   once it has been added", and it is "a commitment that all tracks are
 *   complete, no new tracks will be added to the catalog": a catalog after
 *   one that had it has no track the one before did not have.
 * - §5.2.7, `isLive`: "A True value MUST never follow a False value."
 * - §5.3: "The tuple of Track Namespace and Track Name defines a fixed set
 *   of Track attributes which MUST NOT be modified after being declared. To
 *   modify any attribute, a new track with a different Namespace|Name tuple
 *   is created ... and then the old track is removed." WHY `isLive` is left
 *   out of what may not change: §5.2.7 has it go from true to false for a
 *   track that ended, so it is no attribute in that sense. The sentence
 *   stands among the rules for delta updates; it is applied to whole
 *   catalogs as well, where a reader would see the same change.
 *
 * A track is the same one when it has the same name in the same namespace,
 * whether the namespace is spelled out or is the catalog track's. A track
 * that was removed and comes back is compared with nothing: only the
 * catalog before is known here.
 */
export function nextPublication(
	before: CatalogPublication,
	catalog: Catalog,
): CatalogPublication {
	const next = publication(
		catalog,
		before.at.group + 1n,
		before.namespace,
		before.firstGroup,
	);
	const complete = before.catalog.isComplete === true;
	if (complete && next.catalog.isComplete !== true) {
		throw new RangeError(
			"the catalog before said the broadcast is complete, and this one does not",
		);
	}
	const declared = new Map(
		before.catalog.tracks.map((track) => [
			trackKey(track, before.namespace),
			track,
		]),
	);
	for (const track of next.catalog.tracks) {
		const was = declared.get(trackKey(track, before.namespace));
		if (was === undefined) {
			if (complete) {
				throw new RangeError(
					`the catalog before said the broadcast is complete, and the track "${track.name}" is new`,
				);
			}
			continue;
		}
		if (!was.isLive && track.isLive) {
			throw new RangeError(
				`the track "${track.name}" was not live, and is live in this catalog`,
			);
		}
		// `namespace` is the one field that can differ for one track: left
		// out, or the catalog track's spelled out.
		if (
			attributesOf({ ...was, namespace: undefined }) !==
			attributesOf({ ...track, namespace: undefined })
		) {
			throw new RangeError(
				`the track "${track.name}" has other attributes than it was declared with`,
			);
		}
	}
	return next;
}

/** 2^64 - 1: the Object ID of the last Location a Group has. */
const LAST_OBJECT = 2n ** 64n - 1n;

const isAfter = (a: CatalogLocation, b: CatalogLocation): boolean =>
	a.group > b.group || (a.group === b.group && a.object > b.object);

/** How a publisher of a catalog answers a Standalone FETCH for its track. */
export type CatalogFetchAnswer =
	/** REQUEST_ERROR INVALID_RANGE. */
	| { type: "invalid-range" }
	| {
			type: "serve";
			/** The End Location of the FETCH_OK. */
			endLocation: CatalogLocation;
			/**
			 * What goes onto the fetch stream, in order: the catalog Object, and
			 * an End of Unknown Range indicator with this Location.
			 */
			pieces: ("catalog" | { unknownThrough: CatalogLocation })[];
	  };

/**
 * What a FETCH for the range gets, from a publisher that keeps the newest
 * Group alone. `endLocation` is as in a FETCH: the last Object plus 1, and
 * Object 0 for a whole Group.
 *
 * The rule is the one of the Rust client that answers from what it
 * published (docs/moqt-implementation-notes.md, "A publisher that answers a
 * Standalone FETCH from what it published"), for a publisher that keeps one
 * Object:
 *
 * - Transport §10.12.3: "If ... Start Location is greater than the Largest
 *   Object ... the publisher MUST return REQUEST_ERROR with error code
 *   INVALID_RANGE."
 * - Transport §10.13: the End Location is the one that was asked for,
 *   "unless the requested range extends beyond published data", and then
 *   the Largest Object plus 1.
 * - An Object that does not exist is left out, with no indicator (transport
 *   §10.12.3: "Any gaps in the Group and Object IDs in the response stream
 *   indicate objects that do not exist"; §11.4.4.2 has a publisher not use
 *   End of Non-Existent Range for that).
 * - The catalogs that were replaced were published and are not kept: where
 *   one of them lies in the range, one End of Unknown Range indicator
 *   covers them. **It names a Location that was published and let go of**:
 *   Ascending, Object 0 of the last such Group of the range, and it stands
 *   before the catalog; Descending, Object 0 of the first such Group, and
 *   it stands after (the same notes, "Where a publisher puts an End of
 *   Unknown Range indicator in a Descending fetch"; provisional there).
 *   A range that has none of those Objects gets no indicator, also where it
 *   lies among their Groups ({6, 3} to {6, 4}).
 *
 * **In Ascending order the indicator also covers what was never
 * published**: it says of everything from the start of the range to its
 * Location that it is not known, and a range can start before the first
 * catalog of this publisher. That is not false, and what the Rust client
 * does. In Descending order the indicator does not go below the first
 * Group.
 */
export function answerCatalogFetch(
	publication: CatalogPublication,
	range: {
		startLocation: CatalogLocation;
		endLocation: CatalogLocation;
		groupOrder: "Ascending" | "Descending";
	},
): CatalogFetchAnswer {
	const largest = publication.at;
	const start = range.startLocation;
	if (isAfter(start, largest)) return { type: "invalid-range" };
	// The last Location the End Location covers.
	const asked: CatalogLocation =
		range.endLocation.object === 0n
			? { group: range.endLocation.group, object: LAST_OBJECT }
			: {
					group: range.endLocation.group,
					object: range.endLocation.object - 1n,
				};
	// Transport §10.13: the End Location that was asked for, unless that is
	// beyond the Largest Object.
	const endLocation = isAfter(asked, largest)
		? { group: largest.group, object: largest.object + 1n }
		: range.endLocation;
	// The Groups whose catalog was let go of and lies in the range: from
	// the first Group whose Object 0 is not before the Start Location, and
	// not before the first Group there was, to the Group before the newest,
	// and not beyond the range.
	const fromStart = start.object === 0n ? start.group : start.group + 1n;
	const from =
		fromStart > publication.firstGroup ? fromStart : publication.firstGroup;
	const lastLetGo = largest.group - 1n;
	const to = lastLetGo < asked.group ? lastLetGo : asked.group;
	const unknown = (group: bigint) =>
		from <= to ? [{ unknownThrough: { group, object: 0n } }] : [];
	const catalog: "catalog"[] = isAfter(largest, asked) ? [] : ["catalog"];
	if (range.groupOrder === "Ascending") {
		return {
			type: "serve",
			endLocation,
			pieces: [...unknown(to), ...catalog],
		};
	}
	return {
		type: "serve",
		endLocation,
		pieces: [...catalog, ...unknown(from)],
	};
}
