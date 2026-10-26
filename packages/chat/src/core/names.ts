// The namespaces and tracks of the chat, as `crates/chat-tracks/src/names.rs`
// has them, and `fixtures/chat-tracks/names.tsv` pins for both: a
// participant announces `xmoq/chat/<kind>/<room>/<name>` and publishes the
// track `said` there; the gatherer publishes the track `lines` of
// `xmoq/chat/<kind>/<room>`. The kind is that of the room: a broadcast's or
// a meeting's (decided on 2026-10-26: the two are applications of their own,
// and a broadcast and a meeting of the same name do not share a chat).

/** The track of a participant: what it says. */
export const SAID_TRACK = "said";

/** The track of a room: its lines, numbered. */
export const LINES_TRACK = "lines";

import { type ChatRoom, roomFields } from "./room-names.ts";

export { CHAT_PREFIX, type ChatRoom, roomFields } from "./room-names.ts";

const encoder = new TextEncoder();

/** `xmoq/chat/<kind>/<room>/<name>`: what a participant announces, as the fields of a Track Namespace. */
export function personNamespace(room: ChatRoom, name: string): Uint8Array[] {
	return [...roomFields(room), name].map((field) => encoder.encode(field));
}

/** `xmoq/chat/<kind>/<room>`: the namespace of the track `lines` of a room. */
export function roomNamespace(room: ChatRoom): Uint8Array[] {
	return roomFields(room).map((field) => encoder.encode(field));
}
