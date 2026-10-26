// What the publishers and the viewers of the demo agree on, beside the
// catalog: the names of the tracks, the sample rate of the sound and the
// codecs of the video.

export const TRACK = "video";
export const AUDIO_TRACK = "audio";
export const SAMPLE_RATE = 48_000;
/** What the encoder is configured with, and what the decoder is told: the same profile and level, with the parameter sets in the key frames (`avc3`). */
export const ENCODER_CODEC = "avc1.42E01F";
export const DECODER_CODEC = "avc3.42E01F";

const utf8 = new TextEncoder();
export const namespace = (fields: string[]): Uint8Array[] =>
	fields.map((field) => utf8.encode(field));
export const trackName = (): Uint8Array => utf8.encode(TRACK);
export const audioTrackName = (): Uint8Array => utf8.encode(AUDIO_TRACK);
