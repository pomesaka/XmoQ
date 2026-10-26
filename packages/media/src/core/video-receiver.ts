// The receiving side of a LOC video track, up to where a decoder begins:
// the Objects of a subscription go in as they come, and the frames come out
// in the order a decoder takes them, each a key frame or one that follows
// the frame before it. No I/O, no clock and no WebCodecs.
//
// "LOC" is draft-ietf-moq-loc-04 and "MSF" draft-ietf-moq-msf-01.

import {
	type LocProperty,
	microsecondsOf,
	readLocTiming,
	toAnnexB,
} from "@xmoq/loc";
import { MAX_DECODER_TIME } from "./decoder-time.ts";

/**
 * An Object of the track as a subscription delivers it. The shape of
 * `ReceivedObject` of @xmoq/moqt, which this package does not depend on
 * here: a test hands one of those in.
 */
export interface VideoObject {
	readonly groupId: bigint;
	readonly object: {
		readonly objectId: bigint;
		readonly properties: readonly LocProperty[];
		readonly status: "Normal" | "EndOfGroup" | "EndOfTrack";
		readonly payload: Uint8Array;
	};
}

/** Why an Object did not become a frame. */
export type VideoDropReason =
	/** Its place was passed: the frame was handed on, or its Group given up or over. A second copy is this as well. */
	| "late"
	/** It was held for a key frame that did not come before a later Group began. */
	| "no-key-frame"
	/** A frame before it in its Group did not come before a later Group began, or before the held Objects were too many. */
	| "gap"
	/** A frame before it in its Group could not be used, so nothing after that in the Group can be decoded. */
	| "after-unusable"
	/** The publisher said that nothing exists where it is: an End of Group or an End of Track before it. */
	| "after-end"
	/** It has no time, or its properties or its payload are not LOC's; `detail` says which. */
	| "unusable"
	/** Too much was held, and its Group was the oldest that had no key frame yet. */
	| "overflow"
	/** The caller gave its Group up, as the decoder failed on a frame of it (`abandonGroup`). */
	| "decoder-failed"
	/** The caller gave its Group up, as the decoder did not keep up (`abandonGroup`). */
	| "decoder-behind";

export type VideoReceiverEvent =
	| {
			type: "frame";
			group: bigint;
			object: bigint;
			/**
			 * Whether a decoder can begin with it: Object 0 of its Group. That
			 * Object 0 is a key frame is taken as a promise of the sender, and
			 * nothing in the payload is looked at here. No draft has it as a
			 * rule: MSF §4.1 has a group of pictures in one Group, transport
			 * draft 18 §2.3 that Groups "SHOULD be independently useful", and
			 * LOC-04 §4.2 says it of its example, where "shall" is no key word.
			 */
			key: boolean;
			/**
			 * In microseconds: since the Unix epoch, or with a Timescale since
			 * whatever the sender counts from (LOC-04 §2.3.1). At most 2^53 - 1,
			 * so that it is a number a decoder of WebCodecs takes as it is.
			 */
			timestamp: bigint;
			/** The payload with a start code before every NAL unit (`toAnnexB` of @xmoq/loc). */
			data: Uint8Array;
	  }
	| {
			type: "dropped";
			group: bigint;
			object: bigint;
			because: VideoDropReason;
			/** With "unusable": what was wrong, for a log. */
			detail?: string;
	  }
	/**
	 * The publisher said that the track ends here: no Object exists at this
	 * Location or after it (transport draft 18 §11.2.1.1, End of Track).
	 * Given when it arrives, whatever has come before it: frames before the
	 * Location can still follow.
	 */
	| { type: "end-of-track"; group: bigint; object: bigint };

/**
 * How many Objects are held at most while one before them is missing.
 * **Provisional**: about a second of video at 30 frames, and more than a
 * Group of `moqlivemock` has (25). Not from a measurement.
 */
export const DEFAULT_MAX_HELD_OBJECTS = 32;
/**
 * How many bytes of payload are held at most. **Provisional**: four
 * Objects of about the largest size the sender of this package sends
 * (1,000,000 bytes). An
 * Object that is larger than this by itself is never held, so a viewer
 * sets the limit of its connection for an Object no higher.
 */
export const DEFAULT_MAX_HELD_BYTES = 4 * 1024 * 1024;

