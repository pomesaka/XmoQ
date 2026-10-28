// An actor of a room's garden over MoQ tracks, for a page: it says what it
// does to the garden on a track of its own, `acted`, and reads what lies in
// the garden and what happened there from the room's `garden`, which the
// garden publishes, and where the pet is and what it does from the pet's
// `state`. The names and payloads of the tracks are those of
// `xmoq-pet-tracks`; both read the shared tables of `fixtures/pet-tracks/`
// in their tests.
//
// WHY a package and not a part of `demo/`: the reader of the payloads is a
// second implementation of what `xmoq-pet-tracks` reads, and its tests read
// the shared tables; a test under `demo/` would not run (`mise run check`
// runs the tests of `packages/` alone). WHY NOT a part of `@xmoq/chat`: the
// garden is an application of its own, with a namespace of its own
// (`xmoq/pet`), and a page may have one without the other.

export {
	ACTED,
	actorNamespace,
	actorOf,
	GARDEN,
	type GardenTrack,
	PET_NAME,
	PREFIX,
	roomNamespace,
	STATE,
	trackOf,
} from "./core/names.ts";
export {
	type Acted,
	type Action,
	actedPayload,
	BUBBLE_BYTES,
	checkAction,
	type Doing,
	type EventKind,
	type Facing,
	FLOOR,
	type Garden,
	type GardenEvent,
	gardenPayload,
	type Malformed,
	MOOD_MOST,
	MOST_EVENTS,
	MOST_THINGS,
	type Place,
	type Read,
	readActed,
	readGarden,
	readState,
	type State,
	statePayload,
	TALK_BYTES,
	type Thing,
	type ThingKind,
} from "./core/payloads.ts";
export {
	type ActOutcome,
	type FollowedGardenTrack,
	type GardenActor,
	type GardenStatus,
	type JoinGardenOptions,
	joinGarden,
	MAX_GARDEN_PAYLOAD,
	MAX_STATE_PAYLOAD,
} from "./moqt/participant.ts";
