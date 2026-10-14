// The publishing page of the demo: a camera, or a picture it paints, is
// encoded and published as one video track.
//
// `?source=camera` takes the camera; without it the page paints a clock.
// The sound is a tone that goes up and down, made by the page;
// `?audio=mic` takes the microphone, `?audio=none` sends no audio track.

import {
	type AudioTrackEncoder,
	createAudioTrackEncoder,
	keepPublishing,
	createVideoTrackEncoder,
} from "@xmoq/media";
import { createCatalogPublisher } from "@xmoq/msf";
import {
	AUDIO_TRACK,
	audioTrackName,
	connectToRelay,
	DECODER_CODEC,
	ENCODER_CODEC,
	TRACK,
	HERE,
	MARK,
	marked,
	SAMPLE_RATE,
	HEIGHT,
	namespace,
	joinRoom,
	say,
	through,
	why,
	trackName,
	SIZES,
	WIDTH,
	every,
	perSecond,
} from "./shared.ts";

const FRAMES_A_SECOND = 30;
/** `?size=360|720|1080`: the size the picture is sent at; 360 when left out, or for a size that is none. */
const asked = new URLSearchParams(location.search).get("size");
const SIZE =
	asked === "720" || asked === "1080" ? SIZES[asked] : SIZES["360"];

/** Not in the DOM library of TypeScript yet: a track of a `MediaStream` as a stream of frames (Chromium has it). */
declare class MediaStreamTrackProcessor {
	constructor(init: { track: MediaStreamTrack });
	readonly readable: ReadableStream<VideoFrame | AudioData>;
}

/**
 * A tone whose pitch goes up and down, and a louder beep while the mark
 * is on (`marked`), 20 ms at a time, without an `AudioContext`: that needs
 * a click, and this page is to send from the start.
 *
 * WHY by the clock and not one piece a tick: a timer of a page is late now
 * and then, and the sound would fall behind the picture by every tick that
 * was. Piece `n` is the 20 ms that begin `n * 20` ms after the first, and
 * as many are made as the clock has passed.
 */
/**
 * `?toneSkewPpm=N` (for measuring): the tone makes its pieces N parts in a
 * million faster than this machine's clock passes (slower with a negative
 * N), as a microphone does whose samples are counted by a clock of its own.
 */
const TONE_PACE =
	1 + Number(new URLSearchParams(location.search).get("toneSkewPpm") ?? 0) / 1_000_000;

function tone(onData: (data: AudioData) => void): void {
	const frames = SAMPLE_RATE / 50;
	// WHY the clock begins with the first tick, and not here: the timer is a
	// worker's and takes a while to start (about 10 ms in Chromium, about
	// 100 ms in Firefox). Counted from here, the first tick made all the
	// pieces of that while at once, and the encoder takes the first piece
	// it is given for "now": every piece then carried a time that much
	// ahead of when it was made.
	let began: number | undefined;
	let piece = 0;
	let phase = 0;
	let level = 0;
	every(10, () => {
		began ??= Date.now();
		while (piece * 20 <= (Date.now() - began) * TONE_PACE) {
			const loud = marked(began + piece * 20);
			const samples = new Float32Array(frames);
			for (let index = 0; index < frames; index++) {
				const seconds = (piece * frames + index) / SAMPLE_RATE;
				const pitch = loud ? 1000 : 440 + 220 * Math.sin(Math.PI * seconds);
				phase += (2 * Math.PI * pitch) / SAMPLE_RATE;
				// Loud enough to be heard on the loudspeaker of a laptop. WHY
				// the level moves towards its value and does not jump: a
				// step in the level is a click where the beep begins and
				// ends. It is there within about 2 ms.
				level += ((loud ? 0.6 : 0.25) - level) * 0.02;
				samples[index] = level * Math.sin(phase);
			}
			onData(
				new AudioData({
					format: "f32",
					sampleRate: SAMPLE_RATE,
					numberOfFrames: frames,
					numberOfChannels: 1,
					timestamp: Math.round(piece * 20_000 * CLOCK_SKEW),
					data: samples,
				}),
			);
			piece += 1;
		}
	});
}

