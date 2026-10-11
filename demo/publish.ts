// The publishing page of the demo: a camera, or a picture it paints, is
// encoded and published as one video track.
//
// `?source=camera` takes the camera; without it the page paints a clock.

import { createVideoTrackEncoder } from "@xmoq/media";
import { RequestErrorCode } from "@xmoq/moqt";
import {
	connectToRelay,
	ENCODER_CODEC,
	HEIGHT,
	namespace,
	say,
	trackName,
	WIDTH,
} from "./shared.ts";

const FRAMES_A_SECOND = 30;

/** Not in the DOM library of TypeScript yet: a track of a `MediaStream` as a stream of frames (Chromium has it). */
declare class MediaStreamTrackProcessor {
	constructor(init: { track: MediaStreamTrack });
	readonly readable: ReadableStream<VideoFrame>;
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
	say("connecting");
	const connection = await connectToRelay();
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
		onKeyChunksNotSent(event) {
			say(`key frames are not sent: ${JSON.stringify(event)}`);
		},
	});
	connection.serve({
		subscribe(request) {
			if (encoder.publisher.subscribe(request)) return;
			request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
		},
		fetch(request) {
			if (encoder.publisher.fetch(request)) return;
			request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
		},
	});
	await connection.publishNamespace(namespace());
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
			`encoded ${stats.encoded} frames (${stats.keyChunks} key), ${Math.round(stats.encodedBytes / 1024)} KiB; sent ${sent.written} Objects; not sent ${JSON.stringify(stats.notSent)}; not encoded ${JSON.stringify(stats.notEncoded)}`,
		);
	}, 500);
	void encoder.failed.then((error) => say(`the encoder failed: ${String(error)}`));
	void connection.closed.then((close) => say(`the session ended: ${close.type}`));
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
