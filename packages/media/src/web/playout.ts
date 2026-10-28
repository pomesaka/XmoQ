// Picture and sound of a broadcast played together in a browser: the
// decoded frames and pieces of audio go in, each is held for one buffer
// after the time it carries, and the frame is drawn at a repaint and the
// piece given to an `AudioContext`. When each is played is decided in
// `src/core` (`playout-clock.ts`, `frame-queue.ts`, `audio-seam.ts`); here
// is only what touches the browser.

import { placePiece } from "../core/audio-seam.ts";
import {
	createFrameQueue,
	DEFAULT_HIDE_AFTER_MS,
	DEFAULT_MAX_WAITING_FRAMES,
	type WaitingFrame,
} from "../core/frame-queue.ts";
import {
	createPlayoutClock,
	type PlayoutClock,
	type PlayoutClockOptions,
} from "../core/playout-clock.ts";

/** What became of a piece of audio. */
export interface PlayedPiece {
	/** Its time, in microseconds, and when it is to sound, on the clock of `now` (`Date.now()`), in milliseconds. */
	timestampUs: number;
	due: number;
	now: number;
	/** The samples of its first channel. */
	samples: Float32Array;
	/**
	 * "sounded": given to the `AudioContext`. "too-late": its time was over.
	 * While no sound runs (`startSound` was not called, or the context is
	 * suspended) nothing is sounded, and what would have been is told:
	 * "unheard", and "unheard-too-late".
	 */
	result: "sounded" | "too-late" | "unheard" | "unheard-too-late";
	/** With "sounded": whether it followed the piece before it to the sample, and whether it was played slower or faster. */
	follows: boolean;
	stretched: boolean;
}

/** A frame that was drawn. */
export interface ShownFrame extends WaitingFrame<VideoFrame> {
	/** When its time was (its base and the buffer), and when it was drawn, in milliseconds of `Date.now()`. */
	turn: number;
	now: number;
}

export interface PlayoutOptions extends PlayoutClockOptions {
	/** The sample rate of the audio, for the `AudioContext`. */
	sampleRate: number;
	/** Draws a frame, at a repaint. The frame is closed afterwards, here. */
	draw(frame: VideoFrame): void;
	/**
	 * Takes the picture away (`hideAfterMs` without a frame); called once,
	 * and not again before a frame was drawn. Where left out, the last
	 * picture stays.
	 */
	hide?(): void;
	/** `DEFAULT_HIDE_AFTER_MS` when left out; 0 never hides. */
	hideAfterMs?: number;
	/** `DEFAULT_MAX_WAITING_FRAMES` when left out. */
	maxWaitingFrames?: number;
	/** Called at a repaint with the frames whose time has come, before the last of them is drawn and all are closed. */
	onFramesDue?(due: WaitingFrame<VideoFrame>[], now: number): void;
	onFrameShown?(shown: ShownFrame): void;
	onPiece?(piece: PlayedPiece): void;
	/**
	 * Called with a frame or a piece that is left out because the time it
	 * carries is not of the clock: more than 10 seconds from where the
	 * fastest one puts it (`createFastestWay`).
	 */
	onStray?(kind: "video" | "audio", timestampUs: number): void;
	/**
	 * Connects what sounds to where it is heard; to the destination of the
	 * context when left out. For a caller that listens to what leaves.
	 */
	connectOutput?(output: GainNode, context: AudioContext): void;
}

export interface Playout {
	/** The buffer: set it, or let it follow the link, while playing. */
	readonly clock: PlayoutClock;
	/** For `onFrame` of `playVideoTrack`. The frame is closed here. */
	onFrame(frame: VideoFrame): void;
	/** For `onData` of `playAudioTrack`. The piece is closed here. */
	onData(data: AudioData): void;
	/**
	 * Makes the `AudioContext`, or resumes it. **To be called from a click**:
	 * a browser lets a page make sound only after one. Until then the
	 * picture plays alone.
	 */
	startSound(): AudioContext;
	sound(): AudioContext | undefined;
	waitingFrames(): number;
	/** Stops the repaints, closes what waits and the `AudioContext`. */
	close(): void;
}

/**
 * Plays the frames and pieces it is given by one clock (`PlayoutClock`).
 * A frame waits until its time and is drawn at the next repaint
 * (`requestAnimationFrame`), the newest one that is due; the ones before it
 * are passed over. A piece of audio is given to the `AudioContext` for its
 * time, joined to the piece before it (`placePiece`).
 *
 * The automatic buffer is moved twice a second by a timer of the page.
 */