export interface VideoReceiverOptions {
	/**
	 * The Timescale of the track, where it has one: the Track Property of
	 * LOC (§6.1: the scope of TIMESCALE is "Track, Object"), or `timescale`
	 * of the track in an MSF catalog (§5.2.21, "The number of time units
	 * that pass per second"). **Which of the two a caller hands in where both
	 * are there and differ, no draft says**; this takes one number. Used for
	 * an Object that has a Timestamp and no Timescale of its own. Without
	 * either, a Timestamp is wall-clock time. From 1 to 2^64 - 1.
	 */
	trackTimescale?: bigint;
	/** `DEFAULT_MAX_HELD_OBJECTS` when left out. From 1 to 4096. */
	maxHeldObjects?: number;
	/** `DEFAULT_MAX_HELD_BYTES` when left out. */
	maxHeldBytes?: number;
}

export interface VideoReceiver {
	/**
	 * Takes an Object, in whatever order the Objects come, and gives what
	 * became of it and of the ones that were held, in order: the frames for
	 * the decoder, and what was dropped.
	 *
	 * An Object that is held is held as it was given: its payload is not
	 * copied, and is not to be changed by the caller afterwards. An Object
	 * that only says that a Group or the track ends is no frame, and is
	 * never among what was dropped.
	 */
	push(object: VideoObject): VideoReceiverEvent[];
	/**
	 * Gives up the Group whose frames are handed on: nothing more of it is
	 * a frame, and the next frame is Object 0 of a later Group. For the
	 * caller that has the decoder, which cannot go on within the Group when
	 * the decoder failed or fell behind and frames were not given to it.
	 *
	 * Gives what was held of the Group as dropped for `because`. What comes
	 * of the Group afterwards is `late`. Does nothing while no Group is
	 * handed on: a key frame is waited for then as it is.
	 */
	abandonGroup(
		because: "decoder-failed" | "decoder-behind",
	): VideoReceiverEvent[];
}

type Held = VideoObject["object"];

/** A whole number from 1 to `most`, or a `RangeError`: NaN and a fraction are no limit. */
function limit(name: string, value: number, most: number): number {
	if (!Number.isInteger(value) || value < 1 || value > most) {
		throw new RangeError(
			`${name} is a whole number from 1 to ${most}, not ${value}`,
		);
	}
	return value;
}

/**
 * The most Objects that can be asked to be held. WHY one at all: what is
 * held is put in order each time an Object comes.
 */
const MOST_HELD_OBJECTS = 4096;
/** 2^64 - 1, the largest integer a property carries. */
const MAX_VALUE = 2n ** 64n - 1n;

/**
 * Orders the Objects of a video track and says which a decoder gets.
 *
 * **The frames of a Group come out in the order of their Object IDs, from
 * Object 0, without a gap**, however the Objects were sent: a Group on one
 * stream, as `moqlivemock` does, or every Object on a stream of its own, as
 * MSF §6 asks, where a later one can come first. An Object that is ahead of
 * its turn is held.
 *
 * - **A receiver that comes in the middle of a Group waits for Object 0 of
 *   a later one.** What it gets of the Group it came into is held until
 *   then and dropped.
 * - **A frame that does not come, or cannot be used, ends its Group**:
 *   nothing after it in the Group is handed on, and the next frame is
 *   Object 0 of a later Group.
 * - WHEN a frame counts as not coming, where the publisher did not say so
 *   and no clock is at hand: **when Object 0 of a later Group is there**,
 *   or when the Objects held are more than the limits. WHY the first, at
 *   once: the later Group can be decoded by itself, and waiting for a frame
 *   of the one before would hold the picture back for a frame that is shown
 *   for a moment. WHY NOT only the limits: a Group whose last frames were
 *   lost would hold the next one back until 32 Objects of it had come.
 *   FIXME: decide this again with a buffer for playback and measurements.
 * - **Where the publisher says that nothing exists, nothing is waited
 *   for**: an End of Group or an End of Track as an Object (transport draft
 *   18 §11.2.1.1) is taken when it arrives, whatever came before it. The
 *   other ways a publisher has to say it are not read: the END_OF_GROUP bit
 *   of a subgroup stream, and the Prior Group ID Gap and Prior Object ID
 *   Gap properties (§12.8, §12.9).
 * - Groups come out in ascending order, and one that was passed does not
 *   come back.
 *
 * Throws a `RangeError` for a limit or a Timescale outside its range.
 */
