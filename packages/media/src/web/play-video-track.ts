// A LOC video track played in a browser: the Objects of a subscription go
// through the receiver of this package into a decoder of WebCodecs, and the
// frames it decodes are handed to the caller. What is decided here is only
// what the decoder makes necessary; which frame comes when is the
// receiver's (`src/core/video-receiver.ts`).

import {
	decodeFetchBody,
	type MoqtConnection,
	type ReceivedObject,
	type SubscriptionEnd,
} from "@xmoq/moqt";
import {
	type DeliveryTimeouts,
	deliveryTimeoutParameters,
} from "../core/delivery-timeout.ts";
import {
	createVideoReceiver,
	type VideoDropReason,
	type VideoReceiverEvent,
} from "../core/video-receiver.ts";

/**
 * How many chunks may wait in the decoder before the Group that is played
 * is given up. **Provisional, not measured**: a third of a second at 25
 * frames. WHY a limit at all: a decoder that is slower than the track
 * would hold every chunk that comes, without end.
 */
export const DEFAULT_MAX_DECODE_QUEUE = 8;
/**
 * How many times in a row the decoder may fail, with no frame decoded
 * between, before the playing ends. **Provisional.** Each failure costs the
 * rest of a Group, so three are three Groups without a picture.
 */
export const DEFAULT_MAX_DECODER_FAILURES = 3;

export interface PlayVideoTrackOptions {
	trackNamespace: Uint8Array[];
	trackName: Uint8Array;
	/**
	 * The codec string of the track, as its catalog has it (`codec` of an
	 * MSF track). The caller reads the catalog; this knows nothing of one.
	 * H.264 with the parameter sets in the key frames is what was run.
	 */
	codec: string;
	/** The Timescale of the track, where it has one (`VideoReceiverOptions.trackTimescale`). */
	trackTimescale?: bigint;
	/**
	 * Called with each frame the decoder gives, in the order it gives them.
	 * **The frame is the caller's from then on, and the caller closes it**
	 * (`VideoFrame.close()`); a frame that is not closed keeps its memory.
	 */
	onFrame(frame: VideoFrame): void;
	/**
	 * How long the sender may take with an Object or the end of a Subgroup
	 * before it gives it up (`DeliveryTimeouts`, transport §8), sent with the
	 * SUBSCRIBE. **Nothing is sent when left out.** A stream the sender
	 * gives up is reset, and what was on it does not come: the Group ends
	 * there for the player.
	 */
	deliveryTimeouts?: DeliveryTimeouts;
	/** Aborting it ends the playing: the subscription is cancelled. */
	signal?: AbortSignal;
	/**
	 * Whether the Group the viewer comes into is fetched from its start (a
	 * Relative Joining FETCH with Joining Start 0, transport §10.12.2), so
	 * that the picture begins at once and not with the next key frame.
	 * **True when left out; provisional.** A relay that keeps the latest
	 * Group answers it; where nobody does, the FETCH is refused and the
	 * playing goes on as without it.
	 */
	joiningFetch?: boolean;
	/** `DEFAULT_MAX_DECODE_QUEUE` when left out. */
	maxDecodeQueue?: number;
	/** `DEFAULT_MAX_DECODER_FAILURES` when left out. */
	maxDecoderFailures?: number;
}

/** How the playing ended. */
export type VideoPlaybackEnd =
	/** The subscription ended, and `end` says how; what the decoder still held was decoded first. */
	| { type: "subscription-ended"; end: SubscriptionEnd }
	/** The decoder failed `maxDecoderFailures` times in a row; the subscription was cancelled. */
	| { type: "decoder-failed"; error: unknown }
	/** Something here failed that is no decoder's doing; the subscription was cancelled, and the error was reported with `reportError`. */
	| { type: "failed"; error: unknown };

