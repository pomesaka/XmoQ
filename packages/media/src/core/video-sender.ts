// The sending side of a LOC video track, from where an encoder ends: each
// chunk of the encoder goes in, and what comes out says which Object it is,
// with which properties, and on which stream. No I/O, no clock and no
// WebCodecs.
//
// "LOC" is draft-ietf-moq-loc-04 and "MSF" draft-ietf-moq-msf-01.

import { type LocProperty, locProperties } from "@xmoq/loc";

/**
 * A chunk as an encoder gives it: the fields of `EncodedVideoChunk` of
 * WebCodecs that are used, with its data copied out.
 */
export interface VideoChunk {
	readonly type: "key" | "delta";
	/**
	 * In microseconds, as the encoder counts: the time of the frame in the
	 * media, which begins where the source does and not at the Unix epoch.
	 */
	readonly timestamp: number;
	/** In Annex B form, a start code of 4 bytes first. */
	readonly data: Uint8Array;
}

/** How the Objects of the track go onto streams. */
export type VideoStreamMapping =
	/** Every Object on a stream of its own, as a Subgroup of its own, whose ID is the Object ID. */
	| "stream-per-object"
	/** The Objects of a Group on one stream, Subgroup 0. */
	| "stream-per-group";

/** Why a chunk is not sent. */
export type VideoSkipReason =
	/** A delta chunk before the first key chunk, or after a chunk of its Group that was not sent. */
	| "no-key-frame"
	/** Its data is longer than `maxPayloadLength`. */
	| "too-large"
	/** Its data does not begin with a start code of 4 bytes, or has nothing after it. */
	| "not-annex-b"
	/** Its time is no whole number of microseconds, or with `epoch` lies before the Unix epoch or beyond 2^53 - 1 microseconds after it. */
	| "no-time"
	/** A key chunk when the last Group ID there is was used. */
	| "no-group-id";

/**
 * One thing for the caller to do on the track, in the order the steps are
 * given. The streams are named by their Group and Subgroup: of each, one is
 * open at most.
 */
export type VideoSenderStep =
	/** Write this Object: open the stream of its Group and Subgroup for it where `opensStream` says so, and finish that stream after it where `closesStream` does. */
	| {
			type: "object";
			groupId: bigint;
			/** The Object ID with "stream-per-object", 0 with "stream-per-group". */
			subgroupId: bigint;
			objectId: bigint;
			/** The Timestamp of LOC: wall-clock time in microseconds. */
			properties: LocProperty[];
			payload: Uint8Array;
			/** Whether the Object begins a stream: the stream of this Group and Subgroup is opened for it. */
			opensStream: boolean;
			/** Whether the stream is finished after the Object. */
			closesStream: boolean;
	  }
	/**
	 * With "stream-per-group": finish the stream of this Group, with a FIN.
	 * Every Object the Group has was written to it: the next Group begins,
	 * the track ends, or a chunk was not sent and so is no Object.
	 */
	| { type: "close"; groupId: bigint; subgroupId: bigint }
	/**
	 * With "stream-per-group": reset the stream of this Group. An Object of
	 * the Group could not be written (transport draft 18 §11.4.3: "If a
	 * sender closes the stream before delivering all such objects to the
	 * QUIC stream, it MUST reset the stream").
	 */
	| { type: "reset"; groupId: bigint; subgroupId: bigint }
	/** The chunk is not sent, and nothing is to be done for it. */
	| { type: "skipped"; because: VideoSkipReason };

/**
 * The longest payload that is sent, when no other is given. **Provisional**:
 * below what the relay of this repository passes on, and how large a key
 * frame of a camera picture is was not measured.
 *
 * WHY 1,000,000 and not 1 MiB: the relay holds 1 MiB of a subgroup stream
 * that it has not decoded, and counts in that the fields before the payload
 * (under 40 bytes for an Object with the Timestamp) and what the read that
 * completes the Object brought along of the next one (16 KiB at most). So
 * about 1,032,000 bytes, 1 MiB less 16 KiB and those fields, is the most
 * that passes; a payload of 1 MiB does not. With a Group on one stream the count
 * is the same for every Object: the relay hands an Object on when it is
 * whole.
 */
