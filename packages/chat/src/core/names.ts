// The namespaces and tracks of the chat (decided on 2026-10-24), as
// `crates/chat-tracks/src/names.rs` has them: a participant announces
// `xmoq/chat/<room>/<name>` and publishes the track `said` there; the
// gatherer publishes the track `lines` of `xmoq/chat/<room>`.

/** The fields every namespace of the chat begins with. */
export const CHAT_PREFIX = ["xmoq", "chat"] as const;

/** The track of a participant: what it says. */
export const SAID_TRACK = "said";

/** The track of a room: its lines, numbered. */
export const LINES_TRACK = "lines";

const encoder = new TextEncoder();

/** `xmoq/chat/<room>/<name>`: what a participant announces, as the fields of a Track Namespace. */
export function personNamespace(room: string, name: string): Uint8Array[] {
	return [...CHAT_PREFIX, room, name].map((field) => encoder.encode(field));
}

/** `xmoq/chat/<room>`: the namespace of the track `lines` of a room. */
export function roomNamespace(room: string): Uint8Array[] {
	return [...CHAT_PREFIX, room].map((field) => encoder.encode(field));
}
