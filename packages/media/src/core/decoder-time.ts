/**
 * The largest time a decoder of WebCodecs is given, in microseconds: the
 * largest whole number a JavaScript number has exactly. WHY a limit: the
 * `timestamp` of `EncodedVideoChunkInit` and `EncodedAudioChunkInit` is a
 * `required [EnforceRange] long long` (WebCodecs, W3C Working Draft of
 * 2026-10-19, §8.1 and §8.2), and Web IDL throws a TypeError for a number
 * beyond 2^53 - 1 there as the chunk is made; a LOC Timestamp goes up to
 * 2^64 - 1. What the senders of this package send stays within it, so that
 * their receivers take it.
 */
export const MAX_DECODER_TIME = BigInt(Number.MAX_SAFE_INTEGER);