export const DEFAULT_MAX_PAYLOAD_LENGTH = 1_000_000;

export interface VideoSenderOptions {
	/**
	 * The Group of the first key chunk. MSF §6.1: a publisher that starts
	 * again "MUST ensure the new starting Group ID is greater than any
	 * previously published Group ID for that track. One approach is to use
	 * the current wall clock time in milliseconds since the Unix epoch". The
	 * caller gives it; nothing here knows the time or an earlier run.
	 */
	firstGroup: bigint;
	/**
	 * The wall-clock time, in microseconds since the Unix epoch, of the
	 * moment the encoder's `timestamp` counts from.
	 *
	 * WHY asked for here, and not left to the caller to add: the Timestamp
	 * of LOC without a Timescale is wall-clock time (LOC §2.3.1.1), and an
	 * encoder's is not. With one field for "the time", the encoder's would
	 * be handed on as it is, and read as a moment in 1970.
	 *
	 * From 0 to 2^53 - 1, the largest time the receiver of this package
	 * hands a decoder.
	 */
	epoch: bigint;
	/** "stream-per-object" when left out. */
	mapping?: VideoStreamMapping;
	/** `DEFAULT_MAX_PAYLOAD_LENGTH` when left out. */
	maxPayloadLength?: number;
}

export interface VideoSender {
	/** Takes the next chunk of the encoder and says what is to be written for it, in order. */
	push(chunk: VideoChunk): VideoSenderStep[];
	/**
	 * Ends the Group that is being sent: nothing more of it is sent, and the
	 * next Object is the next key chunk. Gives what is to be done with the
	 * stream that is open, if one is:
	 *
	 * - "track-ended": the track ends, and every Object of the Group was
	 *   written. The stream is finished.
	 * - "write-failed": an Object of the Group could not be written. The
	 *   stream is reset. With a stream for every Object none is open here,
	 *   and the stream of the Object that failed is the caller's to reset.
	 */
	endGroup(because: "track-ended" | "write-failed"): VideoSenderStep[];
}

/** 2^64 - 1, the largest ID MoQ Transport carries. */
const MAX_ID = 2n ** 64n - 1n;
/** The largest time that is sent: what the receiver of this package hands on, the largest whole number a JavaScript number has exactly. */
const MAX_TIMESTAMP = BigInt(Number.MAX_SAFE_INTEGER);

/** LOC §2.1.4: "The start code value is 1 in network byte order". A start code alone is no NAL unit. */
const beginsWithStartCode = (data: Uint8Array): boolean =>
	data.length > 4 &&
	data[0] === 0 &&
	data[1] === 0 &&
	data[2] === 0 &&
	data[3] === 1;

/**
 * Says for each chunk of an encoder which Object of the track it is.
 *
 * - **A key chunk begins a Group, as its Object 0**; a delta chunk is the
 *   next Object of that Group. Groups are numbered one after the other from
 *   `firstGroup` (MSF §6.1: "each subsequent Group ID SHOULD increase by
 *   1"). MSF §4.1 has the frames of a group of pictures in one Group; that
 *   a Group is exactly one, with the key frame first, is the promise the
 *   receiver of this package counts on.
 * - **Every Object has the Timestamp of LOC, and nothing else**: `epoch`
 *   plus the encoder's time, wall-clock microseconds, with no Timescale.
 * - The payload is the chunk's data, unchanged.
 * - **After a chunk that is not sent, nothing of its Group is**: every
 *   frame after it needs it. The next Object is the next key chunk, which
 *   begins the next Group; the Group that was cut short ends where it was
 *   cut, and no Group ID is left out.
 * - WHY a chunk without a start code of 4 bytes is not sent: LOC allows a
 *   start code of 3 bytes (§2.1.4) and a length before every NAL unit
 *   (§2.1.3) as well. This sender writes one form, and a receiver that
 *   tells lengths from start codes by the first four bytes (`toAnnexB` of
 *   @xmoq/loc) reads a start code of 3 bytes as a length.
 * - **The Subgroup ID**: with a stream for every Object, each Object is a
 *   Subgroup of its own, as transport draft 18 §2.2 has "Objects from two
 *   subgroups MUST NOT be sent on the same stream, and Objects from the
 *   same Subgroup MUST NOT be sent on different streams"; its ID is the
 *   Object ID, which a publisher is free to choose (§2.2, "as they see
 *   fit") and which the header can say without a field (§11.4.2, the mode
 *   in which the Subgroup ID is the Object ID of the first Object).
 *
 * Throws a `RangeError` for options that are no ID, no time, no length or
 * no mapping. `push` and `endGroup` do not throw.
 */
