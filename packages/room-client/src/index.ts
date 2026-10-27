// Asking a room (@xmoq/room) over HTTP: where its relay is, tokens for that
// relay, and the seats of a meeting. What the room answers, and how a client
// reads it, is `fixtures/room-api/README.md`.

export {
	broadcastRoom,
	type KeptSeat,
	keepSeat,
	leaveSeat,
	meetingRoom,
	type RoomAddress,
	RoomAnswerError,
	renewSeat,
	takeSeat,
	tokenOfARoom,
} from "./client.ts";
export {
	type BroadcastRoom,
	type MeetingRoom,
	type RelayOfARoom,
	readBroadcastRoom,
	readMeetingRoom,
	readSeatAnswer,
	readToken,
	type Seat,
	type SeatAnswer,
	type SeatRefusal,
} from "./core/answers.ts";
