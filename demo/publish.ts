// The publishing page of the demo: a camera, or a picture it paints, is
// encoded and published as one video track.
//
// `?source=camera` takes the camera; without it the page paints a clock.
// The sound is a tone that goes up and down, made by the page;
// `?audio=mic` takes the microphone, `?audio=none` sends no audio track.

import {
	type AudioTrackEncoder,
	createAudioTrackEncoder,
	createVideoTrackEncoder,
} from "@xmoq/media";
import { RequestErrorCode } from "@xmoq/moqt";
import {
	audioTrackName,
	connectToRelay,
	ENCODER_CODEC,
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
} from "./shared.ts";

const FRAMES_A_SECOND = 30;

/** Not in the DOM library of TypeScript yet: a track of a `MediaStream` as a stream of frames (Chromium has it). */
declare class MediaStreamTrackProcessor {
	constructor(init: { track: MediaStreamTrack });
	readonly readable: ReadableStream<VideoFrame | AudioData>;
}

/**
 * A quiet tone whose pitch goes up and down, and a loud beep while the mark
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
	setInterval(() => {
		while (piece * 20 <= Date.now() - began) {
			const loud = marked(began + piece * 20);
			const samples = new Float32Array(frames);
			for (let index = 0; index < frames; index++) {
				const seconds = (piece * frames + index) / SAMPLE_RATE;
				const pitch = loud ? 1000 : 440 + 220 * Math.sin(Math.PI * seconds);
				phase += (2 * Math.PI * pitch) / SAMPLE_RATE;
				samples[index] = (loud ? 0.6 : 0.1) * Math.sin(phase);
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
	}, 10);
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
		next.value.close();
		onFrame(new VideoFrame(preview as HTMLCanvasElement, { timestamp }));
	}
}

/** Paints a clock and a bar that moves, and calls `onFrame` with each picture. */
function fromACanvas(onFrame: (frame: VideoFrame) => void): void {
	const began = performance.now();
	let painted = 0;
	setInterval(() => {
		if (context === null || context === undefined) return;
		const elapsed = performance.now() - began;
		context.fillStyle = "#10202c";
		context.fillRect(0, 0, WIDTH, HEIGHT);
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
	}, 1000 / FRAMES_A_SECOND);
}

async function main(): Promise<void> {
	const encoder = await createVideoTrackEncoder({
		trackNamespace: namespace(),
		trackName: trackName(),
		// MSF §6.1 has the time in milliseconds as one way to begin.
		firstGroup: BigInt(Date.now()),
		codec: ENCODER_CODEC,
		width: WIDTH,
		height: HEIGHT,
		bitrate: 1_500_000,
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
			// `?audioGroup=N` puts N chunks of audio into a Group, on one stream.
			chunksPerGroup: Number(
				new URLSearchParams(location.search).get("audioGroup") ?? 1,
			),
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
	// One session after the other: the encoders go on, and when the session
	// ends the page connects again after two seconds and announces anew.
	const room = joinRoom("publisher");
	void (async () => {
		for (let session = 1; ; session++) {
			try {
				link = "connecting";
				const connection = await connectToRelay(through(HERE));
				connection.serve({
					subscribe(request) {
						if (encoder.publisher.subscribe(request)) return;
						if (audio?.publisher.subscribe(request)) return;
						request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
					},
					fetch(request) {
						if (encoder.publisher.fetch(request)) return;
						if (audio?.publisher.fetch(request)) return;
						request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
					},
				});
				// Where the rooms give tokens, the announcement carries one.
				const token = await room?.token("pub", HERE.namespace);
				await connection.publishNamespace(
					namespace(),
					token === undefined ? undefined : { authorizationToken: [token] },
				);
				// The room is told where this broadcast is, for viewers to
				// find it; that holds as long as this page is connected to
				// the room.
				room?.publish(HERE);
				link = `publishing (session ${session})`;
				const close = await connection.closed;
				link = `the session ended (${close.type}) at ${new Date().toISOString().slice(11, 19)}; connecting again every 2 s`;
			} catch (error) {
				link = `could not publish (${why(error)}); trying again every 2 s`;
			}
			document.body.dataset.link = link;
			await new Promise((resolve) => setTimeout(resolve, 2000));
		}
	})();
	const onFrame = (frame: VideoFrame): void => {
		encoder.encode(frame);
	};
	if (new URLSearchParams(location.search).get("source") === "camera") {
		void fromCamera(onFrame).catch((error: unknown) => {
			say(`the camera failed: ${String(error)}`);
		});
	} else {
		fromACanvas(onFrame);
	}
	setInterval(() => {
		const stats = encoder.stats();
		const sent = encoder.publisher.stats();
		say(
			`${link}\nvideo: encoded ${stats.encoded} frames (${stats.keyChunks} key), ${Math.round(stats.encodedBytes / 1024)} KiB; sent ${sent.written} Objects; not sent ${JSON.stringify(stats.notSent)}; not encoded ${JSON.stringify(stats.notEncoded)} · audio: ${audio === undefined ? "none" : `encoded ${audio.stats().encoded} chunks, sent ${audio.publisher.stats().written}, skipped ${JSON.stringify(audio.publisher.stats().skipped)}`}`,
		);
	}, 500);
	void encoder.failed.then((error) => say(`the encoder failed: ${String(error)}`));
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
