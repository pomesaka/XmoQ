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
import {
	type PeerFetch,
	type PeerSubscribe,
	type PublishedTrack,
	RequestErrorCode,
	type SubgroupWriter,
} from "@xmoq/moqt";

/** PUBLISH_DONE Status Code TRACK_ENDED (transport draft 18 §10.11). */
const TRACK_ENDED = 0x2n;
/** How many chunks may wait to be written before one is left out. **Provisional, not measured**: half a second of chunks of 20 ms. */
export const DEFAULT_MAX_PENDING_AUDIO_CHUNKS = 25;

export interface AudioChunk {
	/** In microseconds, as the encoder counts. */
	readonly timestamp: number;
	readonly data: Uint8Array;
}

export interface AudioTrackPublisherOptions {
	trackNamespace: readonly Uint8Array[];
	trackName: Uint8Array;
	/** The Group of the first chunk; each chunk after it is the next Group. */
	firstGroup: bigint;
	/** The wall-clock time of the moment the encoder counts from, in microseconds. */
	epoch: bigint;
	/** `DEFAULT_MAX_PENDING_AUDIO_CHUNKS` when left out. */
	maxPendingChunks?: number;
	/**
	 * How many chunks share a Group, and so a stream: 1 when left out, the
	 * example of LOC-04 §4.1. **For trying out.** With more, the
	 * subscriber of this publisher (a relay) begins at a Group's first
	 * chunk; a relay's own subscribers begin where they come, where the
	 * relay starts a stream in the middle of a Subgroup, as the one here
	 * does.
	 */
	chunksPerGroup?: number;
}

export interface AudioTrackPublisherStats {
	/** Objects handed to the transport. */
	written: number;
	/** Chunks that were not sent: "no-subscriber", "behind" (too many wait), "write-failed". */
	skipped: Partial<Record<"no-subscriber" | "behind" | "write-failed", number>>;
}

export interface AudioTrackPublisher {
	/** For the `subscribe` handler of `MoqtConnection.serve`. False for another track. One subscription at a time, as for video. */
	subscribe(request: PeerSubscribe): boolean;
	/** For the `fetch` handler. False for another track; a FETCH for this one is refused with NOT_SUPPORTED. */
	fetch(request: PeerFetch): boolean;
	/** Takes the next chunk of the encoder. Does not wait for the write, and does not throw. */
	push(chunk: AudioChunk): void;
	/** Ends the track: what is queued is written, and the subscription gets PUBLISH_DONE. Never rejects. */
	end(): Promise<void>;
	stats(): AudioTrackPublisherStats;
}

const sameBytes = (a: Uint8Array, b: Uint8Array): boolean =>
	a.length === b.length && a.every((byte, index) => byte === b[index]);

export function createAudioTrackPublisher(
	options: AudioTrackPublisherOptions,
): AudioTrackPublisher {
	const maxPending =
		options.maxPendingChunks ?? DEFAULT_MAX_PENDING_AUDIO_CHUNKS;
	const stats: AudioTrackPublisherStats = { written: 0, skipped: {} };
	const skip = (because: keyof AudioTrackPublisherStats["skipped"]): void => {
		stats.skipped[because] = (stats.skipped[because] ?? 0) + 1;
	};
	let track: PublishedTrack | undefined;
	let answering = false;
	let ended = false;
	let nextGroup = options.firstGroup;
	let pending = 0;
	let queue: Promise<void> = Promise.resolve();
	const chunksPerGroup = Math.max(1, Math.floor(options.chunksPerGroup ?? 1));
	/** The chunk's place in its Group, and the stream of the Group that is open. */
	let nextObject = 0;
	let open: { to: PublishedTrack; stream: SubgroupWriter } | undefined;

	const isThisTrack = (request: PeerSubscribe | PeerFetch): boolean =>
		sameBytes(request.trackName, options.trackName) &&
		request.trackNamespace.length === options.trackNamespace.length &&
		request.trackNamespace.every((field, index) => {
			const own = options.trackNamespace[index];
			return own !== undefined && sameBytes(field, own);
		});

	return {
		subscribe(request) {
			if (!isThisTrack(request)) return false;
			if (ended || track !== undefined || answering) {
				request
					.reject(
						ended
							? RequestErrorCode.DOES_NOT_EXIST
							: RequestErrorCode.DUPLICATE_SUBSCRIPTION,
					)
					.catch(() => {});
				return true;
			}
			answering = true;
			request.accept().then(
				(accepted) => {
					answering = false;
					track = accepted;
					void accepted.ended.then(() => {
						if (track === accepted) track = undefined;
					});
				},
				() => {
					answering = false;
				},
			);
			return true;
		},

		fetch(request) {
			if (!isThisTrack(request)) return false;
			request.reject(RequestErrorCode.NOT_SUPPORTED).catch(() => {});
			return true;
		},

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
			const to = track;
			if (ended || to === undefined) return skip("no-subscriber");
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
						open = {
							to,
							stream: await to.openSubgroup({
								groupId,
								subgroupId: { mode: "Zero" },
								properties: true,
								endOfGroup: false,
							}),
						};
					}
					if (open === undefined || open.to !== to) return;
					const { stream } = open;
					try {
						await stream.write({
							objectId: BigInt(objectId),
							payload: chunk.data,
							properties: locProperties({ timestamp }),
						});
						if (last) {
							open = undefined;
							await stream.close();
						}
						stats.written += 1;
					} catch (error) {
						open = undefined;
						await stream.reset().catch(() => {});
						throw error;
					}
				})
				.catch(() => {
					if (track === to) skip("write-failed");
				})
				.finally(() => {
					pending -= 1;
				});
		},

		async end() {
			if (ended) return;
			ended = true;
			const to = track;
			if (to === undefined) return;
			await queue;
			await open?.stream.close().catch(() => {});
			track = undefined;
			await to.done(TRACK_ENDED, "").catch(() => {});
		},

		stats: () => ({ ...stats, skipped: { ...stats.skipped } }),
	};
}
