// The room of the chat and the fields of its namespace, as text, apart from
// `names.ts`, which makes the bytes: what a room of `@xmoq/room` gives
// tokens in, from here (`@xmoq/chat/room-names`), with no input or output.

/** The fields every namespace of the chat begins with. */
export const CHAT_PREFIX = ["xmoq", "chat"] as const;

/** A room of the chat: that of a broadcast, or that of a meeting, by its name. */
export interface ChatRoom {
	kind: "broadcast" | "meeting";
	name: string;
}

/** The fields of `xmoq/chat/<kind>/<room>`, as text. */
export const roomFields = (room: ChatRoom): string[] => [
	...CHAT_PREFIX,
	room.kind,
	room.name,
];
