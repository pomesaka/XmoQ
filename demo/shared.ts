// What the two pages of the demo agree on without a catalog.

import { connect, type MoqtConnection } from "@xmoq/moqt";

export const NAMESPACE = ["xmoq", "demo"];
export const TRACK = "video";
export const AUDIO_TRACK = "audio";
export const SAMPLE_RATE = 48_000;

/**
 * The mark both tracks carry at the same moment of the publisher's clock,
 * for a viewer to measure by: for the first 200 ms of every 2 seconds the
 * picture has a white square in its upper right corner, and the sound is a
 * loud beep.
 */
export const marked = (wallClockMs: number): boolean =>
	wallClockMs % 2000 < 200;
/** Where the square is. */
export const MARK = { x: 540, y: 20, size: 80 };
export const WIDTH = 640;
export const HEIGHT = 360;
/** What the encoder is configured with, and what the decoder is told: the same profile and level, with the parameter sets in the key frames (`avc3`). */
export const ENCODER_CODEC = "avc1.42E01F";
export const DECODER_CODEC = "avc3.42E01F";

const utf8 = new TextEncoder();
export const namespace = (): Uint8Array[] =>
	NAMESPACE.map((field) => utf8.encode(field));
export const trackName = (): Uint8Array => utf8.encode(TRACK);
export const audioTrackName = (): Uint8Array => utf8.encode(AUDIO_TRACK);

/** The relay, as `serve.ts` wrote it into the page. */
declare const XMOQ_DEMO: { relayUrl: string; certSha256Hex: string };

/** Connects to the relay of the demo. */
export function connectToRelay(): Promise<MoqtConnection> {
	const hash = Uint8Array.from(
		XMOQ_DEMO.certSha256Hex.match(/../g) ?? [],
		(byte) => Number.parseInt(byte, 16),
	);
	return connect(XMOQ_DEMO.relayUrl, {
		implementation: "xmoq-demo/0.0.0",
		// WHY: the relay's certificate is self-signed. Chromium takes it when
		// its hash is given, and it is valid for less than two weeks.
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
		// What the README of @xmoq/media asks of a viewer.
		limits: { maxObjectPayloadLength: 1024 * 1024 },
	});
}

export const say = (text: string): void => {
	const status = document.getElementById("status");
	if (status !== null) status.textContent = text;
};
