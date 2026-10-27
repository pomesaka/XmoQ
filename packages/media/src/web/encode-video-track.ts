// A LOC video track made in a browser: frames go into an encoder of
// WebCodecs, and its chunks into the publisher of this package
// (`src/moqt/video-track-publisher.ts`). What is decided here is only what
// the encoder makes necessary; which chunk becomes which Object is the
// sender's, and what is written when is the publisher's.

// How long `end` waits for the encoder to give what it holds is what the
// publisher waits for its writes (`DEFAULT_END_TIMEOUT_MS`).
import { DEFAULT_END_TIMEOUT_MS } from "@xmoq/tracks";
import { createStamp, type StampClock } from "../core/stamp.ts";
import type {
	VideoSkipReason,
	VideoStreamMapping,
} from "../core/video-sender.ts";
import {
	createVideoTrackPublisher,
	type VideoPushSkipReason,
	type VideoTrackPublisher,
} from "../moqt/video-track-publisher.ts";

/**
 * How many frames may wait in the encoder before a frame is not encoded.
 * **Provisional, not measured.** WHY a limit at all: an encoder that is
 * slower than its source would hold every frame that comes, each with its
 * picture.
 */
export const DEFAULT_MAX_ENCODE_QUEUE = 2;
/**
 * After how many key chunks in a row that could not be sent the caller is
 * told. **Provisional.** WHY not the first: one key frame can be larger
 * than the rest, and the next one is asked for at once.
 */
export const DEFAULT_MAX_UNSENT_KEY_CHUNKS = 3;

/** Why the sender sends a chunk at no time: what the chunk is, and not who takes it. */
const OF_THE_CHUNK: readonly VideoSkipReason[] = [
	"too-large",
	"not-annex-b",
	"no-time",
	"no-group-id",
];

export interface VideoTrackEncoderOptions {
	trackNamespace: readonly Uint8Array[];
	trackName: Uint8Array;
	/** The Group of the first key frame (`VideoSenderOptions.firstGroup`). */
	firstGroup: bigint;
	/**
	 * The codec string the encoder is configured with, H.264: what was run
	 * is `avc1.42E01F`. **The catalog of the track names the same profile
	 * and level as `avc3.…`**: the parameter sets are in the key frames
	 * (Annex B), which is what `avc3` says and `avc1` does not.
	 */
	codec: string;
	width: number;
	height: number;
	/** Bits a second; the encoder's own choice when left out. */
	bitrate?: number;
	/** Frames a second, for the encoder's rate control; it does not make the source keep to it. */
	framerate?: number;
	/**
	 * A frame is made a key frame when this many milliseconds of the
	 * frames' own time have passed since the last one. **No value is taken
	 * for it when left out**: it is how long a Group is, and so how long a
	 * viewer that comes waits and how much a lost frame costs.
	 */
	keyFrameIntervalMs: number;
	/**
	 * How the frames' times are laid onto the wall clock (`StampClock`);
	 * `"first"` when left out. `"least"` is for a source whose clock is not
	 * the one the other track of the broadcast counts by.
	 */
	clock?: StampClock;
	/** How the Objects go onto streams; the sender's default when left out. */
	mapping?: VideoStreamMapping;
	/** The longest payload that is sent (`VideoSenderOptions.maxPayloadLength`). */
	maxPayloadLength?: number;
	/** `DEFAULT_MAX_ENCODE_QUEUE` when left out. */
	maxEncodeQueue?: number;
	/**
	 * A subscriber may ask for a new Group (`VideoTrackPublisherOptions.dynamicGroups`):
	 * the next frame is then a key frame. None when left out.
	 */
	dynamicGroups?: { shortestGroupMs?: number };
	/**
	 * Called when `maxUnsentKeyChunks` key chunks in a row could not be
	 * sent for what they are: too long, mostly (`too-large`). **Nothing of
	 * the track reaches anybody then**: every Group begins with its key
	 * chunk. Called once each time it comes to that, and again only after a
	 * key chunk was sent in between.
	 *
	 * The encoder goes on, and so does the sending: a key frame of another
	 * picture can be smaller, and is sent. What to do about it (a lower
	 * resolution, another `maxPayloadLength`, ending the track) is the
	 * caller's.
	 */
	onKeyChunksNotSent?(event: {
		/** Why the last of them was not sent. */
		because: VideoSkipReason;
		/** How many in a row. */
		inARow: number;
		/** The length of the last of them, in bytes. */
		length: number;
	}): void;
	/** `DEFAULT_MAX_UNSENT_KEY_CHUNKS` when left out. */
	maxUnsentKeyChunks?: number;
	/**
	 * How long `end` waits for the encoder to give what it holds, and then
	 * the publisher for its writes (`VideoTrackPublisherOptions.endTimeoutMs`);
	 * 3000 ms when left out.
	 */
	endTimeoutMs?: number;
}

