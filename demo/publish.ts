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
	WIDTH,
	every,
	perSecond,
} from "./shared.ts";

const FRAMES_A_SECOND = 30;

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
function tone(onData: (data: AudioData) => void): void {
	const frames = SAMPLE_RATE / 50;
	const began = Date.now();
	let piece = 0;
	let phase = 0;
	let level = 0;
	every(10, () => {
		while (piece * 20 <= Date.now() - began) {
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
					timestamp: piece * 20_000,
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
preview.width = WIDTH;
preview.height = HEIGHT;

/** Calls `onFrame` with the frames of the camera, each of which the callee closes. */
/** How long the last 200 frames of the camera took from the camera to here, in milliseconds. */
const cameraToEncoderMs: number[] = [];

async function fromCamera(onFrame: (frame: VideoFrame) => void): Promise<void> {
	const media = await navigator.mediaDevices.getUserMedia({
		video: { width: WIDTH, height: HEIGHT, frameRate: FRAMES_A_SECOND },
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
		context?.drawImage(next.value, 0, 0, WIDTH, HEIGHT);
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

function fromACanvas(onFrame: (frame: VideoFrame) => void): void {
	const began = performance.now();
	let painted = 0;
	every(1000 / FRAMES_A_SECOND, () => {
		if (context === null || context === undefined) return;
		const elapsed = performance.now() - began;
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
				timestamp: Math.round(elapsed * 1000),
			}),
		);
	});
}

/** `?bitrate=`: for seeing what a smaller picture does on a bad link. */
const BITRATE = Number(
	new URLSearchParams(location.search).get("bitrate") ?? 1_500_000,
);

async function main(): Promise<void> {
	const encoder = await createVideoTrackEncoder({
		trackNamespace: namespace(),
		trackName: trackName(),
		// MSF §6.1 has the time in milliseconds as one way to begin.
		firstGroup: BigInt(Date.now()),
		codec: ENCODER_CODEC,
		width: WIDTH,
		height: HEIGHT,
		// `?bitrate=`: for seeing what a smaller picture does on a bad link.
		bitrate: BITRATE,
		framerate: FRAMES_A_SECOND,
		keyFrameIntervalMs: 1000,
		// `?mapping=stream-per-object` puts every Object on a stream of its own.
		...(new URLSearchParams(location.search).get("mapping") ===
		"stream-per-object"
			? { mapping: "stream-per-object" as const }
			: {}),
		onKeyChunksNotSent(event) {
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
			// `?audioGroup=N` puts N chunks of audio into a Group, on one stream;
			// without it, the default of `@xmoq/media`.
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
					// What a decoder is configured with: the parameter sets are in the key frames.
					codec: DECODER_CODEC,
					bitrate: BITRATE,
					width: WIDTH,
					height: HEIGHT,
					framerate: FRAMES_A_SECOND,
				},
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
		encoder.encode(frame);
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
		say(
			`${link} · ${framesIn.rate()} frames a second${document.visibilityState === "hidden" ? " (this tab is in the background)" : ""}${
				new URLSearchParams(location.search).get("source") !== "camera"
					? ""
					: cameraToEncoderMs.length === 0
						? " · from the camera to the encoder: the browser does not say"
						: ` · from the camera to the encoder: median ${Math.round([...cameraToEncoderMs].sort((a, b) => a - b)[Math.floor(cameraToEncoderMs.length / 2)] ?? 0)} ms (not in the viewer's numbers)`
			}\nvideo: encoded ${stats.encoded} frames (${stats.keyChunks} key), ${Math.round(stats.encodedBytes / 1024)} KiB; sent ${sent.written} Objects; not sent ${JSON.stringify(stats.notSent)}; not encoded ${JSON.stringify(stats.notEncoded)} · audio: ${audio === undefined ? "none" : `encoded ${audio.stats().encoded} chunks, sent ${audio.publisher.stats().written}, skipped ${JSON.stringify(audio.publisher.stats().skipped)}`}`,
		);
	});
	void encoder.failed.then((error) => say(`the encoder failed: ${String(error)}`));
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