/** Calls `onData` with the audio of the microphone. */
async function fromMicrophone(onData: (data: AudioData) => void): Promise<void> {
	const media = await navigator.mediaDevices.getUserMedia({
		audio: { sampleRate: SAMPLE_RATE, channelCount: 1 },
	});
	const [track] = media.getAudioTracks();
	if (track === undefined) throw new Error("the microphone gave no track");
	const reader = new MediaStreamTrackProcessor({ track }).readable.getReader();
	for (;;) {
		const next = await reader.read();
		if (next.done) return;
		if (next.value instanceof AudioData) onData(next.value);
		else next.value.close();
	}
}

const preview = document.querySelector("canvas");
const context = preview?.getContext("2d");
if (preview === null || context === null || context === undefined) {
	throw new Error("no canvas");
}
preview.width = SIZE.width;
preview.height = SIZE.height;
// It is shown at one size whatever it is sent at.
preview.style.width = `${WIDTH}px`;

/** Calls `onFrame` with the frames of the camera, each of which the callee closes. */
/** How long the last 200 frames of the camera took from the camera to here, in milliseconds. */
const cameraToEncoderMs: number[] = [];
/** The size of the frames the camera gives, which need not be the size it was asked for. */
let cameraGave: string | undefined;

async function fromCamera(onFrame: (frame: VideoFrame) => void): Promise<void> {
	const media = await navigator.mediaDevices.getUserMedia({
		video: {
			width: SIZE.width,
			height: SIZE.height,
			frameRate: FRAMES_A_SECOND,
		},
	});
	const [track] = media.getVideoTracks();
	if (track === undefined) throw new Error("the camera gave no video track");
	const reader = new MediaStreamTrackProcessor({ track }).readable.getReader();
	for (;;) {
		const next = await reader.read();
		if (next.done) return;
		if (next.value instanceof AudioData) {
			next.value.close();
			continue;
		}
		// The preview shows what is sent; the encoder is fixed to one size,
		// so the frame is drawn to that size and taken from the canvas.
		cameraGave = `${next.value.displayWidth}x${next.value.displayHeight}`;
		context?.drawImage(next.value, 0, 0, SIZE.width, SIZE.height);
		const { timestamp } = next.value;
		// When the camera took the frame, on this page's clock, where the
		// browser says (Chromium does, `captureTime`): how long the frame
		// took to get to the encoder. The viewer cannot see this part.
		const taken: unknown = Reflect.get(next.value.metadata(), "captureTime");
		if (typeof taken === "number") {
			cameraToEncoderMs.push(performance.now() - taken);
			if (cameraToEncoderMs.length > 200) cameraToEncoderMs.shift();
		}
		next.value.close();
		onFrame(new VideoFrame(preview as HTMLCanvasElement, { timestamp }));
	}
}

/** Paints a clock and a bar that moves, and calls `onFrame` with each picture. */
/** `?picture=busy`: the clock's picture with blocks that change in every frame, for a picture that fills the bitrate it asks for. */
const BUSY = new URLSearchParams(location.search).get("picture") === "busy";
/**
 * `?clockSkewPpm=N` (for measuring): the times the painted clock and the
 * tone carry run faster than this machine's clock by N parts in a million
 * (slower with a negative N), as those of a publisher on another machine
 * do against its viewer's. Not for the camera and the microphone, whose
 * times are the devices'.
 */
/**
 * `?stamp=least` (for measuring): both encoders lay their source's times
 * onto the wall clock by the least of the last minute (`StampClock` of
 * @xmoq/media) and not by the first frame alone.
 */
const STAMP =
	new URLSearchParams(location.search).get("stamp") === "least"
		? ("least" as const)
		: ("first" as const);
/**
 * `?canvasClock=performance` (for measuring): the painted clock is timed
 * by `performance.now()` while the tone is by `Date.now()`, which is a
 * broadcast whose two tracks count by two clocks.
 */
const canvasNow =
	new URLSearchParams(location.search).get("canvasClock") === "performance"
		? () => performance.now()
		: () => Date.now();
const CLOCK_SKEW =
	1 + Number(new URLSearchParams(location.search).get("clockSkewPpm") ?? 0) / 1_000_000;

