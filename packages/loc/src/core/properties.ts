// The properties of draft-ietf-moq-loc-04 §2.3 that say when a frame is:
// what goes into the Object Properties of MoQ Transport, and what is read
// out of them. Section numbers are of that draft.

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

/** §2.3.1.1, ID 0x10: "Timestamp of the encoded media frame encoded as vi64." */
export const TIMESTAMP = 0x10n;
/** §2.3.1.2, ID 0x08: "The number of Timestamp units per second, encoded as vi64." */
export const TIMESCALE = 0x08n;

/** The largest integer a property carries. */
const MAX_VALUE = 2n ** 64n - 1n;

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

/**
 * The Object Properties for a frame, in ascending order of type, which is
 * the order MoQ Transport writes them in. Throws a `RangeError` for a
 * number that is no value of a property, and for a timescale of 0.
 */
export function locProperties(timing: LocTiming): LocProperty[] {
	const isValue = (value: bigint): boolean => value >= 0n && value <= MAX_VALUE;
	if (!isValue(timing.timestamp)) {
		throw new RangeError(`${timing.timestamp} is no timestamp of LOC`);
	}
	const { timescale } = timing;
	if (timescale === undefined) {
		return [{ type: TIMESTAMP, value: timing.timestamp }];
	}
	if (timescale === 0n || !isValue(timescale)) {
		throw new RangeError(`${timescale} is no timescale of LOC`);
	}
	return [
		{ type: TIMESCALE, value: timescale },
		{ type: TIMESTAMP, value: timing.timestamp },
	];
}

/** What the Object Properties of an Object say of its time. */
export type LocTimingRead =
	/** `timing` is absent where the Object has no Timestamp: §2.2 has the properties as "optional metadata". */
	| { type: "ok"; timing: LocTiming | undefined }
	/** The properties are not ones LOC writes; `reason` is for a log. */
	| { type: "invalid"; reason: string };

/**
 * Reads the Timestamp and the Timescale out of the Object Properties of an
 * Object. **Of an Object alone**: §6.1 has the Timescale with the scope
 * "Track, Object", and one that is a property of the track is not known
 * here. A caller that has it puts it into the `LocTiming` it gets, before
 * `microsecondsOf`; a Timestamp without it reads as wall-clock time.
 *
 * Every other property is passed over: the other four of LOC (Video
 * Config, Video Frame Marking, Audio Config, Audio Level) are not read, and
 * a property LOC does not define is not LOC's.
 *
 * `invalid`, where the draft says nothing of a receiver: a Timestamp or a
 * Timescale that comes twice, a Timescale of 0, and a Timescale without a
 * Timestamp, which gives a unit to nothing. A value is not looked at
 * beyond that: what the caller hands in as an integer is handed out, also
 * one MoQ Transport cannot carry (below 0, or 2^64 and above).
 */
export function readLocTiming(
	properties: readonly LocProperty[],
): LocTimingRead {
	let timestamp: bigint | undefined;
	let timescale: bigint | undefined;
	for (const { type, value } of properties) {
		if (type !== TIMESTAMP && type !== TIMESCALE) continue;
		const name = type === TIMESTAMP ? "Timestamp" : "Timescale";
		// Both have an even ID, so MoQ Transport carries an integer; bytes
		// are what a caller made up.
		if (typeof value !== "bigint") {
			return { type: "invalid", reason: `the ${name} is no integer` };
		}
		if ((type === TIMESTAMP ? timestamp : timescale) !== undefined) {
			return { type: "invalid", reason: `the ${name} comes twice` };
		}
		if (type === TIMESTAMP) timestamp = value;
		else timescale = value;
	}
	if (timescale === 0n) {
		return { type: "invalid", reason: "the Timescale is 0" };
	}
	if (timestamp === undefined) {
		return timescale === undefined
			? { type: "ok", timing: undefined }
			: { type: "invalid", reason: "a Timescale without a Timestamp" };
	}
	return {
		type: "ok",
		timing: timescale === undefined ? { timestamp } : { timestamp, timescale },
	};
}

/**
 * The time of a frame in microseconds, the unit of `timestamp` in WebCodecs:
 * since the Unix epoch without a timescale, and since whatever the
 * application counts from with one. Rounded down. A `LocTiming` that was
 * read or written here has no timescale of 0; one made by hand with it is
 * a `RangeError`, and a negative one gives a negative time.
 */
export function microsecondsOf(timing: LocTiming): bigint {
	return timing.timescale === undefined
		? timing.timestamp
		: (timing.timestamp * 1_000_000n) / timing.timescale;
}