/** What was counted while a track was played. */
export interface VideoPlaybackStats {
	/** Frames the decoder gave. */
	decoded: number;
	/** Frames of the receiver that were given to the decoder. */
	submitted: number;
	/** Objects that did not become a frame for the decoder, by the reason. */
	dropped: Partial<Record<VideoDropReason, number>>;
	/** Times the decoder failed and was made anew. */
	decoderFailures: number;
	/** What the decoder said the last time it failed. */
	lastDecoderError: string | undefined;
	/** Whether the publisher said that the track has ended. */
	endOfTrack: boolean;
	/**
	 * What became of the Joining FETCH: "not-sent", "pending", "served"
	 * with how many Objects it brought, or "failed" with them and the
	 * reason.
	 */
	joiningFetch: string;
}

export interface VideoPlayback {
	/** Resolves once the playing is over, with how. Never rejects. */
	readonly ended: Promise<VideoPlaybackEnd>;
	/** What was counted so far; a copy. */
	stats(): VideoPlaybackStats;
}

/**
 * Subscribes to a video track and decodes it. Resolves when the SUBSCRIBE
 * was accepted, and rejects as `MoqtConnection.subscribe` does.
 *
 * **Rejects before anything is sent** with a `RangeError` for a limit that
 * is no whole number from 1, with the reason of `signal` where that is
 * aborted already, and with a `DOMException` named NotSupportedError where
 * the browser has no `VideoDecoder` or no decoder for `codec`
 * (`VideoDecoder.isConfigSupported`): a viewer without one is told so, and
 * not that no frame came. A codec string that is none rejects as
 * `isConfigSupported` does, with a `TypeError`.
 *
 * - The decoder is configured for Annex B, without a description:
 *   `{ codec, optimizeForLatency: true }`.
 * - **A decoder that fails is closed and made anew**, and the Group it
 *   failed in is given up: the next chunk it gets is the key frame of a
 *   later Group. After `maxDecoderFailures` failures with no frame decoded
 *   between, the playing ends.
 * - **A decoder that does not keep up loses the rest of a Group**: when
 *   more than `maxDecodeQueue` chunks wait in it (`decodeQueueSize`, which
 *   counts what the decoder has not begun with, and not what is inside
 *   it), the Group is given up and nothing is given to the decoder until
 *   the key frame of a later Group.
 * - An `end-of-track` of the receiver is counted, and the subscription goes
 *   on: it ends when the publisher ends it, or the caller.
 * - **The Group the viewer comes into is fetched** (`joiningFetch`): its
 *   Objects up to where the subscription begins come on a fetch stream and
 *   go through the same receiver, which puts them before the ones of the
 *   subscription and drops what comes twice (`late`). **While they come,
 *   and until the decoder has caught up once, a decoder with more than
 *   `maxDecodeQueue` chunks waiting is not taken to be behind**: the start
 *   of a Group comes all at once, and is what the picture begins with.
 *   These frames are older than the ones after them by up to a Group; they
 *   are handed on as fast as the decoder gives them.
 */
