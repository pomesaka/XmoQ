// A LOC audio track published over a connection. Minimal: every chunk of
// an encoder is a Group of its own with one Object, on a stream of its own,
// as the example of LOC-04 §4.1 has it ("there is one MOQT Object per MOQT
// Group ... each such unitary group is sent over an individual
// unidirectional QUIC stream"). No WebCodecs: a chunk comes in as numbers
// and bytes.
//
// WHY apart from the video publisher: a chunk of audio needs no other, so
// nothing is cut, nothing waits for a key frame, and a chunk that cannot be
// written in time is left out and missed by no later one.

import { locProperties } from "@xmoq/loc";
import type {
	PeerFetch,
	PeerSubscribe,
	PublishedTrack,
	SubgroupWriter,
} from "@xmoq/moqt";
import {
	DEFAULT_END_TIMEOUT_MS,
	publishTrack,
	tellEnd,
	wholeNumber,
} from "@xmoq/tracks";

/** How many chunks may wait to be written before one is left out. **Provisional, not measured**: half a second of chunks of 20 ms. */
export const DEFAULT_MAX_PENDING_AUDIO_CHUNKS = 25;

export interface AudioChunk {
	/** In microseconds, as the encoder counts. */
	readonly timestamp: number;
	readonly data: Uint8Array;
}

/**
 * Every chunk of audio is a Group of its own, on a stream of its own.
 * Decided by the owner on 2026-10-13.
 *
 * WHY 1: it is what the drafts say (the example of LOC-04 §4.1, and what
 * MSF-01 §6 asks); nothing measured speaks for more; and on a link
 * narrower than what is sent, and at 5% loss with a buffer of 50 ms, 1
 * lost fewer pieces in every run. Measured on 2026-10-12 with 1, 5 and 10,
 * three to five runs each (docs/experiments/2026-10-12-one-five-or-ten-
 * pieces-of-sound-a-group.md); without loss and at 1% loss the three could
 * not be told apart. A loss holds back the rest of its stream, and with 1
 * there is no rest.
 *
 * WHY NOT 10, which it was from 2026-10-11 until then: what that was
 * decided on did not hold (the sound was not late with a stream a chunk;
 * the times Chromium's decoder gave its output were early,
 * docs/experiments/2026-10-11-sound-falls-behind-on-a-link-with-delay-
 * and-loss.md). What 1 costs, and is paid: ten times the streams (50 a
 * second for a viewer, against 5), and about a half more for the relay to
 * do. `chunksPerGroup` asks for another number.
 */
export const DEFAULT_AUDIO_CHUNKS_PER_GROUP = 1;

export interface AudioTrackPublisherOptions {
	trackNamespace: readonly Uint8Array[];
	trackName: Uint8Array;
	/** The Group of the first chunk; each chunk after it is the next Group. */
	firstGroup: bigint;
	/** The wall-clock time of the moment the encoder counts from, in microseconds. */
	epoch: bigint;
	/** `DEFAULT_MAX_PENDING_AUDIO_CHUNKS` when left out; a whole number from 1. */
	maxPendingChunks?: number;
	/**
	 * How many chunks share a Group, and so a stream:
	 * `DEFAULT_AUDIO_CHUNKS_PER_GROUP` when left out; a whole number from 1.
	 * The subscriber of
	 * this publisher (a relay) begins at a Group's first chunk; a relay's
	 * own subscribers begin where they come, where the relay starts a
	 * stream in the middle of a Subgroup, as the one here does.
	 */
	chunksPerGroup?: number;
	/**
	 * The Publisher Priority of every stream of the track (transport §7.1;
	 * 0 is the highest and 255 the lowest). Left out, the streams name
	 * none and have the default of the track, 128 (§12.4).
	 */
	publisherPriority?: number;
	/** How long `end` waits for what is queued to be written; 3000 ms when left out, as for video. */
	endTimeoutMs?: number;
}

