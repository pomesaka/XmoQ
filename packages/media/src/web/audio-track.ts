// A LOC audio track in a browser, both ways: `AudioData` into an encoder of
// WebCodecs and on to the audio publisher of this package, and the Objects
// of a subscription into a decoder. Minimal: no buffer for playback, no
// sync with video, no reordering.

import { microsecondsOf, readLocTiming } from "@xmoq/loc";
import {
	decodeFetchBody,
	type MoqtConnection,
	type SubscriptionEnd,
} from "@xmoq/moqt";
import {
	type DeliveryTimeouts,
	deliveryTimeoutParameters,
} from "../core/delivery-timeout.ts";
import { createFastestWay } from "../core/fastest-way.ts";
import { createInOrder } from "../core/in-order.ts";
import {
	type AudioTrackPublisher,
	createAudioTrackPublisher,
} from "../moqt/audio-track-publisher.ts";

export interface AudioTrackEncoderOptions {
	trackNamespace: readonly Uint8Array[];
	trackName: Uint8Array;
	/** The Group of the first chunk. */
	firstGroup: bigint;
	/** "opus" is what was run. */
	codec: string;
	sampleRate: number;
	numberOfChannels: number;
	/** Bits a second; the encoder's own choice when left out. */
	bitrate?: number;
	/** `AudioTrackPublisherOptions.chunksPerGroup`. */
	chunksPerGroup?: number;
	/** `AudioTrackPublisherOptions.publisherPriority`. */
	publisherPriority?: number;
}

export interface AudioTrackEncoderStats {
	/** `AudioData` handed to the encoder. */
	submitted: number;
	/** Chunks the encoder gave. */
	encoded: number;
	encodedBytes: number;
	/**
	 * Whether the encoder gave a `description` with its first chunk
	 * (`EncodedAudioChunkMetadata.decoderConfig`), and how long it was. It
	 * is not sent: whether a decoder is to get one is not decided.
	 */
	descriptionLength: number | undefined;
	encoderError: string | undefined;
}

export interface AudioTrackEncoder {
	/** Its `subscribe` and `fetch` are for the handlers of `MoqtConnection.serve`. */
	readonly publisher: AudioTrackPublisher;
	/** Takes the next audio of the source and **closes it**. */
	encode(data: AudioData): void;
	/** Flushes and closes the encoder, and ends the track. Never rejects. */
	end(): Promise<void>;
	stats(): AudioTrackEncoderStats;
}

/**
 * Makes the publisher of an audio track and the encoder that feeds it.
 * Rejects with a `DOMException` named NotSupportedError where the browser
 * has no `AudioEncoder` or none for the configuration.
 *
 * The time of an Object is the wall-clock time at which the first audio
 * came plus the time since then as the `AudioData` say it, as for video
 * (`createVideoTrackEncoder`).
 */