/** Why a frame did not become an Object, besides the reasons of the publisher. */
export type VideoEncodeSkipReason =
	/** More frames wait in the encoder than may: the frame was not encoded. */
	| "encoder-behind"
	/** Its time is not after the time of the frame before it. */
	| "time-not-after"
	/** The encoder has failed, or `end` was called. */
	| "closed";

export interface VideoTrackEncoderStats {
	/** Frames handed to the encoder. */
	submitted: number;
	/** Chunks the encoder gave. */
	encoded: number;
	/** Of them, key chunks. */
	keyChunks: number;
	/** Frames that were not handed to the encoder, by the reason. */
	notEncoded: Partial<Record<VideoEncodeSkipReason, number>>;
	/** Chunks the publisher did not send, by its reason. */
	notSent: Partial<Record<VideoPushSkipReason, number>>;
	/** Times the publisher asked for a key frame. */
	keyFramesAskedFor: number;
	/** Key chunks that could not be sent for what they are, since the last one that was sent. */
	unsentKeyChunksInARow: number;
	/** The bytes of all chunks. */
	encodedBytes: number;
	/** The longest chunk, in bytes. */
	largestChunk: number;
	/** What the encoder said when it failed. */
	encoderError: string | undefined;
}

export interface VideoTrackEncoder {
	/**
	 * The publisher the chunks go to. Its `subscribe` and `fetch` are for
	 * the handlers of `MoqtConnection.serve`; `push` and `end` are this
	 * encoder's to call.
	 */
	readonly publisher: VideoTrackPublisher;
	/**
	 * Takes the next frame of the source and **closes it**, encoded or not.
	 * Says whether it was handed to the encoder, or why not. Does not wait
	 * for the encoder and does not throw.
	 */
	encode(
		frame: VideoFrame,
		options?: {
			/**
			 * True makes this frame a key frame, whatever the interval says.
			 * For a caller that encodes the same frames for several tracks
			 * and has their Groups begin together (`wantsKeyFrame`).
			 */
			keyFrame?: boolean;
		},
	): "encoded" | VideoEncodeSkipReason;
	/**
	 * Whether the publisher asked for a key frame that was not encoded yet:
	 * a subscriber came, or a Group was cut. The next frame is one. A
	 * caller with several tracks of the same frames asks each, and gives
	 * `keyFrame: true` to all when one says so: their Groups then begin
	 * with the same frame and have the same numbers (the Groups are counted
	 * by the key frames, also while nobody is subscribed).
	 */
	wantsKeyFrame(): boolean;
	/**
	 * The source stopped (a camera turned off): the Group that is being sent
	 * ends now (`VideoTrackPublisher.pause`), and what the encoder still
	 * gives of the frames before is not sent. The next frame is a key frame,
	 * which begins the next Group. Its time is the source's, so the time the
	 * source stood still is in the times of the track, as it is in those of
	 * a sound that stood still as long.
	 */
	pause(): void;
	/**
	 * What the encoder still holds is encoded and sent, for `endTimeoutMs`
	 * at most; then the encoder is closed and the track is ended
	 * (`VideoTrackPublisher.end`, which has its own waits). Never rejects.
	 */
	end(): Promise<void>;
	/**
	 * Resolves with the error when the encoder fails; never when it does
	 * not. From then on no frame is encoded; the track is not ended, which
	 * is the caller's to do with `end`.
	 */
	readonly failed: Promise<unknown>;
	/** What was counted so far; a copy. */
	stats(): VideoTrackEncoderStats;
}