export interface AudioTrackPublisherStats {
	/** Objects handed to the transport. */
	written: number;
	/** Chunks that were not sent: "no-subscriber", "behind" (too many wait), "write-failed". */
	skipped: Partial<Record<"no-subscriber" | "behind" | "write-failed", number>>;
}

export interface AudioTrackPublisher {
	/**
	 * For the `subscribe` handler of `MoqtConnection.serve`. False for
	 * another track. One subscription at a time, as for video. The
	 * SUBSCRIBE_OK has LARGEST_OBJECT, the place of the last chunk, once a
	 * chunk was pushed; with FORWARD 0 nothing is sent to it.
	 */
	subscribe(request: PeerSubscribe): boolean;
	/** For the `fetch` handler. False for another track; a FETCH for this one is refused with NOT_SUPPORTED, and with DOES_NOT_EXIST after `end`. */
	fetch(request: PeerFetch): boolean;
	/** Takes the next chunk of the encoder. Does not wait for the write, and does not throw. */
	push(chunk: AudioChunk): void;
	/**
	 * Ends the track: what is queued is written, and the subscription gets
	 * PUBLISH_DONE (TRACK_ENDED). A SUBSCRIBE after it is refused with
	 * DOES_NOT_EXIST. Never rejects.
	 *
	 * **How long it takes at most**, as for video: `endTimeoutMs` for what
	 * is queued; what is not written by then is not written, its stream is
	 * reset, and what is under way then has `endTimeoutMs` once more. A
	 * SUBSCRIBE that is being answered when `end` is called gets its
	 * PUBLISH_DONE once the answer is out, `endTimeoutMs` at most.
	 */
	end(): Promise<void>;
	stats(): AudioTrackPublisherStats;
}

/**
 * Throws a `RangeError` for a count that is no whole number from 1
 * (`maxPendingChunks`, `chunksPerGroup`) and for an `endTimeoutMs` that is
 * no whole number from 0, as the video publisher does.
 */