export async function createAudioTrackEncoder(
	options: AudioTrackEncoderOptions,
): Promise<AudioTrackEncoder> {
	if (typeof AudioEncoder === "undefined") {
		throw new DOMException(
			"this browser has no AudioEncoder (WebCodecs)",
			"NotSupportedError",
		);
	}
	const config: AudioEncoderConfig = {
		codec: options.codec,
		sampleRate: options.sampleRate,
		numberOfChannels: options.numberOfChannels,
		...(options.bitrate === undefined ? {} : { bitrate: options.bitrate }),
	};
	const { supported } = await AudioEncoder.isConfigSupported(config);
	if (supported !== true) {
		throw new DOMException(
			`this browser has no encoder for ${options.codec} at ${options.sampleRate} Hz`,
			"NotSupportedError",
		);
	}
	const epoch = BigInt(Date.now()) * 1000n;
	const publisher = createAudioTrackPublisher({
		trackNamespace: options.trackNamespace,
		trackName: options.trackName,
		firstGroup: options.firstGroup,
		epoch,
		...(options.chunksPerGroup === undefined
			? {}
			: { chunksPerGroup: options.chunksPerGroup }),
		...(options.publisherPriority === undefined
			? {}
			: { publisherPriority: options.publisherPriority }),
	});
	const stats: AudioTrackEncoderStats = {
		submitted: 0,
		encoded: 0,
		encodedBytes: 0,
		descriptionLength: undefined,
		encoderError: undefined,
	};
	let first: { timestamp: number; sinceEpoch: number } | undefined;
	let closed = false;
	const encoder = new AudioEncoder({
		output: (chunk, metadata) => {
			if (first === undefined) return;
			const description = metadata?.decoderConfig?.description;
			if (stats.encoded === 0) {
				stats.descriptionLength =
					description === undefined ? undefined : description.byteLength;
			}
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			stats.encoded += 1;
			stats.encodedBytes += data.length;
			publisher.push({
				timestamp: chunk.timestamp - first.timestamp + first.sinceEpoch,
				data,
			});
		},
		error: (error) => {
			closed = true;
			stats.encoderError ??= error.message;
		},
	});
	encoder.configure(config);
	return {
		publisher,
		encode(data) {
			try {
				if (closed) return;
				first ??= {
					timestamp: data.timestamp,
					sinceEpoch: Math.max(0, Number(BigInt(Date.now()) * 1000n - epoch)),
				};
				encoder.encode(data);
				stats.submitted += 1;
			} catch (error) {
				closed = true;
				stats.encoderError ??=
					error instanceof Error ? error.message : String(error);
			} finally {
				data.close();
			}
		},
		async end() {
			const wasOpen = !closed;
			closed = true;
			if (wasOpen && encoder.state === "configured") {
				await encoder.flush().catch(() => {});
			}
			if (encoder.state !== "closed") encoder.close();
			await publisher.end();
		},
		stats: () => ({ ...stats }),
	};
}

export interface PlayAudioTrackOptions {
	trackNamespace: Uint8Array[];
	trackName: Uint8Array;
	/** "opus" is what was run. No `description` is given to the decoder. */
	codec: string;
	sampleRate: number;
	numberOfChannels: number;
	/** Called with each decoded piece of audio; **the callee closes it**. */
	onData(data: AudioData): void;
	/**
	 * Whether the Group the listener comes into is fetched from its start (a
	 * Relative Joining FETCH with Joining Start 0). **False when left out;
	 * for trying**: it matters where a Group has several chunks, and what
	 * it brings is older than what the subscription brings.
	 */
	joiningFetch?: boolean;
	/**
	 * How long the sender may take with an Object or the end of a Subgroup
	 * before it gives it up (`DeliveryTimeouts`, transport §8), sent with the
	 * SUBSCRIBE. **Nothing is sent when left out.** A stream the sender
	 * gives up is reset, and what was on it does not come: the Group ends
	 * there for the player.
	 */
	deliveryTimeouts?: DeliveryTimeouts;
	/**
	 * How long an Object that comes after a gap waits for the ones before
	 * it, in milliseconds, before it is decoded without them; asked each
	 * time. **Nothing waits when left out**: Objects are decoded as they
	 * come. With loss, the rest of a Group comes after the next Group has
	 * begun (each Group is a stream, and a lost packet holds back its
	 * stream alone); a player with a buffer can wait that long for it.
	 */
	holdForOrderMs?: () => number;
	signal?: AbortSignal;
}

/**
 * How many chunks may wait in the decoder before one more is dropped.
 * **Provisional, not measured**: a second of pieces of 20 ms.
 */
export const MAX_AUDIO_DECODE_QUEUE = 50;

export interface AudioPlaybackStats {
	/** Objects that came. */
	received: number;
	/** Pieces of audio the decoder gave. */
	decoded: number;
	/**
	 * Objects that were not decoded: "late" (one with a later time was
	 * decoded already), "no-time", "empty", "not-of-the-clock" (its time is
	 * more than 10 seconds from where the fastest one puts it,
	 * `createFastestWay`) and "decoder-behind" (more than
	 * `MAX_AUDIO_DECODE_QUEUE` chunks waited in the decoder).
	 */
	dropped: Partial<
		Record<
			"late" | "no-time" | "empty" | "not-of-the-clock" | "decoder-behind",
			number
		>
	>;
	/** Objects that did not go to the decoder when they came: they came after a gap, and waited for what was missing (`holdForOrderMs`). */
	held: number;
	decoderError: string | undefined;
}