export function createVideoReceiver(
	options: VideoReceiverOptions = {},
): VideoReceiver {
	const maxHeldObjects = limit(
		"maxHeldObjects",
		options.maxHeldObjects ?? DEFAULT_MAX_HELD_OBJECTS,
		MOST_HELD_OBJECTS,
	);
	const maxHeldBytes = limit(
		"maxHeldBytes",
		options.maxHeldBytes ?? DEFAULT_MAX_HELD_BYTES,
		Number.MAX_SAFE_INTEGER,
	);
	const { trackTimescale } = options;
	// The peer's number: 0 would divide by it.
	if (
		trackTimescale !== undefined &&
		(trackTimescale < 1n || trackTimescale > MAX_VALUE)
	) {
		throw new RangeError(
			`trackTimescale is from 1 to 2^64 - 1, not ${trackTimescale}`,
		);
	}
	/** The Objects that are ahead of their turn, by Group and Object ID. */
	const held = new Map<bigint, Map<bigint, Held>>();
	let heldObjects = 0;
	let heldBytes = 0;
	/** The Group whose frames are handed on, and the Object ID that is next; absent while a key frame is waited for. */
	let playing: { group: bigint; next: bigint } | undefined;
	/** Every Group up to this one is over or given up; -1 before the first. */
	let passed = -1n;
	/**
	 * Where the publisher said a Group ends: no Object of the Group has this
	 * Object ID or a larger one. Of as many Groups as Objects may be held, the
	 * highest: a mark that is forgotten is a hint lost.
	 */
	const groupEnds = new Map<bigint, bigint>();
	/** Where the publisher said the track ends. */
	let trackEnd: { group: bigint; object: bigint } | undefined;
	let events: VideoReceiverEvent[] = [];

	const take = (group: bigint, object: bigint): Held | undefined => {
		const objects = held.get(group);
		const found = objects?.get(object);
		if (objects === undefined || found === undefined) return undefined;
		objects.delete(object);
		if (objects.size === 0) held.delete(group);
		heldObjects -= 1;
		heldBytes -= found.payload.length;
		return found;
	};

	/** Drops what is held of a Group, in the order of the Object IDs. */
	const dropGroup = (group: bigint, because: VideoDropReason): void => {
		const objects = held.get(group);
		if (objects === undefined) return;
		const ids = [...objects.keys()].sort((a, b) => (a < b ? -1 : 1));
		for (const object of ids) {
			take(group, object);
			events.push({ type: "dropped", group, object, because });
		}
	};

	/** Whether the publisher said that nothing exists at this Location. */
	const isAfterEnd = (group: bigint, object: bigint): boolean => {
		const ends = groupEnds.get(group);
		return (
			(ends !== undefined && object >= ends) ||
			(trackEnd !== undefined &&
				(group > trackEnd.group ||
					(group === trackEnd.group && object >= trackEnd.object)))
		);
	};

	/** Drops what is held where the publisher has said that nothing exists. */
	const dropAfterEnd = (): void => {
		for (const group of heldGroups()) {
			const objects = held.get(group);
			if (objects === undefined) continue;
			const ids = [...objects.keys()].sort((a, b) => (a < b ? -1 : 1));
			for (const object of ids) {
				if (!isAfterEnd(group, object)) continue;
				take(group, object);
				events.push({ type: "dropped", group, object, because: "after-end" });
			}
		}
	};

	/** Takes an Object that says where a Group or the track ends. */
	const mark = (group: bigint, object: Held): void => {
		const id = object.objectId;
		if (object.status === "EndOfTrack") {
			const earlier =
				trackEnd === undefined ||
				group < trackEnd.group ||
				(group === trackEnd.group && id < trackEnd.object);
			if (earlier) {
				trackEnd = { group, object: id };
				events.push({ type: "end-of-track", group, object: id });
			}
		} else {
			const ends = groupEnds.get(group);
			if (ends === undefined || id < ends) groupEnds.set(group, id);
			// WHY the lowest Group goes first: the Groups that were passed
			// are the lowest, and their marks say nothing any more.
			while (groupEnds.size > maxHeldObjects) {
				const [oldest] = [...groupEnds.keys()].sort((a, b) => (a < b ? -1 : 1));
				if (oldest === undefined) break;
				groupEnds.delete(oldest);
			}
		}
		dropAfterEnd();
	};

	/** The Groups that have something held, ascending. */
	const heldGroups = (): bigint[] =>
		[...held.keys()].sort((a, b) => (a < b ? -1 : 1));

	/** Gives the Group that is played up, for `because`. */
	const giveUp = (because: VideoDropReason): void => {
		if (playing === undefined) return;
		dropGroup(playing.group, because);
		passed = playing.group;
		playing = undefined;
	};

	/** The frame an Object is, or why it is none. */
	const frameOf = (
		group: bigint,
		object: Held,
	): VideoReceiverEvent & { type: "frame" | "dropped" } => {
		const unusable = (detail: string) =>
			({
				type: "dropped",
				group,
				object: object.objectId,
				because: "unusable",
				detail,
			}) as const;
		// A decoder is given no chunk without data.
		if (object.payload.length === 0) return unusable("no payload");
		const read = readLocTiming(object.properties);
		if (read.type === "invalid") return unusable(read.reason);
		if (read.timing === undefined) {
			// LOC has the Timestamp as optional (§2.2). A frame without a
			// time cannot be given to a decoder that asks for one, and
			// nothing here makes one up.
			return unusable("no Timestamp");
		}
		const timing =
			read.timing.timescale === undefined && trackTimescale !== undefined
				? { ...read.timing, timescale: trackTimescale }
				: read.timing;
		const timestamp = microsecondsOf(timing);
		if (timestamp > MAX_DECODER_TIME) {
			return unusable("a time beyond 2^53 - 1 microseconds");
		}
		const annexB = toAnnexB(object.payload);
		if (annexB.type === "invalid") return unusable(annexB.reason);
		return {
			type: "frame",
			group,
			object: object.objectId,
			key: object.objectId === 0n,
			timestamp,
			data: annexB.payload,
		};
	};

	/** Hands on everything that is in turn. */
	const drain = (): void => {
		for (;;) {
			if (playing !== undefined) {
				const { group, next } = playing;
				// The publisher said that the Group has no Object from here on.
				if (isAfterEnd(group, next)) {
					giveUp("after-end");
					continue;
				}
				const object = take(group, next);
				if (object !== undefined) {
					const event = frameOf(group, object);
					events.push(event);
					if (event.type === "frame") {
						playing.next = next + 1n;
					} else {
						giveUp("after-unusable");
					}
					continue;
				}
			}
			// Nothing in turn: a later Group whose Object 0 is there begins.
			// WHY any Group that is held: nothing is held of a Group that is
			// passed, and Object 0 of the one that is played was handed on.
			const begins = heldGroups().find((group) => held.get(group)?.has(0n));
			if (begins === undefined) return;
			giveUp("gap");
			for (const group of heldGroups()) {
				if (group >= begins) break;
				dropGroup(group, "no-key-frame");
			}
			passed = begins - 1n;
			playing = { group: begins, next: 0n };
		}
	};

	/** Gives up Groups, the oldest first, until what is held is within the limits. */
	const keepWithinLimits = (): void => {
		while (heldObjects > maxHeldObjects || heldBytes > maxHeldBytes) {
			if (playing !== undefined && held.has(playing.group)) {
				giveUp("gap");
				continue;
			}
			const [oldest] = heldGroups();
			if (oldest === undefined) return;
			dropGroup(oldest, "overflow");
			// WHY not passed: its Object 0 can still come, and begin it. What
			// was dropped of it is lost to that Group then, which ends at the
			// first of them.
		}
	};

	return {
		push({ groupId: group, object }) {
			events = [];
			const id = object.objectId;
			// Every Group before the one that is played is passed.
			const isLate =
				group <= passed ||
				(playing !== undefined && group === playing.group && id < playing.next);
			if (object.status !== "Normal") {
				mark(group, object);
				drain();
				return events;
			}
			if (isLate || held.get(group)?.has(id)) {
				return [{ type: "dropped", group, object: id, because: "late" }];
			}
			if (isAfterEnd(group, id)) {
				return [{ type: "dropped", group, object: id, because: "after-end" }];
			}
			const objects = held.get(group) ?? new Map<bigint, Held>();
			held.set(group, objects);
			objects.set(id, object);
			heldObjects += 1;
			heldBytes += object.payload.length;
			drain();
			keepWithinLimits();
			return events;
		},

		abandonGroup(because) {
			events = [];
			// Nothing is in turn after it: Object 0 of a later Group would
			// have begun that Group when it came.
			giveUp(because);
			return events;
		},
	};
}
