// The namespaces and tracks of the transcripts (decided on 2026-10-25), as
// `crates/transcript-tracks/src/names.rs` has them: the transcriber that
// listens to the broadcasts under a prefix publishes the track `speech` of
// `xmoq/transcript/<the fields of the prefix>`.

/** The fields every namespace of a transcript begins with. */
export const TRANSCRIPT_PREFIX = ["xmoq", "transcript"] as const;

/** The track of a transcript: what was said, a piece a Group. */
export const SPEECH_TRACK = "speech";

const encoder = new TextEncoder();

/**
 * `xmoq/transcript/<the fields of listened>`: the namespace of the
 * transcript of the broadcasts under `listened`, as the fields of a Track
 * Namespace.
 */
export function transcriptNamespace(listened: readonly string[]): Uint8Array[] {
	return [...TRANSCRIPT_PREFIX, ...listened].map((field) =>
		encoder.encode(field),
	);
}
