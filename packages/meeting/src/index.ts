export { MEET, meetingPrefix, participantOf } from "./core/names.ts";
export {
	createPresence,
	type Presence,
	type PresenceChange,
	SETTLE_MS,
	watchedOf,
} from "./core/presence.ts";
export {
	type ChosenTracks,
	chooseTracks,
	type OfferedTrack,
} from "./core/tracks.ts";
export {
	type AttendingState,
	type KeepAttendingOptions,
	keepAttending,
} from "./web/keep-attending.ts";
export {
	createMeeting,
	DEFAULT_MOST_OTHERS,
	DEFAULT_MOST_WATCHED,
	type Meeting,
	type MeetingEvent,
	type MeetingOptions,
	type ReceivedKind,
} from "./web/meeting.ts";
