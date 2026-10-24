// The public surface is what an application needs to connect, publish and
// subscribe, with the types those calls take and return. The codec and the
// session in src/core are not exported: nothing outside this package reads
// them yet. The types of `ConnectOptions.diagnostics` are in the entry
// "@xmoq/moqt/diagnostics".

export type { FetchRange, Redirect } from "./core/control-message.ts";
export { RequestErrorCode } from "./core/error.ts";
export type { FetchObject, RangeEnd } from "./core/fetch.ts";
export type { KeyValuePair } from "./core/key-value-pair.ts";
export { DEFAULT_ANSWER_WAIT_MS, type SessionLimits } from "./core/limits.ts";
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
export type {
	FetchAnswer,
	FetchRequest,
	PublishRequest,
	SubscribeAnswer,
	SubscribeRequest,
} from "./core/requests.ts";
export {
	createSubgroupStreamDecoder,
	type ObjectStatus,
	type ObjectStreamFailure,
	type SubgroupHeader,
	type SubgroupId,
	type SubgroupObject,
	type SubgroupStreamDecoder,
	type SubgroupStreamItem,
	type SubgroupStreamLimits,
	type SubgroupStreamResult,
} from "./core/subgroup.ts";
export type { FullTrackName, TrackNamespace } from "./core/track.ts";
export type { ConnectOptions } from "./webtransport/connect.ts";
export { connect } from "./webtransport/connect.ts";
export type {
	Fetch,
	FetchCallOptions,
	FetchResponse,
	FetchWriter,
	GoAwayNotice,
	MoqtConnection,
	PeerFetch,
	PeerSubscribe,
	PublishedTrack,
	ReceivedObject,
	ServeHandlers,
	SessionConnectOptions,
	SubgroupWriter,
	Subscription,
} from "./webtransport/connection.ts";
export type {
	FetchEnd,
	PeerStreamEnd,
	PublishEnd,
	RequestAbort,
	ServedFetchEnd,
	SessionClose,
	SubscriptionEnd,
} from "./webtransport/endings.ts";
export {
	FetchEndedError,
	FetchStreamResetError,
	PublishEndedError,
	RequestAbortedError,
	RequestFailedError,
	ServedFetchEndedError,
	SessionClosedError,
	StreamNotOpenedError,
	SubgroupClosedError,
	SubscriptionEndedError,
} from "./webtransport/endings.ts";
export type {
	DecodeFetchBodyOptions,
	FetchItem,
} from "./webtransport/fetch-objects.ts";
export {
	decodeFetchBody,
	FetchBodyError,
} from "./webtransport/fetch-objects.ts";
