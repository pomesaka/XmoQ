// The tracks of an endpoint over a session of @xmoq/moqt, whatever they
// carry: one track kept on one session and asked for again when its
// subscription ends or is refused, a followed track read with what came
// before, and the tracks a connection serves to its peer. What a track
// carries (video, sound, a catalog, lines) is the business of the packages
// above.

export {
	type FollowGap,
	type FollowPlan,
	followPlan,
	followStart,
} from "./core/follow-plan.ts";
export { SeenNumbers } from "./core/seen-numbers.ts";
export {
	FOLLOW_FETCH_TIMEOUT_MS,
	FOLLOW_RENDEZVOUS_MS,
	type FollowedTrack,
	type FollowStatus,
	type FollowTrackOptions,
	followTrack,
	type ItemCame,
} from "./follow.ts";
export {
	type KeepTrackOnSessionOptions,
	type KeptTrackState,
	keepTrackOnSession,
} from "./keep-track.ts";
export {
	DEFAULT_END_TIMEOUT_MS,
	endedByThePeer,
	type PublishingTrack,
	type PublishTrackOptions,
	publishTrack,
	tellEnd,
	wholeNumber,
} from "./publish.ts";
export { type ServedTrack, serveTracks } from "./serve.ts";
export { type StayOptions, stayOnSession } from "./stay-on-session.ts";
