// The catalog track of draft-ietf-moq-msf-01 §5 as a sequence of Objects:
// which catalog a reader has after each of them. No I/O; `src/moqt` feeds
// it the Objects of a subscription. Section numbers are of that draft.

import { type Catalog, type CatalogTrack, readCatalog } from "./catalog.ts";

/** Where an Object stands in its track. The same shape as a Location of MoQ Transport. */
export interface CatalogLocation {
	readonly group: bigint;
	readonly object: bigint;
}

/**
 * What a reader of a catalog track knows.
 *
 * Locations are compared within one subscription only: a publisher that
 * starts again need not go on from where it was (the catalog of
 * `moqlivemock`'s publisher is always Object 0 of Group 0).
 */
export interface CatalogState {
	/**
	 * The last whole catalog that was read, and where it was: Object 0 of
	 * its Group.
	 */
	readonly current?: {
		readonly catalog: Catalog;
		readonly at: CatalogLocation;
	};
	/**
	 * The largest Group an Object was seen of. What comes of a Group before
	 * it is ignored (§5: "Any catalog updates that precede the first Object
	 * of the latest Group MUST be ignored").
	 */
	readonly latestGroup?: bigint;
	/**
	 * Something came after `current`, or before any, that changes the
	 * catalog and could not be used: `current` may no longer be what the
	 * publisher offers. Gone when a whole catalog of a later Group is read.
	 *
	 * WHY `current` stays, where §5 has what precedes the latest Group
	 * ignored: a viewer that plays would lose every track for one update it
	 * cannot read. Whoever uses the state decides.
	 */
	readonly outdated?: {
		/**
		 * The Object that could not be used; where several of one Group
		 * could not, the one with the largest Object ID.
		 */
		readonly at: CatalogLocation;
		/**
		 * `delta-update`: a delta update, which is not applied. `unreadable`:
		 * no catalog of the version that is read. `out-of-place`: an Object
		 * that breaks §5, "The first Object (with Object ID 0) in any Group
		 * ... MUST hold an independent copy of the catalog. All subsequent
		 * Objects within that Group ... MUST hold a delta update."
		 */
		readonly because: "delta-update" | "unreadable" | "out-of-place";
		/** For a log. */
		readonly detail: string;
	};
}

/**
 * The state after an Object of the catalog track. Returns `state` itself
 * where the Object changes nothing: one of a Group before the latest, the
 * catalog that is held, and an Object that cannot be used where one of its
 * Group with the same or a larger Object ID could not be used either.
 * Objects can come in any order and more than once: a Joining FETCH and its
 * subscription overlap where a publisher sends the newest Object on both,
 * and each Object can travel on a stream of its own (§6).
 *
 * An Object is known by where it stands, not by what it holds: one that
 * comes a second time at a place that was seen is not looked at, also when
 * it is another one. A publisher that starts again at the same Location
 * within one subscription is not noticed.
 */
export function catalogStateAfter(
	state: CatalogState,
	location: CatalogLocation,
	payload: Uint8Array,
	maxLength?: number,
): CatalogState {
	const { latestGroup } = state;
	if (latestGroup !== undefined && location.group < latestGroup) return state;
	// A copy: the caller's object can change after the call.
	const at: CatalogLocation = {
		group: location.group,
		object: location.object,
	};
	const first = at.object === 0n;
	// The catalog of this Group is the one that is held.
	if (first && state.current?.at.group === at.group) return state;

	const outdated = (
		because: NonNullable<CatalogState["outdated"]>["because"],
		detail: string,
	): CatalogState => {
		const before = state.outdated?.at;
		// WHY only forwards within a Group: the Objects of a Group that
		// cannot be used come in any order and more than once, and each of
		// them going back and forth would be a new state every time.
		if (before?.group === at.group && at.object <= before.object) return state;
		return {
			...state,
			latestGroup: at.group,
			outdated: { at, because, detail },
		};
	};

	const read = readCatalog(payload, maxLength);
	if (read.type === "unsupported-version") {
		return outdated("unreadable", `a catalog of the version "${read.version}"`);
	}
	if (read.type === "invalid") return outdated("unreadable", read.reason);
	if (first) {
		if (read.type === "delta-update") {
			return outdated("out-of-place", "Object 0 of a Group is a delta update");
		}
		// WHY `outdated` is kept where it is about a later Object of this
		// Group: a delta update came before the first Object did, and is
		// still not applied. One about the first Object itself, which came
		// before in a form that was not read, is over with this catalog.
		const later =
			state.outdated !== undefined &&
			state.outdated.at.group === at.group &&
			state.outdated.at.object > 0n;
		return {
			current: { catalog: read.catalog, at },
			latestGroup: at.group,
			...(later ? { outdated: state.outdated } : {}),
		};
	}
	if (read.type === "catalog") {
		return outdated(
			"out-of-place",
			`Object ${at.object} of a Group is a whole catalog`,
		);
	}
	return outdated("delta-update", "a delta update, which is not applied");
}

const utf8Encoder = new TextEncoder();

/**
 * The Track Namespace a catalog names with a string, as the fields of a
 * tuple: split at "/", and an empty field left out.
 *
 * The draft does not say how the string of `namespace` (§5.2.2) becomes the
 * tuple of MoQ Transport; §11.1.2 has an encoding for URLs only. Its
 * examples have "conference.example.com/conference123/alice", and
 * `moqlivemock` splits at "/" and drops empty fields
 * (`internal/namespace.go`, v0.16.1), which this follows. A namespace with a
 * field that has a "/" in it, or that is no UTF-8, cannot be named this way.
 *
 * The result can be empty, and can have more fields or longer ones than MoQ
 * Transport takes: whoever subscribes with it finds out.
 */
export function namespaceTuple(namespace: string): Uint8Array[] {
	return namespace
		.split("/")
		.filter((field) => field.length > 0)
		.map((field) => utf8Encoder.encode(field));
}

/**
 * The Track Namespace of a track of a catalog: its own where it has one,
 * and that of the catalog track otherwise (§5.2.2: "each track MUST inherit
 * the namespace of the catalog track").
 */
export function trackNamespaceOf(
	track: Pick<CatalogTrack, "namespace">,
	catalogNamespace: Uint8Array[],
): Uint8Array[] {
	return track.namespace === undefined
		? catalogNamespace
		: namespaceTuple(track.namespace);
}