/**
 * Makes the publisher of a video track and the encoder that feeds it.
 *
 * **Rejects** with a `RangeError` for a limit or an interval that is no
 * whole number from 1 and for options the publisher refuses, and with a
 * `DOMException` named NotSupportedError where the browser has no
 * `VideoEncoder` or none for this configuration
 * (`VideoEncoder.isConfigSupported`), and as `isConfigSupported` does for
 * a configuration that is none, with a `TypeError`.
 *
 * - The encoder is configured for Annex B and for low delay:
 *   `{ codec, width, height, avc: { format: "annexb" }, latencyMode:
 *   "realtime" }`, with `bitrate` and `framerate` where they are given.
 *   Annex B is what the sender of this package sends (`VideoChunk.data`).
 * - **A key frame** is asked of the encoder for the first frame, for a
 *   frame `keyFrameIntervalMs` or more after the last key frame, and for
 *   the next frame after the publisher asked for one (a Group that was cut
 *   short, a subscriber that came, a subscriber that asked for a new Group).
 * - **A frame that comes while more than `maxEncodeQueue` wait in the
 *   encoder is not encoded.** WHY dropped before the encoder and not after:
 *   a frame that was never encoded is missed by no later one, so no Group
 *   is cut for it.
 * - **The time of an Object** (the Timestamp of LOC, wall-clock
 *   microseconds) is the wall-clock time at which the first frame came,
 *   plus the time of the frame since the first one, as the frames say it.
 *   WHY the clock is read once: a clock that is set while the track runs
 *   would make the times jump, forwards or back, and a decoder is given
 *   them in the order of the frames. So the times go up as the frames'
 *   own do, whatever the clock does afterwards. The time of the first frame
 *   need not be 0. With `clock: "least"` the clock is read with every
 *   frame instead and the times follow it over minutes (`StampClock` has
 *   why, and what it costs).
 * - **A frame whose time is not after the time of the frame before it is
 *   not encoded** (`time-not-after`): the receiver of this package and a
 *   decoder take the times to go up, and the key frame interval is counted
 *   in them.
 * - **An encoder that fails is not made anew.** Provisional: what makes
 *   one fail is taken to be the configuration or the frames, which a new
 *   one would meet again.
 */