function fromACanvas(onFrame: (frame: VideoFrame) => void): void {
	// WHY `Date.now()` and not `performance.now()`: the tone counts its
	// pieces by `Date.now()`, and the two clocks move against each other
	// (60 ms in an hour on one machine, 2026-10-13). A viewer has one clock
	// for both tracks, so the picture came later and later against the sound.
	const began = canvasNow();
	let painted = 0;
	every(1000 / FRAMES_A_SECOND, () => {
		if (context === null || context === undefined) return;
		const elapsed = canvasNow() - began;
		// Painted in the units of the smallest size, and scaled to the one
		// that is sent.
		context.setTransform(SIZE.width / WIDTH, 0, 0, SIZE.height / HEIGHT, 0, 0);
		context.fillStyle = "#10202c";
		context.fillRect(0, 0, WIDTH, HEIGHT);
		if (BUSY) {
			// A picture that costs bits: 300 blocks, new in every frame.
			for (let block = 0; block < 300; block++) {
				context.fillStyle = `hsl(${Math.random() * 360} 70% ${20 + Math.random() * 50}%)`;
				context.fillRect(
					Math.random() * WIDTH,
					200 + Math.random() * (HEIGHT - 280),
					24,
					24,
				);
			}
		}
		context.fillStyle = "#f2c14e";
		context.fillRect(((elapsed / 10) % (WIDTH + 80)) - 80, HEIGHT - 60, 80, 30);
		context.fillStyle = marked(Date.now()) ? "#ffffff" : "#000000";
		context.fillRect(MARK.x, MARK.y, MARK.size, MARK.size);
		context.fillStyle = "#ffffff";
		context.font = "48px monospace";
		context.fillText(new Date().toISOString().slice(11, 23), 40, 120);
		context.font = "24px monospace";
		context.fillText(`frame ${painted}`, 40, 170);
		painted += 1;
		onFrame(
			new VideoFrame(preview as HTMLCanvasElement, {
				timestamp: Math.round(elapsed * 1000 * CLOCK_SKEW),
			}),
		);
	});
}

/** `?keyint=`: how long a Group is, in milliseconds; a second when left out. */
const KEY_INTERVAL_MS = Number(
	new URLSearchParams(location.search).get("keyint") ?? 1000,
);
/**
 * `?dynamic=N`: a trial. The video track has dynamic Groups, the shortest N
 * milliseconds long: a viewer that asks for a new Group gets one (transport
 * §10.2.13). Off when left out.
 */
const SHORTEST_GROUP_MS = new URLSearchParams(location.search).get("dynamic");
/** `?bitrate=`: for seeing what a smaller picture does on a bad link; the size's own when left out. */
const BITRATE = Number(
	new URLSearchParams(location.search).get("bitrate") ?? SIZE.bitrate,
);

/**
 * `?ladder=1` (**a trial**): the same picture is encoded a second time,
 * smaller, as a track of its own, for a viewer whose link has no room for
 * the first. The catalog has the two in one alternate group (MSF §5.2.12).
 */
const LADDER = new URLSearchParams(location.search).has("ladder");
const LOW = { name: "video-low", width: 320, height: 180, bitrate: 500_000 };

