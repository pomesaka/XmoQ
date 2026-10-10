// The public surface is what an application needs to connect, publish and
// subscribe, with the types those calls take and return. The codec and the
// session in src/core are not exported: nothing outside this package reads
// them yet. The types of `ConnectOptions.diagnostics` are in the entry
// "@xmoq/moqt/diagnostics".

export type { Redirect } from "./core/control-message.ts";
export type { KeyValuePair } from "./core/key-value-pair.ts";
export type { SessionLimits } from "./core/limits.ts";
export type {
	OutgoingObject,
	SubgroupOptions,
} from "./core/outgoing-subgroups.ts";
export type {
	GroupOrder,
	Location,
	MessageParameters,
	SubscriptionFilter,
} from "./core/parameters.ts";
export type { PublishRequest, SubscribeRequest } from "./core/requests.ts";
export type {
	ObjectStatus,
	SubgroupId,
	SubgroupObject,
} from "./core/subgroup.ts";
export type { FullTrackName, TrackNamespace } from "./core/track.ts";
export type { ConnectOptions } from "./webtransport/connect.ts";
export { connect } from "./webtransport/connect.ts";
export type {
	MoqtConnection,
	PublishedTrack,
	ReceivedObject,
	SessionConnectOptions,
	SubgroupWriter,
	Subscription,
} from "./webtransport/connection.ts";
export type {
	PeerStreamEnd,
	PublishEnd,
	RequestAbort,
	SessionClose,
	SubscriptionEnd,
} from "./webtransport/endings.ts";
export {
	PublishEndedError,
	RequestAbortedError,
	RequestFailedError,
	SessionClosedError,
	SubgroupClosedError,
	SubscriptionEndedError,
} from "./webtransport/endings.ts";
