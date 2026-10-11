// What every revision of LOC gives the rest of this repository, whatever
// its numbers are: the shapes, and the functions a revision has to have.
// Nothing here is of one revision; what is, is in the directory that has
// the name of that revision (`src/draft-04/`), and `src/index.ts` chooses.

/**
 * One Object Property as MoQ Transport carries it (a Key-Value-Pair): an
 * even type has an integer of up to 64 bits, an odd type has bytes. The
 * same shape as `KeyValuePair` of @xmoq/moqt, which this package does not
 * depend on.
 */
export interface LocProperty {
	readonly type: bigint;
	readonly value: bigint | Uint8Array;
}

/** When a frame is, as LOC has it. */
export interface LocTiming {
	/**
	 * In the unit `timescale` gives. Without one: "wall-clock time in
	 * microseconds since the Unix epoch" (§2.3.1.1).
	 */
	readonly timestamp: bigint;
	/**
	 * The units of `timestamp` a second has. With it, "the Timestamp
	 * represents media time rather than wall-clock time. The epoch or anchor
	 * point for the timestamp is application-defined" (§2.3.1.2).
	 */
	readonly timescale?: bigint;
}

/** What the Object Properties of an Object say of its time. */
export type LocTimingRead =
	/** `timing` is absent where the Object has no Timestamp: §2.2 has the properties as "optional metadata". */
	| { type: "ok"; timing: LocTiming | undefined }
	/** The properties are not ones LOC writes; `reason` is for a log. */
	| { type: "invalid"; reason: string };

/** What became of a video payload that was to have start codes. */
export type AnnexBPayload =
	| {
			type: "ok";
			/** The payload with a start code of 4 bytes before every NAL unit. */
			payload: Uint8Array;
			/** How many length prefixes were replaced; 0 for a payload that had start codes already. */
			replaced: number;
	  }
	/** The payload is in neither form; `reason` is for a log. */
	| { type: "invalid"; reason: string };

/**
 * One revision of LOC, as far as it is implemented: what `src/index.ts`
 * hands on under the names of this package.
 */
export interface LocRevision {
	/** The name of the document, with its revision: "draft-ietf-moq-loc-04". */
	readonly name: string;
	/** The Object Properties for a frame, in the order MoQ Transport writes them in. Throws a `RangeError` for a number that is no value of a property. */
	locProperties(timing: LocTiming): LocProperty[];
	/** Reads the time of a frame out of the Object Properties of an Object. */
	readLocTiming(properties: readonly LocProperty[]): LocTimingRead;
	/** The time of a frame in microseconds. */
	microsecondsOf(timing: LocTiming): bigint;
	/** A video payload with a start code of 4 bytes before every NAL unit. */
	toAnnexB(payload: Uint8Array): AnnexBPayload;
	/** Whether a video payload begins with a start code of 4 bytes and has something after it: the form `toAnnexB` gives, and leaves as it is. */
	beginsWithStartCode(payload: Uint8Array): boolean;
}
