// The camera and the microphone of a page of the demo, which the bar of
// controls (`controls.ts`) changes while the page sends: another device,
// the camera off and on, the microphone muted. Off and muted, they give
// nothing, and the page has its encoders `pause` (@xmoq/media); the times
// go on from one device to the next with the time between them
// (`createRetimer`).

/** Not in the DOM library of TypeScript yet: a track of a `MediaStream` as a stream of frames (Chromium has it). */
declare class MediaStreamTrackProcessor {
	constructor(init: { track: MediaStreamTrack });
	readonly readable: ReadableStream<VideoFrame | AudioData>;
}

/**
 * The times, in microseconds, of what comes from one source after another.
 * Each source keeps its own spacing; the first time of a source that comes
 * after another is the last of that one and the time between the two by
 * the wall clock (`nowMs`).
 *
 * WHY: two cameras count from times of their own, and so do two
 * microphones. The encoders lay the times onto the wall clock by the first
 * one (`StampClock` of @xmoq/media), so a time that jumped back or ahead
 * where the device changed would be a jump of the broadcast's clock for
 * those who play it. WHY the time between by the wall clock, and not none:
 * a camera that was off for a while is that much later against the sound,
 * whose times went on (`pause` of the audio encoder).
 */
export function createRetimer(): {
	/** The time of what `source` gave at `timeUs`, at `nowMs` of the wall clock. */
	at(source: unknown, timeUs: number, nowMs: number): number;
} {
	let last: unknown;
	let shift = 0;
	let before: { atUs: number; nowMs: number } | undefined;
	return {
		at(source, timeUs, nowMs) {
			if (source !== last) {
				last = source;
				shift =
					before === undefined
						? 0
						: before.atUs + Math.max(1, (nowMs - before.nowMs) * 1000) - timeUs;
			}
			const at = timeUs + shift;
			before = { atUs: at, nowMs };
			return at;
		},
	};
}

/** The microphone, which can be muted and changed while the page sends. */
export interface Microphone {
	/** The id of the device in use. */
	deviceId(): string | undefined;
	/** Uses another device; the one before is let go once it opened. */
	use(deviceId: string): Promise<void>;
	/** Gives its sound (`true`) or nothing. */
	setOn(on: boolean): void;
}

/** What a page asks of a microphone, besides the device. */
const VOICE: MediaTrackConstraints = {
	channelCount: 1,
	// WHY said: Chromium of Playwright 1.64.0 turned the three on without
	// them too (its fake microphone, 2026-10-27), but what a browser does
	// when a page says nothing is the browser's.
	noiseSuppression: true,
	echoCancellation: true,
	autoGainControl: true,
};

/**
 * Opens the microphone (the browser's choice, or `deviceId`) and gives each
 * piece of its sound to `onData`, which the callee closes, with the times
 * going on from one device to the next. Muted, it gives nothing; the track
 * goes on, so its times go on, and the page has the encoder `pause`.
 *
 * WHY nothing and not silence: what is not said is not sent (the person's
 * choice, 2026-10-28). With one piece of sound a Group, no stream is left
 * open meanwhile, and the encoder puts the time of the pause into the times
 * after it, so a player plays them when they are due, after a silence.
 * Silence was sent before (`enabled = false`,
 * docs/experiments/2026-10-27-a-muted-page-sends-silence.md).
 */