async function main(): Promise<void> {
	// MSF §6.1 has the time in milliseconds as one way to begin. One number
	// for both versions: their Groups are counted alike.
	const firstGroup = BigInt(Date.now());
	const low = LADDER
		? await createVideoTrackEncoder({
				trackNamespace: namespace(),
				trackName: new TextEncoder().encode(LOW.name),
				firstGroup,
				codec: ENCODER_CODEC,
				width: LOW.width,
				height: LOW.height,
				bitrate: LOW.bitrate,
				framerate: FRAMES_A_SECOND,
				keyFrameIntervalMs: KEY_INTERVAL_MS,
				clock: STAMP,
			})
		: undefined;
	const small = new OffscreenCanvas(LOW.width, LOW.height).getContext("2d");
	/** What `onKeyChunksNotSent` said: key frames that were over what is sent. */
	const keyChunksNotSent: unknown[] = [];
	const encoder = await createVideoTrackEncoder({
		trackNamespace: namespace(),
		trackName: trackName(),
		firstGroup,
		codec: `avc1.42E0${SIZE.level}`,
		width: SIZE.width,
		height: SIZE.height,
		// `?bitrate=`: for seeing what a smaller picture does on a bad link.
		bitrate: BITRATE,
		framerate: FRAMES_A_SECOND,
		keyFrameIntervalMs: KEY_INTERVAL_MS,
		clock: STAMP,
		...(SHORTEST_GROUP_MS === null
			? {}
			: { dynamicGroups: { shortestGroupMs: Number(SHORTEST_GROUP_MS) } }),
		// `?mapping=stream-per-object` puts every Object on a stream of its own.
		...(new URLSearchParams(location.search).get("mapping") ===
		"stream-per-object"
			? { mapping: "stream-per-object" as const }
			: {}),
		onKeyChunksNotSent(event) {
			keyChunksNotSent.push(event);
			say(`key frames are not sent: ${JSON.stringify(event)}`);
		},
	});
	const audioSource = new URLSearchParams(location.search).get("audio");
	let audio: AudioTrackEncoder | undefined;
	if (audioSource !== "none") {
		audio = await createAudioTrackEncoder({
			trackNamespace: namespace(),
			trackName: audioTrackName(),
			firstGroup: BigInt(Date.now()),
			codec: "opus",
			sampleRate: SAMPLE_RATE,
			numberOfChannels: 1,
			clock: STAMP,
			// `?audioPriority=N`: the Publisher Priority of the sound's streams
			// (0 the highest); without it, the default of `@xmoq/media`.
			...(new URLSearchParams(location.search).has("audioPriority")
				? {
						publisherPriority: Number(
							new URLSearchParams(location.search).get("audioPriority"),
						),
					}
				: {}),
			// `?audioGroup=N` puts N chunks of audio into a Group, on one stream;
			// without it, the default of `@xmoq/media` (1).
			...(new URLSearchParams(location.search).has("audioGroup")
				? {
						chunksPerGroup: Number(
							new URLSearchParams(location.search).get("audioGroup"),
						),
					}
				: {}),
		});
		const sound = audio;
		if (audioSource === "mic") {
			void fromMicrophone((data) => sound.encode(data)).catch(
				(error: unknown) => say(`the microphone failed: ${String(error)}`),
			);
		} else {
			tone((data) => sound.encode(data));
		}
	}
	/** What the line under the picture says of the session. */
	let link = "connecting";
	// One session after the other (`keepPublishing` of @xmoq/media): the
	// encoders go on, and when the session ends the page connects again
	// after two seconds and announces anew.
	const room = joinRoom("publisher");
	// The catalog of MSF says which tracks the broadcast has; a viewer finds
	// them there and is told no name. It is made before a media Object is
	// sent (MSF §5), and stays over the sessions, as the encoders do.
	const catalog = createCatalogPublisher({
		namespace: namespace(),
		catalog: {
			version: "draft-01",
			tracks: [
				{
					name: TRACK,
					packaging: "loc",
					isLive: true,
					role: "video",
					...(low === undefined ? {} : { renderGroup: 1, altGroup: 1 }),
					// What a decoder is configured with: the parameter sets are in the key frames.
					codec: `avc3.42E0${SIZE.level}`,
					bitrate: BITRATE,
					width: SIZE.width,
					height: SIZE.height,
					framerate: FRAMES_A_SECOND,
				},
				...(low === undefined
					? []
					: [
							{
								name: LOW.name,
								packaging: "loc",
								isLive: true,
								role: "video",
								renderGroup: 1,
								altGroup: 1,
								codec: DECODER_CODEC,
								bitrate: LOW.bitrate,
								width: LOW.width,
								height: LOW.height,
								framerate: FRAMES_A_SECOND,
							},
						]),
				...(audio === undefined
					? []
					: [
							{
								name: AUDIO_TRACK,
								packaging: "loc",
								isLive: true,
								role: "audio",
								codec: "opus",
								// What Chromium's encoder was seen to make without a setting.
								bitrate: 67_000,
								samplerate: SAMPLE_RATE,
								// The number of channels, as the draft's examples have it.
								channelConfig: "1",
							},
						]),
			],
		},
	});
	void keepPublishing({
		// Where the rooms give tokens, the connection carries one.
		connect: async () =>
			connectToRelay(through(HERE), await room?.token("pub", HERE.namespace)),
		trackNamespace: namespace(),
		tracks: [
			catalog,
			encoder.publisher,
			...(low === undefined ? [] : [low.publisher]),
			...(audio === undefined ? [] : [audio.publisher]),
		],
		onState(state) {
			switch (state.type) {
				case "connecting":
					link = "connecting";
					return;
				case "publishing":
					// The room is told where this broadcast is, for viewers to
					// find it; that holds as long as this page is connected to
					// the room.
					room?.publish(HERE);
					link = `publishing (session ${state.session})`;
					return;
				case "session-ended":
					link = `the session ended (${state.close.type}) at ${new Date().toISOString().slice(11, 19)}; connecting again every 2 s`;
					break;
				case "failed":
					link = `could not publish (${why(state.error)}); trying again every 2 s`;
					break;
			}
			document.body.dataset.link = link;
		},
	});
	const framesIn = perSecond();
	const onFrame = (frame: VideoFrame): void => {
		framesIn.count();
		if (low === undefined || small === null) {
			encoder.encode(frame);
			return;
		}
		// The same frame, smaller, with the same time. A key frame for one
		// is a key frame for both, so that their Groups begin together and
		// a viewer can go from one to the other at a Group's start.
		small.drawImage(frame, 0, 0, LOW.width, LOW.height);
		const smaller = new VideoFrame(small.canvas, {
			timestamp: frame.timestamp,
		});
		const keyFrame = encoder.wantsKeyFrame() || low.wantsKeyFrame();
		encoder.encode(frame, { keyFrame });
		low.encode(smaller, { keyFrame });
	};
	if (new URLSearchParams(location.search).get("source") === "camera") {
		void fromCamera(onFrame).catch((error: unknown) => {
			say(`the camera failed: ${String(error)}`);
		});
	} else {
		fromACanvas(onFrame);
	}
	every(500, () => {
		const stats = encoder.stats();
		const sent = encoder.publisher.stats();
		// For a script that looks at the page.
		document.body.dataset.publish = JSON.stringify({
			size: `${SIZE.width}x${SIZE.height}`,
			bitrate: BITRATE,
			cameraGave: cameraGave ?? null,
			framesIn: framesIn.rate(),
			stats,
			written: sent.written,
			keyChunksNotSent,
		});
		say(
			`${link} · ${framesIn.rate()} frames a second${document.visibilityState === "hidden" ? " (this tab is in the background)" : ""}${
				new URLSearchParams(location.search).get("source") !== "camera"
					? ""
					: cameraToEncoderMs.length === 0
						? " · from the camera to the encoder: the browser does not say"
						: ` · from the camera to the encoder: median ${Math.round([...cameraToEncoderMs].sort((a, b) => a - b)[Math.floor(cameraToEncoderMs.length / 2)] ?? 0)} ms (not in the viewer's numbers)`
			}${low === undefined ? "" : `\nvideo-low (${LOW.width}x${LOW.height}, ${LOW.bitrate / 1000} kbit/s): encoded ${low.stats().encoded} frames (${low.stats().keyChunks} key), ${Math.round(low.stats().encodedBytes / 1024)} KiB; sent ${low.publisher.stats().written} Objects`}\nvideo: encoded ${stats.encoded} frames (${stats.keyChunks} key), ${Math.round(stats.encodedBytes / 1024)} KiB; sent ${sent.written} Objects;${SHORTEST_GROUP_MS === null ? "" : ` new Groups asked for ${sent.newGroupsAskedFor}, begun ${sent.newGroupsBegun};`} not sent ${JSON.stringify(stats.notSent)}; not encoded ${JSON.stringify(stats.notEncoded)} · audio: ${audio === undefined ? "none" : `encoded ${audio.stats().encoded} chunks, sent ${audio.publisher.stats().written}, skipped ${JSON.stringify(audio.publisher.stats().skipped)}`}`,
		);
	});
	void encoder.failed.then((error) => say(`the encoder failed: ${String(error)}`));
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