export interface AudioPlayback {
	/** Resolves once the subscription is over, with how. Never rejects. */
	readonly ended: Promise<SubscriptionEnd>;
	stats(): AudioPlaybackStats;
}

/**
 * Subscribes to an audio track and decodes it. **Objects are decoded in the
 * order of their times**; one whose time is not after the last one decoded
 * is dropped (`late`). An Object after a gap waits `holdForOrderMs` for
 * the ones before it (not at all when that is left out), and is then
 * decoded without them: the decoder is not told of what is missing.
 */
/**
 * `data` with the time `timestamp`, in microseconds: itself where it has
 * that time already (within a millisecond: a decoder rounds), and a copy
 * with that time else; `data` is then closed.
 */
function withTime(data: AudioData, timestamp: number | undefined): AudioData {
	if (timestamp === undefined || Math.abs(data.timestamp - timestamp) < 1000) {
		return data;
	}
	const samples = new Float32Array(data.numberOfFrames * data.numberOfChannels);
	for (let channel = 0; channel < data.numberOfChannels; channel++) {
		data.copyTo(
			samples.subarray(
				channel * data.numberOfFrames,
				(channel + 1) * data.numberOfFrames,
			),
			{ planeIndex: channel, format: "f32-planar" },
		);
	}
	const copy = new AudioData({
		format: "f32-planar",
		sampleRate: data.sampleRate,
		numberOfFrames: data.numberOfFrames,
		numberOfChannels: data.numberOfChannels,
		timestamp,
		data: samples,
	});
	data.close();
	return copy;
}

