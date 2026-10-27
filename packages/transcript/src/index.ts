// A reader of the transcripts that the transcriber (`crates/transcriber`)
// publishes: the track `speech` of `xmoq/transcript/<prefix>`, a piece of
// speech a Group, followed across sessions and shown in a page as captions.

export {
	SPEECH_TRACK,
	TRANSCRIPT_PREFIX,
	transcriptNamespace,
} from "./core/names.ts";
export type { Malformed, Speech } from "./core/payloads.ts";
export {
	type FollowTranscriptOptions,
	followTranscript,
	LATEST_PIECES,
	type ReceivedSpeech,
	type TranscriptReader,
	type TranscriptStatus,
} from "./moqt/follow.ts";
export {
	type KeepFollowingTranscriptOptions,
	type KeptTranscriptState,
	keepFollowingTranscript,
	type TranscriptConnection,
} from "./moqt/keep-following.ts";
export { type ShowCaptionsOptions, showCaptions } from "./web/captions.ts";