export async function createVideoTrackEncoder(
	options: VideoTrackEncoderOptions,
): Promise<VideoTrackEncoder> {
	const maxEncodeQueue = options.maxEncodeQueue ?? DEFAULT_MAX_ENCODE_QUEUE;
	const maxUnsentKeyChunks =
		options.maxUnsentKeyChunks ?? DEFAULT_MAX_UNSENT_KEY_CHUNKS;
	const endTimeoutMs = options.endTimeoutMs ?? DEFAULT_END_TIMEOUT_MS;
	for (const [name, value] of [
		["maxEncodeQueue", maxEncodeQueue],
		["maxUnsentKeyChunks", maxUnsentKeyChunks],
		["keyFrameIntervalMs", options.keyFrameIntervalMs],
	] as const) {
		if (!Number.isInteger(value) || value < 1) {
			throw new RangeError(`${name} is a whole number from 1, not ${value}`);
		}
	}
	// Where the frames' time begins, for the sender: the moment this is
	// made. The first frame comes later, by `sinceEpoch`.
	const epoch = BigInt(Date.now()) * 1000n;
	const publisher = createVideoTrackPublisher({
		trackNamespace: options.trackNamespace,
		trackName: options.trackName,
		firstGroup: options.firstGroup,
		epoch,
		...(options.mapping === undefined ? {} : { mapping: options.mapping }),
		...(options.maxPayloadLength === undefined
			? {}
			: { maxPayloadLength: options.maxPayloadLength }),
		...(options.dynamicGroups === undefined
			? {}
			: { dynamicGroups: options.dynamicGroups }),
		endTimeoutMs,
	});
	if (typeof VideoEncoder === "undefined") {
		throw new DOMException(
			"this browser has no VideoEncoder (WebCodecs)",
			"NotSupportedError",
		);
	}
	const config: VideoEncoderConfig = {
		codec: options.codec,
		width: options.width,
		height: options.height,
		avc: { format: "annexb" },
		latencyMode: "realtime",
		...(options.bitrate === undefined ? {} : { bitrate: options.bitrate }),
		...(options.framerate === undefined
			? {}
			: { framerate: options.framerate }),
	};
	const { supported } = await VideoEncoder.isConfigSupported(config);
	if (supported !== true) {
		throw new DOMException(
			`this browser has no encoder for ${options.codec} at ${options.width}x${options.height}`,
			"NotSupportedError",
		);
	}

	const stats: VideoTrackEncoderStats = {
		submitted: 0,
		encoded: 0,
		keyChunks: 0,
		notEncoded: {},
		notSent: {},
		keyFramesAskedFor: 0,
		unsentKeyChunksInARow: 0,
		encodedBytes: 0,
		largestChunk: 0,
		encoderError: undefined,
	};
	const notEncoded = (
		because: VideoEncodeSkipReason,
	): VideoEncodeSkipReason => {
		stats.notEncoded[because] = (stats.notEncoded[because] ?? 0) + 1;
		return because;
	};
	const stamp = createStamp(options.clock ?? "first", Number(epoch));
	/** The time of the last frame that was asked to be a key frame. */
	let lastKey: number | undefined;
	/** The publisher asked for a key frame, and no frame was encoded since. */
	let wantsKey = false;
	/** The time of the last frame that was handed to the encoder. */
	let lastTime: number | undefined;
	/** The time of the last frame before `pause`: what the encoder gives of it and of the frames before is not sent. */
	let pausedAfter: number | undefined;
	let closed = false;
	let fail: (error: unknown) => void = () => {};
	const failed = new Promise<unknown>((resolve) => {
		fail = resolve;
	});

	const encoder = new VideoEncoder({
		output: (chunk) => {
			if (pausedAfter !== undefined && chunk.timestamp <= pausedAfter) return;
			const wall = stamp.of(chunk.timestamp);
			if (wall === undefined) return;
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			stats.encoded += 1;
			if (chunk.type === "key") stats.keyChunks += 1;
			stats.encodedBytes += data.length;
			stats.largestChunk = Math.max(stats.largestChunk, data.length);
			const result = publisher.push({
				type: chunk.type,
				// Not below 0: with `"least"`, where the wall clock was set
				// back since `epoch`.
				timestamp: Math.max(0, wall - Number(epoch)),
				data,
			});
			if (result.skipped !== undefined) {
				stats.notSent[result.skipped] =
					(stats.notSent[result.skipped] ?? 0) + 1;
			}
			if (result.askForKeyFrame) {
				stats.keyFramesAskedFor += 1;
				wantsKey = true;
			}
			if (chunk.type !== "key") return;
			if (result.skipped === undefined) {
				stats.unsentKeyChunksInARow = 0;
				return;
			}
			// Not for want of a subscriber, or for one that is behind: those
			// are not the chunk's doing, and pass by themselves.
			const because = OF_THE_CHUNK.find((reason) => reason === result.skipped);
			if (because === undefined) return;
			stats.unsentKeyChunksInARow += 1;
			if (stats.unsentKeyChunksInARow !== maxUnsentKeyChunks) return;
			try {
				options.onKeyChunksNotSent?.({
					because,
					inARow: stats.unsentKeyChunksInARow,
					length: data.length,
				});
			} catch (error) {
				reportError(error);
			}
		},
		error: (error) => {
			// The encoder is closed by the browser when it says this.
			closed = true;
			stats.encoderError ??= error.message;
			fail(error);
		},
	});
	try {
		encoder.configure(config);
	} catch (error) {
		// Nobody else has it.
		encoder.close();
		throw error;
	}

	let ended: Promise<void> | undefined;
	return {
		publisher,
		wantsKeyFrame: () => wantsKey,
		pause() {
			pausedAfter = lastTime;
			publisher.pause();
			wantsKey = true;
		},
		encode(frame, { keyFrame: asked = false } = {}) {
			try {
				if (closed) return notEncoded("closed");
				if (lastTime !== undefined && frame.timestamp <= lastTime) {
					return notEncoded("time-not-after");
				}
				if (encoder.encodeQueueSize > maxEncodeQueue) {
					return notEncoded("encoder-behind");
				}
				stamp.take(frame.timestamp, Date.now());
				const keyFrame =
					asked ||
					wantsKey ||
					lastKey === undefined ||
					frame.timestamp - lastKey >= options.keyFrameIntervalMs * 1000;
				try {
					encoder.encode(frame, { keyFrame });
				} catch (error) {
					// `encode` throws for a frame the encoder cannot take, and
					// for an encoder that is closed.
					closed = true;
					stats.encoderError ??=
						error instanceof Error ? error.message : String(error);
					if (encoder.state !== "closed") encoder.close();
					fail(error);
					return notEncoded("closed");
				}
				if (keyFrame) {
					lastKey = frame.timestamp;
					wantsKey = false;
				}
				lastTime = frame.timestamp;
				stats.submitted += 1;
				return "encoded";
			} finally {
				frame.close();
			}
		},
		end() {
			ended ??= (async () => {
				const wasOpen = !closed;
				closed = true;
				if (wasOpen && encoder.state === "configured") {
					// What the encoder still holds becomes Objects first. WHY
					// with a time limit: nothing says that an encoder that
					// stands still answers a flush.
					let timer: ReturnType<typeof setTimeout> | undefined;
					await Promise.race([
						encoder.flush().catch(() => {}),
						new Promise((resolve) => {
							timer = setTimeout(resolve, endTimeoutMs);
						}),
					]);
					clearTimeout(timer);
				}
				if (encoder.state !== "closed") encoder.close();
				await publisher.end();
			})();
			return ended;
		},
		failed,
		stats: () => ({
			...stats,
			notEncoded: { ...stats.notEncoded },
			notSent: { ...stats.notSent },
		}),
	};
}