export async function playAudioTrack(
	connection: MoqtConnection,
	options: PlayAudioTrackOptions,
): Promise<AudioPlayback> {
	if (typeof AudioDecoder === "undefined") {
		throw new DOMException(
			"this browser has no AudioDecoder (WebCodecs)",
			"NotSupportedError",
		);
	}
	const config: AudioDecoderConfig = {
		codec: options.codec,
		sampleRate: options.sampleRate,
		numberOfChannels: options.numberOfChannels,
	};
	const { supported } = await AudioDecoder.isConfigSupported(config);
	if (supported !== true) {
		throw new DOMException(
			`this browser has no decoder for ${options.codec}`,
			"NotSupportedError",
		);
	}
	const stats: AudioPlaybackStats = {
		received: 0,
		decoded: 0,
		dropped: {},
		held: 0,
		decoderError: undefined,
	};
	const drop = (because: keyof AudioPlaybackStats["dropped"]): void => {
		stats.dropped[because] = (stats.dropped[because] ?? 0) + 1;
	};
	/**
	 * The times of the chunks that are in the decoder, in the order they
	 * went in: a decoder of Opus gives one piece for each chunk.
	 *
	 * WHY the time is not taken from what the decoder gives: Chromium's
	 * decoder numbers its output from the time of the first chunk, by the
	 * samples it has given, and does not look at the times of the chunks
	 * after it (Chromium 156). Each chunk that never went in (one dropped
	 * as late, one that was lost) then leaves everything after it 20 ms
	 * early for good: with 17 of 971 dropped, the pieces said they were
	 * made 340 ms before they were, and a player that goes by the time
	 * took all of them for late.
	 */
	const timesIn: number[] = [];
	const decoder = new AudioDecoder({
		output: (decoded) => {
			stats.decoded += 1;
			const data = withTime(decoded, timesIn.shift());
			try {
				options.onData(data);
			} catch (error) {
				data.close();
				reportError(error);
			}
		},
		error: (error) => {
			stats.decoderError ??= error.message;
		},
	});
	decoder.configure(config);
	let subscription: Awaited<ReturnType<MoqtConnection["subscribe"]>>;
	try {
		subscription = await connection.subscribe({
			trackNamespace: options.trackNamespace,
			trackName: options.trackName,
			parameters: deliveryTimeoutParameters(options.deliveryTimeouts),
		});
	} catch (error) {
		decoder.close();
		throw error;
	}
	const stop = (): void => {
		subscription.cancel().catch(() => {});
	};
	options.signal?.addEventListener("abort", stop, { once: true });
	if (options.signal?.aborted) stop();
	/** How many Objects went on to the decoder, or were dropped as late: one that is neither when `push` returns is held. */
	let passed = 0;
	const decode = (chunk: { timestamp: number; data: Uint8Array }): void => {
		passed += 1;
		if (decoder.state !== "configured") return;
		// WHY a limit: how fast chunks come is the publisher's to decide, and
		// a decoder that is slower would hold every one of them.
		if (decoder.decodeQueueSize > MAX_AUDIO_DECODE_QUEUE) {
			drop("decoder-behind");
			return;
		}
		timesIn.push(chunk.timestamp);
		decoder.decode(new EncodedAudioChunk({ type: "key", ...chunk }));
	};
	const order = createInOrder<{ timestamp: number; data: Uint8Array }>({
		holdMs: options.holdForOrderMs ?? (() => 0),
		next: decode,
		late: () => {
			passed += 1;
			drop("late");
		},
	});
	const fastest = createFastestWay();
	let release: ReturnType<typeof setTimeout> | undefined;
	/** Lets on what waited long enough, and comes back when the next has. */
	const releaseInTime = (): void => {
		clearTimeout(release);
		order.release(performance.now());
		const at = order.nextRelease();
		if (at === undefined) return;
		release = setTimeout(releaseInTime, Math.max(0, at - performance.now()));
	};
	/** Decodes the Objects in the order of their times. */
	const take = (
		_groupId: bigint,
		object: {
			objectId: bigint;
			properties: Parameters<typeof readLocTiming>[0];
			payload: Uint8Array;
		},
	): void => {
		stats.received += 1;
		if (object.payload.length === 0) {
			drop("empty");
			return;
		}
		const read = readLocTiming(object.properties);
		if (read.type !== "ok" || read.timing === undefined) {
			drop("no-time");
			return;
		}
		const timestamp = Number(microsecondsOf(read.timing));
		// The time is the publisher's to say. One that is far from the
		// others would be taken for the latest, and every piece after it
		// dropped as late; it is left out, and after 30 of them in a row the
		// order begins anew.
		const came = Date.now();
		const way = fastest.take(came - timestamp / 1000, came);
		if (way === undefined) {
			drop("not-of-the-clock");
			return;
		}
		if (way.anew) order.anew();
		const before = passed;
		order.push(
			timestamp,
			{ timestamp, data: object.payload },
			performance.now(),
		);
		// Nothing went on: this one waits. (One that lets earlier ones on
		// with it is not counted, and it did not wait.)
		if (passed === before) stats.held += 1;
		releaseInTime();
	};
	if (options.joiningFetch === true) {
		// What it brings comes before what the subscription brings only
		// where it is there first; what comes after is dropped as late.
		void (async () => {
			const fetch = await connection.fetch({
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: 0n,
				},
			});
			fetch.response.catch(() => {});
			const reader = decodeFetchBody(connection, fetch).getReader();
			for (;;) {
				const next = await reader.read();
				if (next.done) return;
				if (next.value.type !== "object") continue;
				take(next.value.object.groupId, next.value.object);
			}
		})().catch(() => {});
	}
	const ended = (async (): Promise<SubscriptionEnd> => {
		const reader = subscription.objects.getReader();
		for (;;) {
			let next: Awaited<ReturnType<typeof reader.read>>;
			try {
				next = await reader.read();
			} catch {
				break;
			}
			if (next.done) break;
			take(next.value.groupId, next.value.object);
		}
		options.signal?.removeEventListener("abort", stop);
		clearTimeout(release);
		const end = await subscription.ended;
		if (decoder.state === "configured") await decoder.flush().catch(() => {});
		if (decoder.state !== "closed") decoder.close();
		return end;
	})();
	return {
		ended,
		stats: () => ({ ...stats, dropped: { ...stats.dropped } }),
	};
}