export function createPlayout(options: PlayoutOptions): Playout {
	const clock = createPlayoutClock(options);
	const hideAfterMs = options.hideAfterMs ?? DEFAULT_HIDE_AFTER_MS;
	/** When the last frame was drawn, by `Date.now()`; undefined before one was, and once the picture was taken away. */
	let lastDrawn: number | undefined;
	const frames = createFrameQueue<VideoFrame>(
		options.maxWaitingFrames ?? DEFAULT_MAX_WAITING_FRAMES,
	);
	let closed = false;
	let context: AudioContext | undefined;
	let output: GainNode | undefined;
	/** Where the last piece that was given to the AudioContext ends, by its clock. */
	let soundUntil: number | undefined;

	const repaint = (): void => {
		if (closed) return;
		const now = Date.now();
		const bufferMs = clock.bufferMs();
		const due = frames.due(now, bufferMs);
		const show = due.at(-1);
		if (show !== undefined) {
			try {
				options.onFramesDue?.(due, now);
				options.draw(show.frame);
				options.onFrameShown?.({ ...show, turn: show.base + bufferMs, now });
			} catch (error) {
				reportError(error);
			}
			lastDrawn = now;
			for (const each of due) each.frame.close();
		} else if (
			hideAfterMs > 0 &&
			lastDrawn !== undefined &&
			now - lastDrawn >= hideAfterMs
		) {
			lastDrawn = undefined;
			try {
				options.hide?.();
			} catch (error) {
				reportError(error);
			}
		}
		requestAnimationFrame(repaint);
	};
	requestAnimationFrame(repaint);
	const ticking = setInterval(() => clock.tick(Date.now()), 500);

	return {
		clock,
		onFrame(frame) {
			if (closed) {
				frame.close();
				return;
			}
			const now = Date.now();
			const base = clock.baseOf(frame.timestamp, now);
			if (base === undefined) {
				// Not of the clock (`PlayoutClock.baseOf`): never shown.
				options.onStray?.("video", frame.timestamp);
				frame.close();
				return;
			}
			const gone = frames.push({ frame, base, came: now });
			for (const each of gone) each.frame.close();
		},
		onData(data) {
			const channels = Array.from(
				{ length: data.numberOfChannels },
				(_, planeIndex) => {
					const samples = new Float32Array(data.numberOfFrames);
					data.copyTo(samples, { planeIndex, format: "f32-planar" });
					return samples;
				},
			);
			const timestampUs = data.timestamp;
			data.close();
			const [samples] = channels;
			if (closed || samples === undefined) return;
			const now = Date.now();
			const base = clock.baseOf(timestampUs, now);
			if (base === undefined) {
				options.onStray?.("audio", timestampUs);
				return;
			}
			const due = base + clock.audioBufferMs();
			const told = (
				result: PlayedPiece["result"],
				follows = false,
				stretched = false,
			): void => {
				options.onPiece?.({
					timestampUs,
					due,
					now,
					samples,
					result,
					follows,
					stretched,
				});
			};
			if (
				context === undefined ||
				output === undefined ||
				context.state !== "running"
			) {
				told(due < now ? "unheard-too-late" : "unheard");
				return;
			}
			const buffer = context.createBuffer(
				channels.length,
				samples.length,
				options.sampleRate,
			);
			const placed = placePiece({
				wanted: context.currentTime + (due - now) / 1000,
				soundUntil,
				now: context.currentTime,
				duration: buffer.duration,
			});
			if (placed.type === "too-late") {
				told("too-late");
				return;
			}
			channels.forEach((channel, index) => {
				buffer.copyToChannel(channel, index);
			});
			const source = context.createBufferSource();
			source.buffer = buffer;
			source.connect(output);
			source.playbackRate.value = placed.playbackRate;
			source.start(placed.begin);
			soundUntil = placed.until;
			told("sounded", placed.follows, placed.stretched);
		},
		startSound() {
			if (context === undefined) {
				context = new AudioContext({ sampleRate: options.sampleRate });
				output = context.createGain();
				if (options.connectOutput === undefined) {
					output.connect(context.destination);
				} else {
					options.connectOutput(output, context);
				}
			}
			void context.resume();
			return context;
		},
		sound: () => context,
		waitingFrames: () => frames.waiting(),
		close() {
			closed = true;
			clearInterval(ticking);
			for (const each of frames.due(Number.POSITIVE_INFINITY, 0)) {
				each.frame.close();
			}
			void context?.close();
		},
	};
}