export function createVideoSender(options: VideoSenderOptions): VideoSender {
	const { firstGroup, epoch } = options;
	const mapping = options.mapping ?? "stream-per-object";
	const maxPayloadLength =
		options.maxPayloadLength ?? DEFAULT_MAX_PAYLOAD_LENGTH;
	if (firstGroup < 0n || firstGroup > MAX_ID) {
		throw new RangeError(`${firstGroup} is no Group ID`);
	}
	if (epoch < 0n || epoch > MAX_TIMESTAMP) {
		throw new RangeError(
			`${epoch} is no time since the Unix epoch that can be sent`,
		);
	}
	if (mapping !== "stream-per-object" && mapping !== "stream-per-group") {
		throw new RangeError(`${String(mapping)} is no mapping`);
	}
	if (!Number.isInteger(maxPayloadLength) || maxPayloadLength < 1) {
		throw new RangeError(
			`maxPayloadLength is a whole number from 1, not ${maxPayloadLength}`,
		);
	}
	/** The Group of the next key chunk. */
	let nextGroup = firstGroup;
	/** The Group that is being sent and the ID of its next Object; absent before the first key chunk and after a Group was cut short. */
	let sending: { group: bigint; next: bigint } | undefined;
	/** With "stream-per-group": the Group whose stream is open. */
	let open: bigint | undefined;

	/** What is to be done with the stream that is open, if one is. */
	const end = (type: "close" | "reset"): VideoSenderStep[] => {
		if (open === undefined) return [];
		const step = { type, groupId: open, subgroupId: 0n };
		open = undefined;
		return [step];
	};
	const close = (): VideoSenderStep[] => end("close");

	const skip = (because: VideoSkipReason): VideoSenderStep[] => {
		sending = undefined;
		return [...close(), { type: "skipped", because }];
	};

	return {
		push(chunk) {
			if (chunk.type === "delta" && sending === undefined) {
				return [{ type: "skipped", because: "no-key-frame" }];
			}
			if (chunk.data.length > maxPayloadLength) return skip("too-large");
			if (!beginsWithStartCode(chunk.data)) return skip("not-annex-b");
			if (!Number.isSafeInteger(chunk.timestamp)) return skip("no-time");
			const timestamp = epoch + BigInt(chunk.timestamp);
			if (timestamp < 0n || timestamp > MAX_TIMESTAMP) return skip("no-time");
			// Before anything is changed for the chunk: the Group before it
			// ends as with any chunk that is not sent.
			if (chunk.type === "key" && nextGroup > MAX_ID) {
				return skip("no-group-id");
			}

			const closed = chunk.type === "key" ? close() : [];
			if (chunk.type === "key") {
				sending = { group: nextGroup, next: 0n };
				nextGroup += 1n;
			}
			// A delta chunk without a Group was skipped above.
			if (sending === undefined) return closed;
			const { group, next } = sending;
			sending.next = next + 1n;
			const perObject = mapping === "stream-per-object";
			const opensStream = perObject || open !== group;
			if (!perObject) open = group;
			return [
				...closed,
				{
					type: "object",
					groupId: group,
					subgroupId: perObject ? next : 0n,
					objectId: next,
					properties: locProperties({ timestamp }),
					payload: chunk.data,
					opensStream,
					closesStream: perObject,
				},
			];
		},

		endGroup(because) {
			sending = undefined;
			return end(because === "write-failed" ? "reset" : "close");
		},
	};
}
