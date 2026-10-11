// The watching page of the demo: plays the video track and draws every
// frame as it comes out of the decoder. No buffer: a frame is shown when it
// is decoded.

import { playVideoTrack } from "@xmoq/media";
import {
	connectToRelay,
	DECODER_CODEC,
	namespace,
	say,
	trackName,
} from "./shared.ts";

async function main(): Promise<void> {
	const canvas = document.querySelector("canvas");
	const context = canvas?.getContext("2d");
	if (canvas === null || context === null || context === undefined) {
		throw new Error("no canvas");
	}
	say("connecting");
	const connection = await connectToRelay();
	say("subscribing; the picture begins with the next key frame");
	const began = performance.now();
	let firstFrameAfterMs: number | undefined;
	let lastDelayMs = 0;
	const playback = await playVideoTrack(connection, {
		trackNamespace: namespace(),
		trackName: trackName(),
		codec: DECODER_CODEC,
		// `?join=no` waits for the next key frame, as without the FETCH.
		joiningFetch: new URLSearchParams(location.search).get("join") !== "no",
		onFrame(frame) {
			firstFrameAfterMs ??= Math.round(performance.now() - began);
			if (
				canvas.width !== frame.displayWidth ||
				canvas.height !== frame.displayHeight
			) {
				canvas.width = frame.displayWidth;
				canvas.height = frame.displayHeight;
			}
			context.drawImage(frame, 0, 0);
			// The time of a frame is the publisher's wall clock, in
			// microseconds: how old the picture is when it is drawn, where
			// the two pages have the same clock.
			lastDelayMs = Math.round(Date.now() - frame.timestamp / 1000);
			frame.close();
		},
	});
	setInterval(() => {
		const stats = playback.stats();
		say(
			`decoded ${stats.decoded} frames; first after ${firstFrameAfterMs ?? "?"} ms; the last one ${lastDelayMs} ms after it was made; dropped ${JSON.stringify(stats.dropped)}; decoder failures ${stats.decoderFailures}; joining FETCH ${stats.joiningFetch}`,
		);
		// For a script that looks at the page.
		document.body.dataset.decoded = String(stats.decoded);
	}, 500);
	const end = await playback.ended;
	say(`the playing ended: ${end.type}`);
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