export async function openMicrophone(
	sampleRate: number,
	onData: (data: AudioData) => void,
	deviceId?: string,
): Promise<Microphone> {
	const retimer = createRetimer();
	let on = true;
	let track: MediaStreamTrack | undefined;
	const open = async (id: string | undefined): Promise<MediaStreamTrack> => {
		const media = await navigator.mediaDevices.getUserMedia({
			audio: {
				...VOICE,
				sampleRate,
				...(id === undefined ? {} : { deviceId: { exact: id } }),
			},
		});
		const [opened] = media.getAudioTracks();
		if (opened === undefined) throw new Error("the microphone gave no track");
		return opened;
	};
	const read = async (from: MediaStreamTrack): Promise<void> => {
		const reader = new MediaStreamTrackProcessor({
			track: from,
		}).readable.getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			const data = next.value;
			if (!(data instanceof AudioData) || from !== track || !on) {
				data.close();
				continue;
			}
			const timestamp = retimer.at(from, data.timestamp, Date.now());
			if (timestamp === data.timestamp) {
				onData(data);
				continue;
			}
			onData(withTimestamp(data, timestamp));
			data.close();
		}
	};
	const start = (opened: MediaStreamTrack): void => {
		const before = track;
		track = opened;
		before?.stop();
		void read(opened);
	};
	start(await open(deviceId));
	return {
		deviceId: () => track?.getSettings().deviceId,
		async use(id) {
			start(await open(id));
		},
		setOn(now) {
			on = now;
		},
	};
}

/** A copy of `data` with another time. */
function withTimestamp(data: AudioData, timestamp: number): AudioData {
	const planes: Float32Array[] = [];
	for (let plane = 0; plane < data.numberOfChannels; plane++) {
		const samples = new Float32Array(data.numberOfFrames);
		data.copyTo(samples, { planeIndex: plane, format: "f32-planar" });
		planes.push(samples);
	}
	const all = new Float32Array(data.numberOfFrames * data.numberOfChannels);
	planes.forEach((samples, plane) => {
		all.set(samples, plane * data.numberOfFrames);
	});
	return new AudioData({
		format: "f32-planar",
		sampleRate: data.sampleRate,
		numberOfFrames: data.numberOfFrames,
		numberOfChannels: data.numberOfChannels,
		timestamp,
		data: all,
	});
}

/** The camera, which can be turned off and on and changed while the page sends. */
export interface Camera {
	/** Whether it gives frames now. */
	on(): boolean;
	/** The id of the device in use, or last used. */
	deviceId(): string | undefined;
	/** Turns it on (with the device used last) or off. Rejects where it does not open, and stays off. */
	turn(on: boolean): Promise<void>;
	/** Uses another device: at once where it is on, and when it is turned on where it is off. */
	use(deviceId: string): Promise<void>;
}

/**
 * The camera, asked for `width` by `height` at `frameRate`, which gives each
 * of its frames to `onFrame` while it is on, with the track it came from;
 * the callee closes the frame. Off, it gives none and its light is off: the
 * page has the encoder `pause`, which ends the Group under way.
 */
export function createCamera(options: {
	width: number;
	height: number;
	frameRate: number;
	onFrame(frame: VideoFrame, from: MediaStreamTrack): void;
	deviceId?: string;
}): Camera {
	let track: MediaStreamTrack | undefined;
	let chosen = options.deviceId;
	const read = async (from: MediaStreamTrack): Promise<void> => {
		const reader = new MediaStreamTrackProcessor({
			track: from,
		}).readable.getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			const frame = next.value;
			if (!(frame instanceof VideoFrame) || from !== track) {
				frame.close();
				continue;
			}
			options.onFrame(frame, from);
		}
	};
	const open = async (id: string | undefined): Promise<void> => {
		const media = await navigator.mediaDevices.getUserMedia({
			video: {
				width: options.width,
				height: options.height,
				frameRate: options.frameRate,
				...(id === undefined ? {} : { deviceId: { exact: id } }),
			},
		});
		const [opened] = media.getVideoTracks();
		if (opened === undefined) throw new Error("the camera gave no video track");
		const before = track;
		track = opened;
		chosen = opened.getSettings().deviceId;
		before?.stop();
		void read(opened);
	};
	return {
		on: () => track !== undefined,
		deviceId: () => chosen,
		async turn(on) {
			if (!on) {
				track?.stop();
				track = undefined;
				return;
			}
			if (track === undefined) await open(chosen);
		},
		async use(id) {
			chosen = id;
			if (track !== undefined) await open(id);
		},
	};
}