export async function playVideoTrack(
	connection: MoqtConnection,
	options: PlayVideoTrackOptions,
): Promise<VideoPlayback> {
	const maxDecodeQueue = options.maxDecodeQueue ?? DEFAULT_MAX_DECODE_QUEUE;
	const maxDecoderFailures =
		options.maxDecoderFailures ?? DEFAULT_MAX_DECODER_FAILURES;
	for (const [name, value] of [
		["maxDecodeQueue", maxDecodeQueue],
		["maxDecoderFailures", maxDecoderFailures],
	] as const) {
		if (!Number.isInteger(value) || value < 1) {
			throw new RangeError(`${name} is a whole number from 1, not ${value}`);
		}
	}
	// Before anything is sent: a time that is none throws here.
	const parameters = deliveryTimeoutParameters(options.deliveryTimeouts);
	options.signal?.throwIfAborted();
	if (typeof VideoDecoder === "undefined") {
		throw new DOMException(
			"this browser has no VideoDecoder (WebCodecs)",
			"NotSupportedError",
		);
	}
	const config: VideoDecoderConfig = {
		codec: options.codec,
		optimizeForLatency: true,
	};
	const { supported } = await VideoDecoder.isConfigSupported(config);
	if (supported !== true) {
		throw new DOMException(
			`this browser has no decoder for ${options.codec}`,
			"NotSupportedError",
		);
	}
	const receiver = createVideoReceiver(
		options.trackTimescale === undefined
			? {}
			: { trackTimescale: options.trackTimescale },
	);
	const stats: VideoPlaybackStats = {
		decoded: 0,
		submitted: 0,
		dropped: {},
		decoderFailures: 0,
		lastDecoderError: undefined,
		endOfTrack: false,
		joiningFetch: "not-sent",
	};
	/** The Objects of the Joining FETCH are coming, or the decoder has not caught up with them. */
	let catchingUp = false;
	const drop = (because: VideoDropReason): void => {
		stats.dropped[because] = (stats.dropped[because] ?? 0) + 1;
	};

	/** Failures of the decoder since it last gave a frame. */
	let failuresInARow = 0;
	/** Set when the playing ends for another reason than its subscription; the reading stops then. */
	let gaveUp: VideoPlaybackEnd | undefined;
	/** The caller stopped the playing, or it is over: no frame is handed on any more. */
	let stopped = false;
	/** Set once the SUBSCRIBE was accepted. */
	let cancel: () => void = () => {};

	const newDecoder = (): VideoDecoder => {
		const made: VideoDecoder = new VideoDecoder({
			output: (frame) => {
				if (made !== decoder || stopped) {
					// Of a decoder that failed afterwards, or after the end: not
					// shown.
					frame.close();
					return;
				}
				stats.decoded += 1;
				failuresInARow = 0;
				try {
					options.onFrame(frame);
				} catch (error) {
					// The caller did not take the frame: it is closed here, which
					// does nothing where the caller closed it.
					frame.close();
					reportError(error);
				}
			},
			error: (error) => decoderFailed(made, error),
		});
		try {
			made.configure(config);
		} catch (error) {
			// Nobody else has it, and a decoder that is not closed keeps what
			// it holds.
			made.close();
			throw error;
		}
		return made;
	};
	// WHY before the SUBSCRIBE: a decoder that cannot be made, or configured,
	// is known before a subscription exists that would have to be cancelled.
	let decoder = newDecoder();

	/** The decoder failed, in `decode()` or by itself. */
	const decoderFailed = (failed: VideoDecoder, error: unknown): void => {
		// A failure of a decoder that was replaced already was counted.
		if (failed !== decoder || gaveUp !== undefined) return;
		// WHY closed here: a decoder that says so with its `error` callback
		// is closed already, and one whose `decode()` threw is not.
		if (failed.state !== "closed") failed.close();
		stats.decoderFailures += 1;
		stats.lastDecoderError =
			error instanceof Error ? error.message : String(error);
		failuresInARow += 1;
		if (failuresInARow >= maxDecoderFailures) {
			gaveUp = { type: "decoder-failed", error };
			cancel();
			return;
		}
		decoder = newDecoder();
		count(receiver.abandonGroup("decoder-failed"));
	};

	/** Counts what the receiver dropped, and gives its frames to the decoder. */
	const count = (events: VideoReceiverEvent[]): void => {
		/** A Group that was given up at one of these events: its later frames among them are counted, and not decoded. */
		let givenUp: { group: bigint; because: VideoDropReason } | undefined;
		for (const event of events) {
			if (event.type === "dropped") {
				drop(event.because);
				continue;
			}
			if (event.type === "end-of-track") {
				stats.endOfTrack = true;
				continue;
			}
			if (gaveUp !== undefined) {
				// A frame the receiver handed on after the playing was given up.
				drop("decoder-failed");
				continue;
			}
			if (givenUp?.group === event.group) {
				drop(givenUp.because);
				continue;
			}
			if (catchingUp && stats.joiningFetch !== "pending") {
				// The FETCH is over: caught up when the decoder has room.
				catchingUp = decoder.decodeQueueSize > maxDecodeQueue;
			}
			if (!catchingUp && decoder.decodeQueueSize > maxDecodeQueue) {
				// This frame, and what the receiver held of its Group.
				drop("decoder-behind");
				count(receiver.abandonGroup("decoder-behind"));
				givenUp = { group: event.group, because: "decoder-behind" };
				continue;
			}
			// WHY outside the `try` below: a chunk that cannot be made is no
			// failure of the decoder. The receiver sees to its fields, so it
			// throws for a fault here, which ends the playing as one.
			const chunk = new EncodedVideoChunk({
				type: event.key ? "key" : "delta",
				// At most 2^53 - 1, which the receiver sees to.
				timestamp: Number(event.timestamp),
				data: event.data,
			});
			const submitted = decoder;
			try {
				submitted.decode(chunk);
				stats.submitted += 1;
			} catch (error) {
				// `decode` throws for a chunk the decoder cannot begin with,
				// and for a decoder that is closed.
				drop("decoder-failed");
				decoderFailed(submitted, error);
				givenUp = { group: event.group, because: "decoder-failed" };
			}
		}
	};

	let subscription: Awaited<ReturnType<MoqtConnection["subscribe"]>>;
	try {
		subscription = await connection.subscribe({
			trackNamespace: options.trackNamespace,
			trackName: options.trackName,
			parameters,
		});
	} catch (error) {
		decoder.close();
		throw error;
	}
	cancel = () => {
		// Rejects when the session is gone, which ends the subscription too.
		subscription.cancel().catch(() => {});
	};
	const stopBySignal = (): void => {
		stopped = true;
		cancel();
	};
	options.signal?.addEventListener("abort", stopBySignal, { once: true });
	if (options.signal?.aborted) stopBySignal();

	/** Cancels the Joining FETCH; does nothing where there is none, or it is over. */
	let cancelJoiningFetch: () => void = () => {};
	if (options.joiningFetch !== false && !stopped) {
		stats.joiningFetch = "pending";
		catchingUp = true;
		let objects = 0;
		void (async () => {
			const fetch = await connection.fetch({
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: 0n,
				},
			});
			cancelJoiningFetch = () => {
				fetch.cancel().catch(() => {});
			};
			// A refusal is told by the body as well, below.
			fetch.response.catch(() => {});
			if (stopped || gaveUp !== undefined) cancelJoiningFetch();
			const reader = decodeFetchBody(connection, fetch).getReader();
			for (;;) {
				const next = await reader.read();
				if (next.done) break;
				if (next.value.type !== "object") continue;
				objects += 1;
				const { groupId, objectId, properties, payload } = next.value.object;
				count(
					receiver.push({
						groupId,
						object: { objectId, properties, status: "Normal", payload },
					}),
				);
			}
			stats.joiningFetch = `served ${objects}`;
		})().catch((error: unknown) => {
			// Refused, cut off, or cancelled: the subscription has the rest.
			stats.joiningFetch = `failed ${objects}: ${error instanceof Error ? error.message : String(error)}`;
		});
	}

	const ended = (async (): Promise<VideoPlaybackEnd> => {
		let reader: ReadableStreamDefaultReader<ReceivedObject> | undefined;
		try {
			reader = subscription.objects.getReader();
			for (;;) {
				let next: Awaited<ReturnType<typeof reader.read>>;
				try {
					next = await reader.read();
				} catch {
					// The subscription was cut off; `ended` of it says how.
					break;
				}
				if (next.done || gaveUp !== undefined) break;
				count(receiver.push(next.value));
			}
		} catch (error) {
			// Not the subscription and not the decoder: a fault here. The
			// subscription is not left open for it.
			gaveUp ??= { type: "failed", error };
			reportError(error);
		} finally {
			reader?.releaseLock();
			options.signal?.removeEventListener("abort", stopBySignal);
			// Does nothing for a subscription that has ended.
			cancel();
			cancelJoiningFetch();
		}
		const end = await subscription.ended;
		if (gaveUp === undefined && !stopped && decoder.state === "configured") {
			// What the decoder still holds comes out before the end is told.
			await decoder.flush().catch(() => {});
		}
		stopped = true;
		if (decoder.state !== "closed") decoder.close();
		return gaveUp ?? { type: "subscription-ended", end };
	})();

	return {
		ended,
		stats: () => ({ ...stats, dropped: { ...stats.dropped } }),
	};
}