export function createAudioTrackPublisher(
	options: AudioTrackPublisherOptions,
): AudioTrackPublisher {
	const maxPending = wholeNumber(
		"maxPendingChunks",
		options.maxPendingChunks ?? DEFAULT_MAX_PENDING_AUDIO_CHUNKS,
		1,
	);
	const endTimeoutMs = wholeNumber(
		"endTimeoutMs",
		options.endTimeoutMs ?? DEFAULT_END_TIMEOUT_MS,
		0,
	);
	const chunksPerGroup = wholeNumber(
		"chunksPerGroup",
		options.chunksPerGroup ?? DEFAULT_AUDIO_CHUNKS_PER_GROUP,
		1,
	);
	const stats: AudioTrackPublisherStats = { written: 0, skipped: {} };
	const skip = (because: keyof AudioTrackPublisherStats["skipped"]): void => {
		stats.skipped[because] = (stats.skipped[because] ?? 0) + 1;
	};
	let track: PublishedTrack | undefined;
	/**
	 * FORWARD 0 (transport §5.1: "The publisher does not send Objects if the
	 * Forward State is 0") for `track`. As the SUBSCRIBE had it, as for
	 * video: `PublishedTrack` has its parameters and no later ones.
	 */
	let silent = false;
	let ended: Promise<void> | undefined;
	/** The place of the last chunk that was pushed, for LARGEST_OBJECT. */
	let largest: { group: bigint; object: bigint } | undefined;
	let nextGroup = options.firstGroup;
	let pending = 0;
	let queue: Promise<void> = Promise.resolve();
	/** The chunk's place in its Group, and the stream of the Group that is open. */
	let nextObject = 0;
	let open: { to: PublishedTrack; stream: SubgroupWriter } | undefined;

	const published = publishTrack({
		trackNamespace: options.trackNamespace,
		trackName: options.trackName,
		subscribers: "one",
		// §10.2.11: "If Objects have been published on this Track the
		// Publisher MUST include this parameter".
		answer: () =>
			largest === undefined ? {} : { parameters: { largestObject: largest } },
		subscribed(accepted, request) {
			track = accepted;
			silent = request.parameters.forward === false;
			void accepted.ended.then(() => {
				if (track === accepted) track = undefined;
			});
		},
	});

	return {
		subscribe: (request) => published.subscribe(request),
		fetch: (request) => published.fetch(request),

		push(chunk) {
			// A place for every chunk, also for one that is not sent: the IDs
			// say how many chunks there were.
			const groupId = nextGroup;
			const objectId = nextObject;
			nextObject += 1;
			const last = nextObject === chunksPerGroup;
			if (last) {
				nextGroup += 1n;
				nextObject = 0;
			}
			if (ended !== undefined) return skip("no-subscriber");
			largest = { group: groupId, object: BigInt(objectId) };
			const to = track;
			if (to === undefined || silent) return skip("no-subscriber");
			if (pending >= maxPending) return skip("behind");
			pending += 1;
			const timestamp = options.epoch + BigInt(Math.round(chunk.timestamp));
			queue = queue
				.then(async () => {
					if (track !== to) return;
					// A subscriber begins with a Group's first chunk, and a
					// Group whose stream failed is not gone on with.
					if (objectId === 0) {
						if (open !== undefined) await open.stream.reset().catch(() => {});
						open = undefined;
						const opened = await to.openSubgroup({
							groupId,
							subgroupId: { mode: "Zero" },
							...(options.publisherPriority === undefined
								? {}
								: { publisherPriority: options.publisherPriority }),
							properties: true,
							endOfGroup: false,
						});
						// WHY looked at again: `end` gave the track up while the
						// stream was opened, and a stream left open keeps
						// PUBLISH_DONE from being sent.
						if (track !== to) {
							await opened.reset().catch(() => {});
							return;
						}
						open = { to, stream: opened };
					}
					if (open === undefined || open.to !== to) return;
					const { stream } = open;
					let handed = false;
					try {
						await stream.write({
							objectId: BigInt(objectId),
							payload: chunk.data,
							properties: locProperties({ timestamp }),
						});
						// Counted here, as the video publisher does: the Object
						// is with the transport, whatever its stream's close does.
						handed = true;
						stats.written += 1;
						if (last) {
							open = undefined;
							await stream.close();
						}
					} catch (error) {
						open = undefined;
						await stream.reset().catch(() => {});
						// WHY a close that fails is no chunk left out: the chunk
						// was handed to the transport before it.
						if (!handed) throw error;
					}
				})
				.catch(() => {
					if (track === to) skip("write-failed");
				})
				.finally(() => {
					pending -= 1;
				});
		},

		end() {
			ended ??= (async () => {
				const answered = published.end();
				let timer: ReturnType<typeof setTimeout> | undefined;
				// One timer at a time: the one before has fired, or its wait
				// is over.
				const timeIsOver = (): Promise<"late"> =>
					new Promise((resolve) => {
						clearTimeout(timer);
						timer = setTimeout(() => resolve("late"), endTimeoutMs);
					});
				try {
					const to = track;
					if (to === undefined) {
						// A SUBSCRIBE that is being answered is told of the end
						// when its answer is out (`publishTrack`).
						await Promise.race([answered, timeIsOver()]);
						return;
					}
					if ((await Promise.race([queue, timeIsOver()])) === "late") {
						// WHY before the reset: the write that is stuck fails with
						// it, and the chunk behind it would open a stream of its
						// own; with `track` gone it does not (`track !== to`).
						track = undefined;
						// A stream that is neither closed nor reset keeps
						// PUBLISH_DONE from being sent (transport §10.11).
						await open?.stream.reset().catch(() => {});
						open = undefined;
						await Promise.race([queue, timeIsOver()]);
					} else {
						await open?.stream.close().catch(() => {});
					}
					track = undefined;
					await tellEnd(to);
				} finally {
					clearTimeout(timer);
				}
			})();
			return ended;
		},

		stats: () => ({ ...stats, skipped: { ...stats.skipped } }),
	};
}
