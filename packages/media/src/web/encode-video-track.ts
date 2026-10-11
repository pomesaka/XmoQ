// A LOC video track made in a browser: frames go into an encoder of
// WebCodecs, and its chunks into the publisher of this package
// (`src/moqt/video-track-publisher.ts`). What is decided here is only what
// the encoder makes necessary; which chunk becomes which Object is the
// sender's, and what is written when is the publisher's.

import type { VideoStreamMapping } from "../core/video-sender.ts";
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
	/** How the Objects go onto streams; the sender's default when left out. */
	mapping?: VideoStreamMapping;
	/** `DEFAULT_MAX_ENCODE_QUEUE` when left out. */
	maxEncodeQueue?: number;
}

/** Why a frame did not become an Object, besides the reasons of the publisher. */
export type VideoEncodeSkipReason =
	/** More frames wait in the encoder than may: the frame was not encoded. */
	| "encoder-behind"
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
	encode(frame: VideoFrame): "encoded" | VideoEncodeSkipReason;
	/**
	 * What the encoder still holds is encoded and sent, the encoder is
	 * closed, and the track is ended (`VideoTrackPublisher.end`). Never
	 * rejects.
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
 * (`VideoEncoder.isConfigSupported`).
 *
 * - The encoder is configured for Annex B and for low delay:
 *   `{ codec, width, height, avc: { format: "annexb" }, latencyMode:
 *   "realtime" }`, with `bitrate` and `framerate` where they are given.
 *   Annex B is what the sender of this package sends (`VideoChunk.data`).
 * - **A key frame** is asked of the encoder for the first frame, for a
 *   frame `keyFrameIntervalMs` or more after the last key frame, and for
 *   the next frame after the publisher asked for one (a Group that was cut
 *   short, a subscriber that came).
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
 *   need not be 0.
 * - **An encoder that fails is not made anew.** Provisional: what makes
 *   one fail is taken to be the configuration or the frames, which a new
 *   one would meet again.
 */
export async function createVideoTrackEncoder(
	options: VideoTrackEncoderOptions,
): Promise<VideoTrackEncoder> {
	const maxEncodeQueue = options.maxEncodeQueue ?? DEFAULT_MAX_ENCODE_QUEUE;
	for (const [name, value] of [
		["maxEncodeQueue", maxEncodeQueue],
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
	/** The time of the first frame, as the frames say it, and how long after `epoch` it came; both in microseconds. */
	let first: { timestamp: number; sinceEpoch: number } | undefined;
	/** The time of the last frame that was asked to be a key frame. */
	let lastKey: number | undefined;
	/** The publisher asked for a key frame, and no frame was encoded since. */
	let wantsKey = false;
	let closed = false;
	let fail: (error: unknown) => void = () => {};
	const failed = new Promise<unknown>((resolve) => {
		fail = resolve;
	});

	const encoder = new VideoEncoder({
		output: (chunk) => {
			if (first === undefined) return;
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			stats.encoded += 1;
			if (chunk.type === "key") stats.keyChunks += 1;
			stats.encodedBytes += data.length;
			stats.largestChunk = Math.max(stats.largestChunk, data.length);
			const result = publisher.push({
				type: chunk.type,
				timestamp: chunk.timestamp - first.timestamp + first.sinceEpoch,
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
		},
		error: (error) => {
			// The encoder is closed by the browser when it says this.
			closed = true;
			stats.encoderError = error.message;
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
		encode(frame) {
			try {
				if (closed) return notEncoded("closed");
				if (encoder.encodeQueueSize > maxEncodeQueue) {
					return notEncoded("encoder-behind");
				}
				first ??= {
					timestamp: frame.timestamp,
					// Not below 0, where the clock was set back since `epoch`.
					sinceEpoch: Math.max(0, Number(BigInt(Date.now()) * 1000n - epoch)),
				};
				const keyFrame =
					wantsKey ||
					lastKey === undefined ||
					frame.timestamp - lastKey >= options.keyFrameIntervalMs * 1000;
				try {
					encoder.encode(frame, { keyFrame });
				} catch (error) {
					// `encode` throws for a frame the encoder cannot take, and
					// for an encoder that is closed.
					closed = true;
					stats.encoderError =
						error instanceof Error ? error.message : String(error);
					if (encoder.state !== "closed") encoder.close();
					fail(error);
					return notEncoded("closed");
				}
				if (keyFrame) {
					lastKey = frame.timestamp;
					wantsKey = false;
				}
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
					// What the encoder still holds becomes Objects first.
					await encoder.flush().catch(() => {});
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
