// A participant of the chat over MoQ tracks, for a page: it says its lines
// on a track of its own and reads the lines of the room from the gatherer
// (`crates/chat`). The names, payloads and limits of the tracks are those of
// `xmoq-chat-tracks`.
//
// WHY a package and not a part of `demo/`: the reader of the payloads is a
// second implementation of what `xmoq-chat-tracks` reads, and its tests read
// the shared table (`fixtures/chat-tracks/`); a test under `demo/` would not
// run (`mise run check` runs the tests of `packages/` alone). The interop run
// (`interop/chat-pages.ts`) drives the same `joinChat` the demo's pages use,
// which it could not do with code of `demo/`, which nothing may depend on.
// WHY NOT a part of `@xmoq/media`: the chat has nothing of media, and would
// sit in layer 2 for no reason.

export {
	CATCH_UP_LINES,
	CATCH_UP_LINES_MOST,
	catchUpLines,
} from "./core/history.ts";
export {
	type ChatLimits,
	chatLimits,
	type KeptBack,
	keepBack,
	RateWindow,
} from "./core/limits.ts";
export {
	type ChatRoom,
	personNamespace,
	roomFields,
	roomNamespace,
} from "./core/names.ts";
export {
	type Line,
	type Malformed,
	MOST_EMOJI_UNITS,
	type Present,
	type Reactions,
} from "./core/payloads.ts";
export {
	type ChatConnection,
	type KeepInTheRoomOptions,
	keepInTheRoom,
	type RoomState,
} from "./moqt/keep-in-the-room.ts";
export {
	type ChatParticipant,
	type ChatStatus,
	type JoinChatOptions,
	joinChat,
	type ReceivedLine,
	type SayOutcome,
} from "./moqt/participant.ts";
export {
	type ChatSaying,
	type ChatView,
	type ShowChatOptions,
	showChat,
} from "./web/chat-view.ts";
