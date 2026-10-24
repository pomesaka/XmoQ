import {
	InactiveError,
	MoqtError,
	RequestErrorCode,
	SessionErrorCode,
	StreamResetErrorCode,
} from "../core/error.ts";
import type { FetchObject, RangeEnd } from "../core/fetch.ts";
import type { KeyValuePair } from "../core/key-value-pair.ts";
import {
	answerWaitMs,
	DEFAULT_ANSWER_WAIT_MS,
	type SessionLimits,
} from "../core/limits.ts";
import type {
	OutgoingObject,
	SubgroupOptions,
} from "../core/outgoing-subgroups.ts";
import type {
	GroupOrder,
	Location,
	MessageParameters,
} from "../core/parameters.ts";
import type {
	FetchAnswer,
	FetchRequest,
	PublishRequest,
	SubscribeAnswer,
	SubscribeRequest,
} from "../core/requests.ts";
import { createSession, type Session } from "../core/session.ts";
import type {
	SessionEvent,
	SessionOutput,
	SessionResult,
} from "../core/session-output.ts";
import type { SubgroupObject } from "../core/subgroup.ts";
import type { FullTrackName, TrackNamespace } from "../core/track.ts";
import { closeCode, closeReason } from "./close-info.ts";
import type { Diagnostics } from "./diagnostics.ts";
import {
	type FetchEnd,
	FetchEndedError,
	FetchStreamResetError,
	type PeerStreamEnd,
	type PublishEnd,
	PublishEndedError,
	type RequestAbort,
	RequestAbortedError,
	RequestFailedError,
	type ServedFetchEnd,
	ServedFetchEndedError,
	type SessionClose,
	SessionClosedError,
	StreamNotOpenedError,
	SubgroupClosedError,
	type SubscriptionEnd,
	SubscriptionEndedError,
} from "./endings.ts";
import { createObjectQueue, type ObjectQueue } from "./object-queue.ts";
import { type Backpressure, createStreamIo } from "./stream-io.ts";
import type { MoqtTransport } from "./transport.ts";

/** Provisional defaults; the project has not chosen any of them. */
const DEFAULT_SUBSCRIPTION_HIGH_WATER_MARK = 16;
/**
 * Provisional, and the value of the Rust implementation (`max_uni_streams` in
 * crates/moqt-io): the draft sets no limit, and XmoQ has not decided one.
 */
const DEFAULT_MAX_PEER_UNI_STREAMS = 64;
/**
 * Provisional, like every limit here (docs/open-questions.md, on the
 * receive limits). The same number as the Rust implementation reads at once
 * (`max_request_streams` of `StreamConfig`).
 */
const DEFAULT_MAX_PEER_BIDI_STREAMS = 64;
/**
 * Reads of a fetch stream that `Fetch.body` holds; with the last of them
 * the connection stops reading the stream. Provisional. The Rust reader
 * holds five, four in its queue and one that waits for room. How large a
 * read is is the transport's affair.
 */
const FETCH_BODY_HIGH_WATER_MARK = 4;
/**
 * The largest code an application may reset a stream with.
 *
 * WHY 255 and not the 2^32 - 1 of W3C WebTransport (`[Clamp] unsigned long?
 * streamErrorCode`): Chromium 156.0.8078.4 clamps the code of a
 * `WebTransportError` to 255 (checked in a page on 2026-10-10: 256, 65535 and
 * 2^32 - 1 all gave 255), so a larger one cannot be sent from it. Every Stream
 * Reset Error Code of draft 18 (§3.3.3) is 0x12 or less.
 * FIXME: raise this when browsers take the 32 bits of the specification. The
 * Rust implementation sends any 32-bit code.
 */
const MAX_STREAM_ERROR_CODE = 255;
const DEFAULT_SETUP_TIMEOUT_MS = 10_000;
/** Provisional: the time the peer's SETUP and an answer get. The Rust client has the same (`STREAM_OPEN_WAIT`). */
const DEFAULT_OPEN_STREAM_TIMEOUT_MS = 10_000;
/** The Error Reason of the PUBLISH_DONE that ends a PUBLISH nobody answered; the Rust client sends the same. */
const NO_ANSWER_IN_TIME = "no answer in time";

/** The Status Code INTERNAL_ERROR of PUBLISH_DONE (§10.11): "An implementation specific or generic error occurred." */
const PUBLISH_DONE_INTERNAL_ERROR = 0x0n;

/** Whether the event is the peer's answer to a request of this endpoint. */
function isAnswer(event: SessionEvent): event is Extract<
	SessionEvent,
	{
		type:
			| "publish-namespace-ok"
			| "publish-ok"
			| "subscribe-ok"
			| "request-error"
			| "fetch-ok";
	}
> {
	switch (event.type) {
		case "publish-namespace-ok":
		case "publish-ok":
		case "subscribe-ok":
		case "request-error":
		case "fetch-ok":
			return true;
		default:
			return false;
	}
}
/**
 * Provisional, and the value of the Rust implementation
 * (`first_bytes_timeout` in crates/moqt-io); docs/open-questions.md, on the
 * receive limits.
 */
const DEFAULT_FIRST_BYTES_TIMEOUT_MS = 10_000;
/**
 * §10.11 asks for at least the larger of the two delivery timeouts. Neither
 * is implemented, so this is a fixed value for now.
 */
const DEFAULT_PUBLISH_DONE_TIMEOUT_MS = 5_000;

/**
 * The value the Rust client has (PUBLISH_DONE_ANSWER_WAIT in
 * crates/moqt-client). Provisional.
 *
 * WHY short: a peer that follows only the draft may never end its
 * direction, and every `done()` against such a peer takes this long.
 * WHY longer than the 2 seconds an XmoQ relay waits for the counted
 * streams: the relay's reset at the end of its wait is to be seen.
 */
const DEFAULT_PEER_STREAM_END_TIMEOUT_MS = 3_000;

/** The longest delay `setTimeout` takes (HTML, "timer initialization steps": a signed 32-bit number of milliseconds). */
const MAX_TIMER_DELAY_MS = 2 ** 31 - 1;

/**
 * A wait of the connect options, checked for what a timer takes.
 *
 * WHY refused and not corrected: NaN and a negative value make a timer fire
 * at once, and so does a delay above 2^31 - 1, Infinity among them. A wait
 * that was asked for and does not happen is not noticed.
 */
function timerDelay(name: string, value: number): number {
	if (!Number.isInteger(value) || value < 0 || value > MAX_TIMER_DELAY_MS) {
		throw new RangeError(
			`${name} is a whole number from 0 to ${MAX_TIMER_DELAY_MS}, not ${String(value)}`,
		);
	}
	return value;
}

/**
 * A limit on the streams read at once, checked. NaN and Infinity would
 * switch the limit off, and a fraction is no number of streams.
 */
function streamLimit(name: string, value: number, least: number): number {
	if (!Number.isInteger(value) || value < least) {
		throw new RangeError(
			`${name} is a whole number from ${least}, not ${String(value)}`,
		);
	}
	return value;
}

export interface SessionConnectOptions {
	/** Value of the MOQT_IMPLEMENTATION Setup Option. */
	implementation?: string;
	/**
	 * An AUTHORIZATION TOKEN for the whole connection: the Token structure as
	 * bytes (§10.2.2). **It is put into every request this connection sends
	 * that can carry one**: PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and FETCH,
	 * which are the ones of §10.2.2 ("It MAY appear in a PUBLISH, SUBSCRIBE,
	 * REQUEST_UPDATE, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS,
	 * PUBLISH_NAMESPACE, TRACK_STATUS or FETCH message") that this library
	 * sends. It goes before the tokens a request has of its own
	 * (`parameters.authorizationToken`). Nothing is added when left out.
	 *
	 * **It is in SETUP as well** (the AUTHORIZATION TOKEN Setup Option,
	 * §10.3.1.4: "functionally equivalent to the AUTHORIZATION TOKEN message
	 * parameter ... that the peer can use to authorize MOQT session
	 * establishment"), so a relay can turn a connection down before any
	 * request. Not registered under an alias: the whole token travels each
	 * time. Decided on 2026-10-11; what it carries and who checks it is the
	 * application's.
	 */
	authorizationToken?: Uint8Array;
	/** Limits to use in place of the defaults; the ones left out keep their default. */
	limits?: Partial<SessionLimits>;
	/**
	 * Objects a `Subscription` holds before the connection stops reading the
	 * subgroup streams that feed it.
	 */
	subscriptionHighWaterMark?: number;
	/**
	 * Unidirectional streams of the peer read at once, the peer's control
	 * stream among them; a whole number from 2, or `connect` rejects with a
	 * `RangeError`. Further streams wait in the transport, and a stream holds
	 * its place until it has ended.
	 *
	 * This bounds the streams that feed a subscription nobody reads, not its
	 * Objects: a stream waits once the queue is full, but only after it has
	 * queued every Object of the chunk it read. So the queue holds up to
	 * `subscriptionHighWaterMark` - 1 Objects plus one chunk's worth from
	 * each of `maxPeerUniStreams` - 1 streams. That is one Object a stream
	 * where a chunk carries one, and as many as fit into a chunk where
	 * Objects are small; neither Objects nor bytes are counted.
	 */
	maxPeerUniStreams?: number;
	/**
	 * Bidirectional streams of the peer read at once, which are its requests;
	 * a whole number from 1, or `connect` rejects with a `RangeError`. Further
	 * streams wait in the transport, and a stream holds its place until the
	 * peer has ended its direction.
	 *
	 * This bounds what requests that have not fully arrived can hold: up to
	 * 64 KiB each, for a FETCH or a SUBSCRIBE, which are decoded whole.
	 *
	 * A SUBSCRIBE that was accepted (`MoqtConnection.serve`) keeps its stream
	 * open, and its place, while the subscription lasts. With as many
	 * subscriptions accepted as this number, no further request of the peer
	 * is read until one of them ends.
	 */
	maxPeerBidiStreams?: number;
	/**
	 * How long a stream the peer opens has to say what it is, 10000 when left
	 * out: for a unidirectional stream its type, with its whole Subgroup
	 * Header or FETCH_HEADER where it has one, and for a bidirectional one
	 * its first message, whole. A stream that has not by then is stopped,
	 * and the session stays open. The time runs from when the stream is read,
	 * so not for one that waits for a place among `maxPeerUniStreams` or
	 * `maxPeerBidiStreams`. A page in the background runs its timers late,
	 * so there the time can be longer, never shorter.
	 *
	 * 0 does not switch this off: a stream that has not said what it is in
	 * its first read is then stopped at once. There is no value for "never".
	 */
	firstBytesTimeoutMs?: number;
	/**
	 * How long to wait for the peer's SETUP before closing with
	 * CONTROL_MESSAGE_TIMEOUT. Like the two waits below, a whole number from
	 * 0 to 2147483647, the longest delay a timer takes, or `connect` rejects
	 * with a `RangeError`. With 0, only a SETUP that has arrived already is
	 * in time.
	 */
	setupTimeoutMs?: number;
	/**
	 * How long the answer to a request is waited for, 10000 when left out
	 * (**provisional**); a SUBSCRIBE with a RENDEZVOUS_TIMEOUT and a FETCH
	 * with a FILL_TIMEOUT are waited for that much longer (`answerWaitMs`).
	 * When it is over the request alone is cancelled, as `cancel()` does it
	 * (its stream is reset with CANCELLED), and fails with a
	 * `RequestAbortedError` of `no-answer-in-time`; **the session stays
	 * open**. A whole number as for `setupTimeoutMs`.
	 *
	 * WHY the request and not the session, which §3.5 has a code for
	 * (CONTROL_MESSAGE_TIMEOUT, "the peer took too long to respond to a
	 * control message"): a relay answers a SUBSCRIBE when the publisher
	 * behind it has, so one slow publisher would end every other
	 * subscription of the session. docs/moqt-implementation-notes.md has the
	 * row, and the Rust client does the same.
	 */
	answerTimeoutMs?: number;
	/**
	 * How long a request may wait for its stream to be opened, 10000 when
	 * left out (**provisional**): a stream is opened when the peer allows
	 * one more. Then the request fails with a `RequestAbortedError` of
	 * `not-sent-in-time`; nothing was sent, and **the session stays open**.
	 * A whole number as for `setupTimeoutMs`.
	 *
	 * The draft has no time for it (§13.5: "Endpoints MAY impose a MAX
	 * STREAM count limit which would restrict the number of concurrent
	 * streams which an application could have in flight"). The Rust client
	 * and the terminal viewer give a request the same time
	 * (docs/moqt-implementation-notes.md).
	 */
	openStreamTimeoutMs?: number;
	/**
	 * How long a subscription waits for late streams after PUBLISH_DONE when
	 * its Stream Count is not reached (§10.11). With 0 it does not wait: the
	 * subscription ends with PUBLISH_DONE, and the Objects still on their
	 * way are lost.
	 */
	publishDoneTimeoutMs?: number;
	/**
	 * How long `PublishedTrack.done()` waits for the subscriber to end its
	 * direction of the request stream, 3000 when left out.
	 *
	 * The peer has the same time to end its direction after a FETCH that
	 * was served and after a request of it that was refused, by the
	 * application or by the session. When it is over, that direction is
	 * stopped (CANCELLED) and the session stays open.
	 *
	 * With 0, `done()` does not wait: it resolves with `fin` for a subscriber
	 * that had finished before, and with `nothing-in-time` otherwise, after
	 * it has stopped the subscriber's direction as at the end of any wait. A
	 * session closed right after it can lose what was sent last.
	 */
	peerStreamEndTimeoutMs?: number;
	/** Hooks for logging, typed in the entry "@xmoq/moqt/diagnostics". */
	diagnostics?: Diagnostics;
}

export interface ReceivedObject {
	groupId: bigint;
	subgroupId: bigint;
	publisherPriority: number | undefined;
	object: SubgroupObject;
}

/** A subgroup stream this endpoint writes to. */
export interface SubgroupWriter {
	/**
	 * Resolves when the transport accepted the Object, which is where
	 * backpressure shows. Rejects with `PublishEndedError` when the
	 * subscription is over, with `SessionClosedError` when the session is,
	 * and with `SubgroupClosedError` when only this stream is: it was closed
	 * or reset, or the subscriber stopped it, which the error says with
	 * `metStop` where this write met the stop. The stream is reset then
	 * (§11.4.3) and counts as ended, so PUBLISH_DONE is not kept back. Another
	 * failure of the write is passed on as the transport gave it. In each
	 * case `reset()` is safe to call.
	 *
	 * **Wait for a write before the next.** A write is handed to the
	 * transport when it is called, and one that the transport has not taken
	 * is held. With 16 writes of this stream under way, a further one rejects
	 * with a `RangeError` and writes nothing; the stream is as it was.
	 */
	write(object: OutgoingObject): Promise<void>;
	/**
	 * Every Object of the subgroup was written: close with a FIN (§11.4.3).
	 * Does nothing on a stream that is already closed.
	 */
	close(): Promise<void>;
	/**
	 * The subgroup is incomplete: reset the stream (§11.4.3). Does nothing on
	 * a stream that is already closed. `code` is a Stream Reset Error Code
	 * (§3.3.3), CANCELLED when left out; a value that is not a whole number
	 * from 0 to 255 rejects with a `RangeError` and leaves the stream as it is.
	 */
	reset(code?: number): Promise<void>;
}

/**
 * A subscription this endpoint is the publisher of: one it started with
 * PUBLISH, or one the peer asked for with a SUBSCRIBE that was accepted
 * (`MoqtConnection.serve`).
 */
export interface PublishedTrack {
	/** The Request ID of the PUBLISH, or of the peer's SUBSCRIBE. */
	readonly requestId: bigint;
	readonly trackAlias: bigint;
	/** What the subscriber asked for: the parameters of its PUBLISH_OK, or of its SUBSCRIBE. */
	readonly parameters: MessageParameters;
	/** Resolves once the subscription has ended, with how. Never rejects. */
	readonly ended: Promise<PublishEnd>;
	/**
	 * Names what is called when the subscriber asks for a new Group with a
	 * REQUEST_UPDATE (§10.2.13): with the largest Group ID it knows plus 1,
	 * or 0 where it knows none. The request is answered with REQUEST_OK
	 * before the call, whatever the handler does; whether and when a new
	 * Group begins is the publisher's ("The Original Publisher MAY delay
	 * the NEW_GROUP_REQUEST"). A request that came before a handler was
	 * named is handed over when one is, the last of them only. A second
	 * call replaces the handler. What the handler throws is reported with
	 * `reportError` and changes nothing.
	 *
	 * A subscriber "MUST NOT send this parameter ... if the Track did not
	 * include the DYNAMIC_GROUPS Property with value 1": a publisher that
	 * wants these sets that Track Property (0x30) on its PUBLISH or
	 * SUBSCRIBE_OK. The NEW_GROUP_REQUEST of a SUBSCRIBE or a PUBLISH_OK is
	 * in `parameters`.
	 */
	onNewGroupRequest(handler: (group: bigint) => void): void;
	/** Rejects with `PublishEndedError` once the subscription is over. */
	openSubgroup(options: SubgroupOptions): Promise<SubgroupWriter>;
	/**
	 * Sends PUBLISH_DONE and closes the request stream (§10.11), then waits
	 * for the subscriber to end its direction of that stream, and resolves
	 * with what it saw. The wait is at most `peerStreamEndTimeoutMs` (3
	 * seconds) from the moment PUBLISH_DONE and the FIN were written; a
	 * subscriber that does not read can hold up that write, and `done()` with
	 * it. `ended` resolves when PUBLISH_DONE was sent and does not wait.
	 *
	 * Every subgroup stream has to be closed or reset first; otherwise this
	 * rejects with an `Error` and nothing is sent. Rejects with
	 * `PublishEndedError` once the subscription is over, which a second call
	 * finds: only the first one waits and resolves.
	 *
	 * WHY it waits: a transport can lose a stream that arrived and was not
	 * accepted when the session closes, so a session closed right after the
	 * last Object can lose it. An XmoQ relay finishes its direction once it
	 * has the streams PUBLISH_DONE counted. Close the session after this
	 * promise has settled. Calling `close()` without waiting for it can end
	 * the session before PUBLISH_DONE itself has gone out.
	 */
	done(statusCode: bigint, errorReason: string): Promise<PeerStreamEnd>;
}

/** A subscription this endpoint started with SUBSCRIBE. */
export interface Subscription {
	readonly requestId: bigint;
	readonly trackAlias: bigint;
	/** Parameters of the SUBSCRIBE_OK. */
	readonly parameters: MessageParameters;
	readonly trackProperties: KeyValuePair[];
	/**
	 * Objects in the order their streams delivered them. Objects of different
	 * subgroups are not ordered against each other.
	 *
	 * Every Object that arrived before the subscription ended is read first,
	 * also when the subscription was cut off: what the publisher sent before
	 * that is good data, and its last Object must not be the one that is lost.
	 * After the last Object the stream closes if the subscription was ended on
	 * purpose, and fails with a `SubscriptionEndedError` if it was cut off, so
	 * that a reader of `objects` alone cannot take a broken subscription for a
	 * finished one. Where this endpoint ends the subscription itself, the
	 * Objects not read yet are dropped: the reader chose to stop.
	 *
	 * | `ended` resolves with | `objects` |
	 * |---|---|
	 * | `publish-done` | delivers what is left, then closes |
	 * | `peer-closed` | delivers what is left, then fails |
	 * | `peer-reset` | delivers what is left, then fails |
	 * | `malformed-track` | delivers what is left, then fails |
	 * | `session-closed`, closed by the peer or by an error | delivers what is left, then fails |
	 * | `cancelled` | closes at once; unread Objects are dropped |
	 * | `session-closed`, closed by this endpoint | closes at once; unread Objects are dropped |
	 *
	 * A `publish-done` whose `statusCode` reports an error still closes the
	 * stream: the publisher finished the subscription, and `ended` says why.
	 *
	 * `ended` resolves when the subscription ends and does not wait for
	 * `objects` to be read, so it can resolve while Objects are still queued.
	 * For a malformed Track, the Objects delivered are the ones decoded before
	 * the fault was found.
	 */
	readonly objects: ReadableStream<ReceivedObject>;
	/** Resolves once the subscription has ended, with how. Never rejects. */
	readonly ended: Promise<SubscriptionEnd>;
	/** Cancels the SUBSCRIBE request (§3.3.2). Does nothing once the subscription or the session has ended. */
	cancel(): Promise<void>;
	/**
	 * Asks the publisher to change parameters of the subscription with a
	 * REQUEST_UPDATE (§10.9); a parameter that is left out stays as it
	 * was. Resolves with the parameters of the REQUEST_OK.
	 *
	 * Rejects with `RequestFailedError` when the publisher refuses: the
	 * subscription then ends, with the publisher's PUBLISH_DONE
	 * (UPDATE_FAILED, §10.9.1). Rejects with `SubscriptionEndedError` when
	 * the subscription ends before the answer, with a `RangeError` for a
	 * parameter a REQUEST_UPDATE may not carry or when too many updates
	 * wait for their answer, and with an `Error` when no answer comes
	 * within `answerTimeoutMs`: the subscription goes on then, and what
	 * the publisher made of the update is not known.
	 */
	update(parameters: MessageParameters): Promise<MessageParameters>;
}

/** What a FETCH_OK says (§10.13). */
export interface FetchResponse {
	/** Whether `endLocation` is the final Object of the track. */
	readonly endOfTrack: boolean;
	/** The end of the range the response covers: the last Object plus 1, or Object 0 for a whole Group. */
	readonly endLocation: Location;
	readonly parameters: MessageParameters;
	readonly trackProperties: KeyValuePair[];
}

/**
 * A FETCH this endpoint sent (§10.12). Provisional: the shape can change
 * while nothing but tests uses it.
 *
 * The answer and the Objects come on two streams, in either order (§10.13),
 * so they are two things here. The FETCH is over when it was rejected or
 * cancelled, or when both `response` has resolved and `body` has ended. A
 * publisher that does not answer has it given up after `answerTimeoutMs`
 * (`ended` then has `no-answer-in-time`); one that answers and sends no
 * stream, or never ends it, leaves it open, where it takes up one of the
 * requests the session may have: give the call a `signal` or call `cancel()`.
 */
export interface Fetch {
	readonly requestId: bigint;
	/**
	 * The Group Order the FETCH asked for, Ascending where it named none
	 * (§10.2.8). The fetch stream counts its Group IDs up or down by it and
	 * does not carry it, so whoever decodes `body` needs it from here.
	 */
	readonly groupOrder: GroupOrder;
	/**
	 * Resolves with the FETCH_OK. Rejects with `RequestFailedError` for a
	 * REQUEST_ERROR, with `RequestAbortedError` when the FETCH ended without
	 * an answer, and with `SessionClosedError`. A rejection nobody handles is
	 * not reported as unhandled.
	 */
	readonly response: Promise<FetchResponse>;
	/**
	 * The fetch stream after its FETCH_HEADER, as it came: Objects and End of
	 * Range indicators in the encoding of §11.4.4, not decoded, cut wherever
	 * a read ended. `decodeFetchBody` makes Objects of it.
	 *
	 * The connection reads the stream only as fast as this is read: it holds
	 * a few reads and then leaves the rest with the transport, where flow
	 * control holds the publisher back. A body nobody reads keeps one of the
	 * unidirectional streams read at once (`maxPeerUniStreams`).
	 *
	 * | What happened | `body` |
	 * |---|---|
	 * | The publisher finished the fetch stream | delivers what is left, then closes |
	 * | The publisher reset the fetch stream | delivers what is left, then fails with `FetchStreamResetError` |
	 * | The FETCH was rejected, or ended by the publisher or with the session, before the stream ended | delivers what is left, then fails with `FetchEndedError`, or `SessionClosedError` |
	 * | `cancel()`, the `signal`, or this endpoint closed the session | fails at once with `FetchEndedError` or `SessionClosedError`; what was not read is dropped |
	 * | A reader cancels `body` | the FETCH is cancelled; that reader sees no error |
	 *
	 * So a `body` that closes has every byte the publisher sent on the
	 * stream, and one that fails says by the class and `end` of its error
	 * whether the publisher, the session or this endpoint ended it.
	 */
	readonly body: ReadableStream<Uint8Array>;
	/** Resolves once the FETCH is over, with how. Never rejects. */
	readonly ended: Promise<FetchEnd>;
	/**
	 * Cancels the FETCH (§5.2): the request stream is reset and stopped, and
	 * the fetch stream is stopped at once, which frees its place among the
	 * streams read. Does nothing once the FETCH or the session has ended.
	 */
	cancel(): Promise<void>;
}

export interface FetchCallOptions {
	/**
	 * Cancels the FETCH when it aborts, like `cancel()`. With a signal that
	 * has aborted already nothing is sent, and `fetch` rejects with its
	 * reason.
	 */
	signal?: AbortSignal;
}

/**
 * A SUBSCRIBE of the peer, to be answered once with `accept` or `reject`.
 *
 * Nothing limits how long the answer takes. A relay gives up on a
 * publisher that does not answer (an XmoQ relay after 5 seconds) and
 * cancels, which `signal` reports.
 */
export interface PeerSubscribe extends FullTrackName {
	readonly requestId: bigint;
	/** Parameters of the SUBSCRIBE: its filter, FORWARD, and the rest of §10.7. */
	readonly parameters: MessageParameters;
	/**
	 * Aborts when the request went before it was answered. Its reason is a
	 * `RequestAbortedError` when the peer cancelled, or sent a REQUEST_UPDATE,
	 * and a `SessionClosedError` when the session ended. Not aborted by an
	 * answer.
	 *
	 * The `code` of a cancellation is there when the browser gave one. A
	 * relay cancels by resetting its direction of the stream and stopping
	 * this endpoint's; Chromium reports the first with its code and the
	 * second without one, and whichever is seen first ends the request. The
	 * same holds for the `code` of `PublishedTrack.ended` after `accept`.
	 */
	readonly signal: AbortSignal;
	/**
	 * Sends SUBSCRIBE_OK (§10.8) and resolves with the subscription, which is
	 * used like one made with PUBLISH. The Track Alias is chosen here:
	 * counted up from 0, the first number after the one given last that no
	 * subscription this endpoint publishes uses. The alias of a subscription
	 * that ended is not given again at once. An application that also calls
	 * `publish()` picks its own aliases clear of these.
	 *
	 * **A publisher that has published Objects of the Track has to give
	 * `parameters.largestObject`**, the largest Location it has published
	 * (§10.2.11: "If Objects have been published on this Track the Publisher
	 * MUST include this parameter"; left out, it says that none were). The
	 * library keeps no Object and cannot tell, so this is the application's
	 * to do. A relay works out the range of a viewer's Joining FETCH from
	 * it, and refuses that FETCH with `INVALID_RANGE` where it is missing
	 * and the relay has seen no Object itself. A value smaller than what was
	 * published leaves a gap between what such a FETCH gives a viewer and
	 * what its subscription gives it, for as long as the relay has not seen
	 * an Object beyond it.
	 *
	 * Rejects with the reason of `signal` for a request that is gone, with
	 * an `Error` for one that was answered, and with a `RangeError` for an
	 * answer that cannot be sent: a parameter that is none of SUBSCRIBE_OK,
	 * or a value that does not fit its field. After a `RangeError` nothing
	 * was sent, and the request still waits for `accept` or `reject`.
	 */
	accept(answer?: SubscribeAnswer): Promise<PublishedTrack>;
	/**
	 * Sends REQUEST_ERROR with a code of §15.10.2 (`RequestErrorCode` has
	 * the ones this library sends itself) and ends the request.
	 *
	 * Rejects like `accept`: with a `RangeError`, and the request still
	 * waiting, for a code that is no whole number from 0 to 2^62 - 1 and for
	 * a reason of more than 1024 bytes of UTF-8.
	 */
	reject(errorCode: bigint, errorReason?: string): Promise<void>;
}

/**
 * A Standalone FETCH of the peer, to be answered once with `accept` or
 * `reject`: a relay that has no cache asks the publisher for a range of a
 * Track. A relay turns a viewer's Joining FETCH into one of these.
 *
 * A relay waits a limited time for the answer (an XmoQ relay 5 seconds), and
 * then cancels, which `signal` reports.
 *
 * What the draft asks of the publisher of a FETCH, and the library leaves
 * to the application, which is the one that knows what was published:
 *
 * - §10.12.3: "If no Objects have been published for the track or Start
 *   Location is greater than the Largest Object the publisher MUST return
 *   REQUEST_ERROR with error code INVALID_RANGE" (`0x11`): `reject`.
 * - §10.13: the End Location of the answer "is the End Location from the
 *   FETCH request unless the requested range extends beyond published
 *   data", and then {Largest.Group, Largest.Object + 1}.
 * - §10.12.3: "A publisher MUST send fetched groups in the requested group
 *   order", and no Object beyond the Largest one.
 */
export interface PeerFetch extends FullTrackName {
	readonly requestId: bigint;
	/** The first Object that is asked for. */
	readonly startLocation: Location;
	/**
	 * Where the range ends, as the FETCH has it: **the last Object plus 1, and
	 * Object 0 for a whole Group**. {2, 0} asks for everything up to the end
	 * of Group 2; {1, 3} for everything up to Object 2 of Group 1.
	 */
	readonly endLocation: Location;
	/** The order the Groups are to be sent in; Ascending where the FETCH does not say. */
	readonly groupOrder: GroupOrder;
	/**
	 * FILL_TIMEOUT of the FETCH, in milliseconds, where it has one (§10.2.5):
	 * how long a relay may take to get Objects it does not have from
	 * upstream. A publisher that is the origin of the Track has nothing to
	 * wait for: for what it does not have it writes `endOfRange`. The
	 * library runs no timer on it.
	 */
	readonly fillTimeout: bigint | undefined;
	readonly parameters: MessageParameters;
	/** Aborts when the FETCH went before it was answered, as for `PeerSubscribe.signal`. */
	readonly signal: AbortSignal;
	/**
	 * Sends FETCH_OK (§10.13) and opens the fetch stream at once, and
	 * resolves with what writes it. `endLocation` is in the form of
	 * `PeerFetch.endLocation`: the end of what the stream will cover, which
	 * is what was asked for, or less where the Track ends before.
	 *
	 * Rejects like `PeerSubscribe.accept`; the `RangeError` that leaves the
	 * FETCH waiting is for an `endLocation` before `startLocation`. One
	 * beyond what was asked for is sent as it is: to stay within what was
	 * asked for is the application's to keep.
	 */
	accept(answer: FetchAnswer): Promise<FetchWriter>;
	/** Sends REQUEST_ERROR and ends the request, like `PeerSubscribe.reject`. */
	reject(errorCode: bigint, errorReason?: string): Promise<void>;
}

/**
 * The fetch stream of a FETCH this endpoint accepted. The library keeps no
 * Object: what is written goes to the transport.
 *
 * A relay gives a fetch stream up when nothing comes on it for some time
 * (an XmoQ relay after FILL_TIMEOUT, 5 seconds without one), so write or
 * close without long pauses.
 *
 * The calls reject with `ServedFetchEndedError` once the FETCH is over, the
 * end of the session among the ways it ends; and with `SessionClosedError`
 * for a write that was under way when the session ended.
 *
 * A handler of `ServeHandlers.fetch` may let these rejections through: one
 * that says the subscriber gave the FETCH up, or that the session ended, is
 * no error of the application and is not reported. One that says the
 * application itself had closed or reset the writer is.
 */
export interface FetchWriter {
	readonly requestId: bigint;
	/** Resolves once the FETCH is over, with how. Never rejects. */
	readonly ended: Promise<ServedFetchEnd>;
	/**
	 * Writes the next Object. Resolves when the transport took it, which is
	 * where a subscriber that reads slowly holds the writer back. **Wait for
	 * a write before the next**: with 16 writes and indicators of this
	 * stream under way, a further one rejects with a `RangeError` and writes
	 * nothing.
	 *
	 * Rejects with a `RangeError`, and writes nothing, for an Object that
	 * does not come after the one before it: a Group that goes against
	 * `PeerFetch.groupOrder`, or an Object ID that does not grow within a
	 * Group. The writer is as it was, and the next Object can be written.
	 * **Whether an Object lies in the range that was asked for is not
	 * checked**: that is the application's to keep.
	 */
	write(object: FetchObject): Promise<void>;
	/**
	 * Writes an End of Range indicator (§11.4.4.2): the Objects after what
	 * was written before, up to and with `location`, do not exist
	 * ("NonExistent") or are not known to this endpoint ("Unknown"). Rejects
	 * with a `RangeError`, like `write`, for a Location that goes back, and
	 * for one more than 16 under way.
	 */
	endOfRange(kind: RangeEnd, location: Location): Promise<void>;
	/**
	 * Everything was written: finishes the fetch stream, and the FETCH is
	 * over; `ended` resolves with `served`. Does nothing on a FETCH that is
	 * over.
	 *
	 * **This endpoint's direction of the request stream is not finished at
	 * once**: the subscriber has `peerStreamEndTimeoutMs` (3 seconds) to end
	 * its own first, which a subscriber does when it has read the fetch
	 * stream, and this endpoint finishes its direction when it does, or when
	 * that time is over. After that the subscriber has the same time again
	 * before its direction is stopped, as after `PublishedTrack.done()`.
	 * Nothing of that is reported here.
	 */
	close(): Promise<void>;
	/**
	 * Gives the FETCH up: resets both of its streams (§5.2). `code` is a
	 * Stream Reset Error Code (§3.3.3), CANCELLED when left out; a value that
	 * is not a whole number from 0 to 255 rejects with a `RangeError`. Does
	 * nothing on a FETCH that is over.
	 */
	reset(code?: number): Promise<void>;
}

/** What an application does with the requests the peer opens. */
export interface ServeHandlers {
	/**
	 * Called for each SUBSCRIBE of the peer, which a relay sends to the
	 * publisher of a namespace. The request may be answered later, also
	 * after the handler has returned. Without this, a SUBSCRIBE is refused
	 * with `NOT_SUPPORTED`. A handler that throws, or returns a promise (or
	 * another thenable) that rejects, refuses the request with
	 * `INTERNAL_ERROR` unless it was answered, and the error is reported with
	 * `reportError`. One exception: a handler that lets through what
	 * `accept` or `reject` rejected with because the request had gone, or
	 * what a call on the `PublishedTrack` or the `FetchWriter` it accepted
	 * rejected with because the peer ended it or the session is over, is not
	 * reported, and refuses nothing. A `SubgroupClosedError` is reported: the
	 * subscriber stopped one stream, the Track goes on, and what to do about
	 * that stream is the handler's to decide.
	 *
	 * Not called for a Track this endpoint already publishes to the peer
	 * (`DUPLICATE_SUBSCRIPTION`, §5.1), nor while `limits.maxPeerRequests`
	 * requests of the peer are open (`EXCESSIVE_LOAD`).
	 */
	// WHY `void` and not `void | Promise<void>`: with `void` alone, a
	// function that returns something else is still taken, an `async` one
	// among them, and what it returns is looked at when it is called.
	subscribe?(request: PeerSubscribe): void;
	/**
	 * Called for each Standalone FETCH of the peer, in the way of
	 * `subscribe`. Without this, a FETCH is refused with `NOT_SUPPORTED`.
	 *
	 * Not called for a FETCH whose range ends before it starts
	 * (`INVALID_RANGE`), for a Joining FETCH (a relay sends a publisher
	 * none), nor while `limits.maxPeerFetches` FETCHes of the peer are open
	 * (`EXCESSIVE_LOAD`).
	 */
	fetch?(request: PeerFetch): void;
}

/** What a GOAWAY of the peer said (§10.4). */
export interface GoAwayNotice {
	/** Where to continue; empty where the current URI is reused. */
	newSessionUri: string;
	/** Milliseconds the peer waits before it closes the session; 0 for no particular time. */
	timeoutMs: number;
}

export interface MoqtConnection {
	/**
	 * Says how the requests the peer opens are answered, from the next one
	 * on; a later call replaces the handlers. Until it is called, every
	 * request of the peer is refused with `NOT_SUPPORTED`, so call it before
	 * `publishNamespace()`.
	 */
	serve(handlers: ServeHandlers): void;
	/** Setup Options of the peer's SETUP. */
	readonly peerSetupOptions: readonly KeyValuePair[];
	/** Resolves once the session has ended, with how. Never rejects. */
	readonly closed: Promise<SessionClose>;
	/**
	 * Resolves when the peer sends GOAWAY (§10.4), and never where it does
	 * not. From then on the requests below reject with an `Error` and send
	 * nothing ("SHOULD NOT initiate new requests to the peer"); what is open
	 * goes on. Where to go, and when to close this session, is the caller's:
	 * §3.6 has the client connect anew, to `newSessionUri` where it is not
	 * empty, and close this session with NO_ERROR once nothing it subscribed
	 * to is left.
	 */
	readonly goAway: Promise<GoAwayNotice>;
	/**
	 * The three requests reject with `RequestFailedError`,
	 * `RequestAbortedError` or `SessionClosedError`.
	 *
	 * `publish()` rejects with a `RangeError`, and sends nothing, for a Track
	 * Alias that a subscription this endpoint publishes uses (§11.1), and for
	 * a Track it already publishes to the peer (§5.1 allows one subscription
	 * to a Track in each role): one it sent PUBLISH for before, answered or
	 * not, and one the peer asked for with a SUBSCRIBE that waits for its
	 * answer or was accepted.
	 */
	publishNamespace(
		trackNamespace: TrackNamespace,
		parameters?: MessageParameters,
	): Promise<void>;
	publish(request: PublishRequest): Promise<PublishedTrack>;
	subscribe(request: SubscribeRequest): Promise<Subscription>;
	/**
	 * Sends a FETCH (§10.12) and resolves once it is with the transport,
	 * before any answer. Rejects with a `RangeError`, and sends nothing, for
	 * a parameter that is none of a FETCH, for a Standalone range that ends
	 * before it starts, and for a Joining Request ID that is not a SUBSCRIBE
	 * of this connection that is pending or established; and with
	 * `SessionClosedError`.
	 */
	fetch(request: FetchRequest, options?: FetchCallOptions): Promise<Fetch>;
	/**
	 * Closes the session (§3.5). `code` is a Session Termination Error Code,
	 * NO_ERROR when left out; a value that is not a whole number from 0 to
	 * 2^32 - 1 throws a `RangeError` and leaves the session open. The draft
	 * has these codes as integers of up to 62 bits; WebTransport carries 32.
	 * `reason` is sent up to 1024 bytes of UTF-8, cut between two characters,
	 * and `closed` reports the code and the reason as they were sent. Where
	 * the session ends for an error instead, `closed` has that error, and
	 * the peer gets its message cut in the same way.
	 */
	close(code?: number, reason?: string): void;
}

interface Deferred<T> {
	promise: Promise<T>;
	resolve(value: T): void;
	reject(error: Error): void;
}

function deferred<T>(): Deferred<T> {
	let resolve: (value: T) => void = () => {};
	let reject: (error: Error) => void = () => {};
	const promise = new Promise<T>((onResolve, onReject) => {
		resolve = onResolve;
		reject = onReject;
	});
	return { promise, resolve, reject };
}

type EventOf<Type extends SessionEvent["type"]> = Extract<
	SessionEvent,
	{ type: Type }
>;

/** What the connection keeps for an Established SUBSCRIBE. */
interface ActiveSubscription {
	objects: ObjectQueue<ReceivedObject>;
	ended: Deferred<SubscriptionEnd>;
	/** PUBLISH_DONE, once it arrived. */
	publishDone: EventOf<"publish-done"> | undefined;
	publishDoneTimer: ReturnType<typeof setTimeout> | undefined;
	/** The `update()` calls that wait for their answer, in the order sent. */
	updates: Deferred<MessageParameters>[];
}

/** What the connection keeps for a FETCH, from the moment it is sent. */
interface ActiveFetch {
	body: ObjectQueue<Uint8Array>;
	/** Called when the FETCH is written; set where it is registered. */
	sent: () => void;
	response: Deferred<FetchResponse>;
	ended: Deferred<FetchEnd>;
	/** How the publisher ended the fetch stream, once it has. */
	streamEnd: EventOf<"fetch-stream-ended"> | undefined;
}

/**
 * A wait for the peer to end its direction of a request stream on which
 * this endpoint has finished its own: after `done()`, which hands on how it
 * ended, after a FETCH that was served, and after a request of the peer
 * that was refused.
 */
interface PeerStreamWait {
	ended: Deferred<PeerStreamEnd>;
	timer: ReturnType<typeof setTimeout> | undefined;
}

/** What the connection keeps for a FETCH of the peer, from its FETCH_OK on. */
interface ActiveServedFetch {
	ended: Deferred<ServedFetchEnd>;
	/** Set once the FETCH is over. */
	end: ServedFetchEnd | undefined;
}

/** What the connection keeps for a PUBLISH, from its PUBLISH_OK on. */
interface ActivePublish {
	ended: Deferred<PublishEnd>;
	/** Set once the subscription has ended. */
	end: PublishEnd | undefined;
	/** The session took a PUBLISH_DONE for it; `end` follows when it is written. */
	doneTaken: boolean;
	/** What `PublishedTrack.onNewGroupRequest` named. */
	onNewGroup: ((group: bigint) => void) | undefined;
	/** The last NEW_GROUP_REQUEST that came while no handler was named. */
	newGroupAsked: bigint | undefined;
}

/**
 * Writes of one stream that may be under way at once: made, and not yet
 * taken by the transport. One more is refused.
 *
 * WHY: a write is encoded and handed to the transport when it is called,
 * and the transport holds what it cannot send. An application that does not
 * wait for its writes, to a peer that does not read, would have every
 * Object it made held here, without an end. WHY 16: the number of Objects a
 * subscription holds for a reader that does not read
 * (`subscriptionHighWaterMark`). Provisional.
 */
const MAX_WRITES_UNDER_WAY = 16;

/**
 * Runs the writes of one stream, and refuses one more than
 * `MAX_WRITES_UNDER_WAY` under way. `over` says what a write is rejected
 * with once what the stream belongs to has ended.
 */
function boundedWrites(
	over: () => Error | undefined,
): <T>(write: () => Promise<T>) => Promise<T> {
	let underWay = 0;
	return async (write) => {
		// WHY before the count: writes that were under way when the peer or
		// the session ended it can stay so, and a further write is then
		// rejected for that end, which is nobody's mistake, and not for the
		// count, which would be the application's.
		const ended = over();
		if (ended !== undefined) throw ended;
		// Before anything is done: the refusal leaves the stream as it was.
		if (underWay >= MAX_WRITES_UNDER_WAY) {
			throw new RangeError(
				`${MAX_WRITES_UNDER_WAY} writes of the stream are under way: wait for a write before the next`,
			);
		}
		underWay += 1;
		try {
			return await write();
		} finally {
			underWay -= 1;
		}
	};
}

/**
 * Whether a handler failed with what a call on a request, or on what the
 * request became, rejects with because the peer ended it or the session is
 * over. That is how such a request ends, and no error of the application.
 */
function endedByThePeer(error: unknown): boolean {
	if (error instanceof SessionClosedError) return true;
	if (error instanceof ServedFetchEndedError) {
		return (
			error.end.type === "peer-cancelled" || error.end.type === "session-closed"
		);
	}
	if (error instanceof PublishEndedError) {
		return error.end.type !== "done";
	}
	return false;
}

function isThenable(value: unknown): value is PromiseLike<unknown> {
	return (
		typeof value === "object" &&
		value !== null &&
		"then" in value &&
		typeof value.then === "function"
	);
}

function toError(value: unknown): Error {
	return value instanceof Error ? value : new Error(String(value));
}

/**
 * What becomes of `objects` for an ending. The table is in the doc of
 * `Subscription.objects`.
 */
function objectsOutcome(end: SubscriptionEnd): "close" | "discard" | "fail" {
	switch (end.type) {
		case "publish-done":
			return "close";
		case "cancelled":
			return "discard";
		case "session-closed":
			return end.close.type === "local" ? "discard" : "fail";
		case "peer-closed":
		case "peer-reset":
		case "malformed-track":
			return "fail";
	}
}

/**
 * Runs a MOQT session over an established transport session. This module
 * pairs requests with their responses and gives the application its objects;
 * protocol decisions are made in `Session`, and streams are handled by
 * `StreamIo`.
 *
 * Resolves once the peer's SETUP arrived. On any failure before that the
 * transport session is closed.
 */
export async function connectOver(
	transport: MoqtTransport,
	options: SessionConnectOptions = {},
): Promise<MoqtConnection> {
	// WHY from 2: one place is the peer's control stream for as long as the
	// session lasts, so with 1 no data stream would ever be read, and with 0
	// not even SETUP.
	const maxPeerUniStreams = streamLimit(
		"maxPeerUniStreams",
		options.maxPeerUniStreams ?? DEFAULT_MAX_PEER_UNI_STREAMS,
		2,
	);
	// WHY from 1: with 0 no request of the peer would ever be read, and so
	// never answered.
	const maxPeerBidiStreams = streamLimit(
		"maxPeerBidiStreams",
		options.maxPeerBidiStreams ?? DEFAULT_MAX_PEER_BIDI_STREAMS,
		1,
	);
	const peerStreamEndTimeoutMs = timerDelay(
		"peerStreamEndTimeoutMs",
		options.peerStreamEndTimeoutMs ?? DEFAULT_PEER_STREAM_END_TIMEOUT_MS,
	);
	const publishDoneTimeoutMs = timerDelay(
		"publishDoneTimeoutMs",
		options.publishDoneTimeoutMs ?? DEFAULT_PUBLISH_DONE_TIMEOUT_MS,
	);
	const firstBytesTimeoutMs = timerDelay(
		"firstBytesTimeoutMs",
		options.firstBytesTimeoutMs ?? DEFAULT_FIRST_BYTES_TIMEOUT_MS,
	);
	const setupTimeoutMs = timerDelay(
		"setupTimeoutMs",
		options.setupTimeoutMs ?? DEFAULT_SETUP_TIMEOUT_MS,
	);
	const answerTimeoutMs = timerDelay(
		"answerTimeoutMs",
		options.answerTimeoutMs ?? DEFAULT_ANSWER_WAIT_MS,
	);
	const openStreamTimeoutMs = timerDelay(
		"openStreamTimeoutMs",
		options.openStreamTimeoutMs ?? DEFAULT_OPEN_STREAM_TIMEOUT_MS,
	);
	const session: Session = createSession({
		implementation: options.implementation,
		authorizationToken: options.authorizationToken,
		limits: options.limits,
	});
	/** The requests of this endpoint whose answer the session has read, until their response is given. */
	const answered = new Set<bigint>();
	const pendingNamespaces = new Map<bigint, Deferred<void>>();
	const pendingPublishes = new Map<bigint, Deferred<PublishedTrack>>();
	const pendingSubscribes = new Map<bigint, Deferred<Subscription>>();
	/** Track Alias of each PUBLISH that is waiting for its response. */
	const pendingPublishAliases = new Map<bigint, bigint>();
	const subscriptions = new Map<bigint, ActiveSubscription>();
	const publishes = new Map<bigint, ActivePublish>();
	const fetches = new Map<bigint, ActiveFetch>();
	const peerStreamWaits = new Map<bigint, PeerStreamWait>();
	/** Requests of the peer the application has not answered; each is aborted when its request goes. */
	const pendingPeerRequests = new Map<bigint, AbortController>();
	/** FETCHes of the peer that are being answered. */
	const servedFetches = new Map<bigint, ActiveServedFetch>();
	let serveHandlers: ServeHandlers = {};
	/** Serializes outputs so that writes reach each stream in the order the session produced them. */
	let queue: Promise<unknown> = Promise.resolve();
	let sessionClose: SessionClose | undefined;
	const closed = deferred<SessionClose>();
	const peerSetup = deferred<KeyValuePair[]>();
	const goAway = deferred<GoAwayNotice>();
	// The session can end before anything awaits this promise; the failure is
	// reported by the `await` below when it is reached, and by `closed`.
	peerSetup.promise.catch(() => {});

	const io = createStreamIo({
		transport,
		maxPeerUniStreams,
		maxPeerBidiStreams,
		firstBytesTimeoutMs,
		openBidiTimeoutMs: openStreamTimeoutMs,
		trace: (direction, name, data) =>
			options.diagnostics?.onBytes?.(direction, name, data),
		input: (action) => applyInput(action),
		fatal: (error) => fatal(error),
	});

	const endSubscription = (requestId: bigint, end: SubscriptionEnd): void => {
		const subscription = subscriptions.get(requestId);
		if (subscription === undefined) return;
		subscriptions.delete(requestId);
		clearTimeout(subscription.publishDoneTimer);
		for (const update of subscription.updates.splice(0)) {
			update.reject(new SubscriptionEndedError(end));
		}
		switch (objectsOutcome(end)) {
			case "close":
				subscription.objects.end();
				break;
			case "discard":
				subscription.objects.discard();
				break;
			case "fail":
				subscription.objects.end(new SubscriptionEndedError(end));
				break;
		}
		subscription.ended.resolve(end);
	};

	/** Settles a `done()` that waits for the subscriber to end its direction of the request stream. */
	const endPeerStreamWait = (requestId: bigint, end: PeerStreamEnd): void => {
		const wait = peerStreamWaits.get(requestId);
		if (wait === undefined) return;
		peerStreamWaits.delete(requestId);
		clearTimeout(wait.timer);
		wait.ended.resolve(end);
	};

	/**
	 * Starts the wait for the subscriber to end its direction of the request
	 * stream. When it is over, that direction is stopped and the session
	 * forgets the stream.
	 */
	const startPeerStreamTimer = (
		requestId: bigint,
		wait: PeerStreamWait,
	): void => {
		wait.timer = setTimeout(() => {
			endPeerStreamWait(requestId, { type: "nothing-in-time" });
			void applyInput((s) => s.expirePeerEnd(requestId));
		}, peerStreamEndTimeoutMs);
	};

	/**
	 * A FETCH of the peer was served and its fetch stream finished: the
	 * subscriber gets the time of `startPeerStreamTimer` to end its direction
	 * of the request stream first. When that is over this endpoint finishes
	 * its own, and the wait of `startPeerStreamTimer` begins.
	 */
	const startServedFetchWait = (
		requestId: bigint,
		wait: PeerStreamWait,
	): void => {
		wait.timer = setTimeout(() => {
			void applyInput((s) => s.finishServedFetchSend(requestId));
			if (peerStreamWaits.get(requestId) === wait) {
				startPeerStreamTimer(requestId, wait);
			}
		}, peerStreamEndTimeoutMs);
	};

	/**
	 * A request of the peer was refused, by the application or by the
	 * session: REQUEST_ERROR and a FIN. Nobody asks how the peer ends its
	 * direction of the stream, but the stream is not left open for it.
	 *
	 * WHY: the request is kept until its stream is gone, for what the peer
	 * sent before it saw the refusal, and holds one of
	 * `limits.maxPeerRequests` places and one of the peer's bidirectional
	 * streams meanwhile. A peer that never ends its direction would hold
	 * them for good.
	 *
	 * WHY the wait of `done()`, and no stop at once: §3.3.2 has a request
	 * rejected with "a REQUEST_ERROR and FIN", and stopping a direction is
	 * how it has a request cancelled. A peer that ends its direction when it
	 * has read the answer, which both own implementations do, is never
	 * stopped.
	 */
	const awaitPeerEndAfterRefusal = (requestId: bigint): void => {
		const wait: PeerStreamWait = { ended: deferred(), timer: undefined };
		peerStreamWaits.set(requestId, wait);
		startPeerStreamTimer(requestId, wait);
	};

	/** A request of the peer went before the application answered it. */
	const endPendingPeerRequest = (requestId: bigint, reason: Error): boolean => {
		const gone = pendingPeerRequests.get(requestId);
		if (gone === undefined) return false;
		pendingPeerRequests.delete(requestId);
		gone.abort(reason);
		return true;
	};

	const endServedFetch = (requestId: bigint, end: ServedFetchEnd): void => {
		const fetch = servedFetches.get(requestId);
		if (fetch === undefined) return;
		servedFetches.delete(requestId);
		fetch.end = end;
		fetch.ended.resolve(end);
	};

	const endPublish = (requestId: bigint, end: PublishEnd): void => {
		const publish = publishes.get(requestId);
		if (publish === undefined) return;
		publishes.delete(requestId);
		publish.end = end;
		publish.ended.resolve(end);
	};

	/**
	 * Ends a FETCH for whoever holds it. `response` is rejected where it had
	 * not resolved; `body` ends by the table in the doc of `Fetch.body`.
	 */
	const endFetch = (requestId: bigint, end: FetchEnd): void => {
		const fetch = fetches.get(requestId);
		if (fetch === undefined) return;
		fetches.delete(requestId);
		const aborted = (abort: RequestAbort): void => {
			fetch.response.reject(new RequestAbortedError(abort));
			fetch.body.end(new FetchEndedError(end));
		};
		switch (end.type) {
			case "served":
			case "stream-reset":
				// Both have come: the answer, and the end of the stream.
				break;
			case "rejected":
				fetch.response.reject(end.error);
				fetch.body.end(new FetchEndedError(end));
				break;
			case "peer-closed":
				aborted({ type: "peer-closed" });
				break;
			case "peer-reset":
				aborted({ type: "peer-reset", code: end.code });
				break;
			case "unsupported-track-property":
				aborted({ type: "unsupported-track-property" });
				break;
			case "cancelled":
			case "no-answer-in-time":
			case "not-sent-in-time":
				fetch.response.reject(new RequestAbortedError({ type: end.type }));
				fetch.body.abort(new FetchEndedError(end));
				break;
			case "session-closed": {
				const error = new SessionClosedError(end.close);
				fetch.response.reject(error);
				if (end.close.type === "local") fetch.body.abort(error);
				else fetch.body.end(error);
				break;
			}
			default: {
				// A kind of ending that is not handled above does not compile.
				const unhandled: never = end;
				throw new Error(`unknown end of a FETCH: ${String(unhandled)}`);
			}
		}
		fetch.ended.resolve(end);
	};

	const rejectPending = (requestId: bigint, error: Error): void => {
		pendingNamespaces.get(requestId)?.reject(error);
		pendingPublishes.get(requestId)?.reject(error);
		pendingSubscribes.get(requestId)?.reject(error);
		pendingNamespaces.delete(requestId);
		pendingPublishes.delete(requestId);
		pendingSubscribes.delete(requestId);
		pendingPublishAliases.delete(requestId);
	};

	/** Fails everything that waits on the session. The first cause wins. */
	function teardown(close: SessionClose): void {
		if (sessionClose !== undefined) return;
		sessionClose = close;
		const error = new SessionClosedError(close);
		session.transportClosed();
		io.clear();
		peerSetup.reject(error);
		for (const requestId of [
			...pendingNamespaces.keys(),
			...pendingPublishes.keys(),
			...pendingSubscribes.keys(),
		]) {
			rejectPending(requestId, error);
		}
		for (const requestId of [...subscriptions.keys()]) {
			endSubscription(requestId, { type: "session-closed", close });
		}
		for (const requestId of [...publishes.keys()]) {
			endPublish(requestId, { type: "session-closed", close });
		}
		for (const requestId of [...fetches.keys()]) {
			endFetch(requestId, { type: "session-closed", close });
		}
		for (const requestId of [...peerStreamWaits.keys()]) {
			endPeerStreamWait(requestId, { type: "session-closed", close });
		}
		for (const requestId of [...pendingPeerRequests.keys()]) {
			endPendingPeerRequest(requestId, error);
		}
		for (const requestId of [...servedFetches.keys()]) {
			endServedFetch(requestId, { type: "session-closed", close });
		}
		closed.resolve(close);
	}

	/** Ends the session because this endpoint cannot go on. */
	function fatal(
		error: Error,
		code: number = SessionErrorCode.INTERNAL_ERROR,
	): void {
		if (sessionClose !== undefined) return;
		// WHY the error is reported whole when its message is sent cut: it is
		// the cause this endpoint failed with, with its class and its stack,
		// and `closed` hands that on. The peer gets the message as a reason.
		teardown({ type: "error", error });
		transport.close({ closeCode: code, reason: closeReason(error.message) });
	}

	const subscriptionFrom = (event: EventOf<"subscribe-ok">): Subscription => {
		const { requestId } = event;
		const cancelRequest = (): Promise<void> => {
			endSubscription(requestId, { type: "cancelled" });
			// The session agrees: cancelling after the end does nothing.
			if (sessionClose !== undefined) return Promise.resolve();
			return applyVoid((s) => s.cancelRequest(requestId));
		};
		const active: ActiveSubscription = {
			objects: createObjectQueue<ReceivedObject>(
				options.subscriptionHighWaterMark ??
					DEFAULT_SUBSCRIPTION_HIGH_WATER_MARK,
				cancelRequest,
			),
			ended: deferred(),
			publishDone: undefined,
			publishDoneTimer: undefined,
			updates: [],
		};
		subscriptions.set(requestId, active);
		const update = async (
			parameters: MessageParameters,
		): Promise<MessageParameters> => {
			if (subscriptions.get(requestId) !== active) {
				throw new SubscriptionEndedError(await active.ended.promise);
			}
			const answer = deferred<MessageParameters>();
			// Before the write: the answer finds it whenever it comes.
			active.updates.push(answer);
			const unanswered = setTimeout(() => {
				const at = active.updates.indexOf(answer);
				if (at < 0) return;
				// WHY the place is kept, with an answer nobody waits for:
				// answers carry no Request ID and are told apart by their
				// order alone.
				active.updates[at] = deferred();
				active.updates[at]?.promise.catch(() => {});
				answer.reject(new Error(NO_ANSWER_IN_TIME));
			}, answerTimeoutMs);
			try {
				await applyVoid((s) => s.updateSubscription(requestId, parameters));
				return await answer.promise;
			} catch (error) {
				// Not sent: no answer will come for it.
				const at = active.updates.indexOf(answer);
				if (at >= 0) active.updates.splice(at, 1);
				throw error;
			} finally {
				clearTimeout(unanswered);
			}
		};
		return {
			requestId,
			trackAlias: event.trackAlias,
			parameters: event.parameters,
			trackProperties: event.trackProperties,
			objects: active.objects.readable,
			ended: active.ended.promise,
			cancel: cancelRequest,
			update,
		};
	};

	/**
	 * Runs a call on a published Track. Once the Track has ended the call
	 * fails with `PublishEndedError`, and a stream that is closed while the
	 * Track goes on fails with `SubgroupClosedError`. A write the transport
	 * rejected is given one of these, or `SessionClosedError`, where the
	 * transport says which it was, so that the caller can tell by the class.
	 */
	const onPublish = async <T>(
		publish: ActivePublish,
		call: () => Promise<T>,
		/** Whether this endpoint has reset the stream the call is on. */
		resetHere: () => boolean = () => false,
	): Promise<T> => {
		if (publish.end !== undefined) throw new PublishEndedError(publish.end);
		try {
			return await call();
		} catch (error) {
			if (transport.isSessionError(error) && sessionClose === undefined) {
				// The session is ending, and `closed` of the transport settles
				// with this error, at most 0.1 ms apart and in either order
				// (Chromium 156, 1080 samples). One turn of the timers is given
				// for it, so that the error can say how the session ended.
				// WHY not until it settles: nothing promises that it does, and a
				// write would then never be answered.
				await Promise.race([
					closed.promise,
					new Promise((resolve) => setTimeout(resolve, 0)),
				]);
				throw new SessionClosedError(
					sessionClose ?? { type: "error", error: toError(error) },
				);
			}
			// WHY before the error is looked at for a stop: a failure to open a
			// stream is a NetworkError in Chromium as well, and ends the
			// session here. It must not be read as a stream the peer stopped.
			if (sessionClose !== undefined) {
				throw new SessionClosedError(sessionClose);
			}
			// WHY before the stop as well: `reset()` aborts the writer with a
			// stream error that carries this endpoint's own code, and Chromium
			// rejects a write that was under way with that very error (seen in
			// 40 of 40, spikes/wt-write-abort/out/own-reset-20.txt). It looks
			// like a stop of the peer and is none.
			const stopped =
				!resetHere() &&
				(transport.isStreamError(error) || transport.isStopWithoutCode(error));
			// Either form: the peer's stop can have come first and be what the
			// write failed with, when `reset()` was called before this ran.
			const closedHere =
				resetHere() &&
				(transport.isStreamError(error) || transport.isStopWithoutCode(error));
			if (!stopped && !closedHere && !(error instanceof InactiveError)) {
				throw error;
			}
			// The session had already ended the subscription or the stream, or
			// is told so by the stop that failed this write, and the event that
			// says so may still be in the queue. Once the queue is through,
			// `publish.end` tells whether the subscription went with it.
			await queue;
			if (publish.end !== undefined) throw new PublishEndedError(publish.end);
			if (stopped) {
				throw new SubgroupClosedError(
					"the subscriber stopped the stream (STOP_SENDING)",
					{ code: transport.streamErrorCode(error) },
				);
			}
			throw new SubgroupClosedError(
				closedHere
					? "the stream was reset by this endpoint"
					: toError(error).message,
			);
		}
	};

	/** `close` and `reset` of a subgroup stream: done already counts as done. */
	const onPublishIfOpen = async (
		publish: ActivePublish,
		call: () => Promise<void>,
	): Promise<void> => {
		if (publish.end !== undefined) return;
		try {
			await call();
		} catch (error) {
			if (!(error instanceof InactiveError)) throw error;
		}
	};

	const subgroupWriter = (
		publish: ActivePublish,
		stream: number,
	): SubgroupWriter => {
		/** `reset()` was called: a write that fails from then on was not stopped by the peer. */
		let resetHere = false;
		const bounded = boundedWrites(() =>
			publish.end === undefined
				? undefined
				: new PublishEndedError(publish.end),
		);
		return {
			write: (object) =>
				bounded(() =>
					onPublish(
						publish,
						() => applyVoid((s) => s.writeObject(stream, object)),
						() => resetHere,
					),
				),
			close: () =>
				onPublishIfOpen(publish, () =>
					applyVoid((s) => s.closeSubgroup(stream)),
				),
			reset: (code = StreamResetErrorCode.CANCELLED) => {
				// Before anything is done to the stream: a code the browser cannot
				// send must not leave the stream half reset.
				if (
					!Number.isInteger(code) ||
					code < 0 ||
					code > MAX_STREAM_ERROR_CODE
				) {
					return Promise.reject(
						new RangeError(
							`a stream is reset with a whole number from 0 to ${MAX_STREAM_ERROR_CODE}, not ${String(code)}`,
						),
					);
				}
				resetHere = true;
				return onPublishIfOpen(publish, () =>
					applyVoid((s) => s.resetSubgroup(stream, code)),
				);
			},
		};
	};

	/** Hands a NEW_GROUP_REQUEST to the application, or keeps it for a handler to come. */
	const newGroupAsked = (publish: ActivePublish, group: bigint): void => {
		if (publish.onNewGroup === undefined) {
			publish.newGroupAsked = group;
			return;
		}
		try {
			publish.onNewGroup(group);
		} catch (error) {
			reportError(error);
		}
	};

	/** `subscription` is the Request ID of the subscription and what its subscriber asked for. */
	const publishedTrackFrom = (
		subscription: { requestId: bigint; parameters: MessageParameters },
		trackAlias: bigint,
	): PublishedTrack => {
		const { requestId } = subscription;
		const publish: ActivePublish = {
			ended: deferred(),
			end: undefined,
			doneTaken: false,
			onNewGroup: undefined,
			newGroupAsked: undefined,
		};
		publishes.set(requestId, publish);
		return {
			requestId,
			trackAlias,
			parameters: subscription.parameters,
			ended: publish.ended.promise,
			onNewGroupRequest(handler) {
				publish.onNewGroup = handler;
				const asked = publish.newGroupAsked;
				publish.newGroupAsked = undefined;
				if (asked !== undefined) newGroupAsked(publish, asked);
			},
			openSubgroup: (subgroupOptions) =>
				onPublish(publish, async () => {
					let opening: number | undefined;
					const stream = await apply((s) => {
						const result = s.openSubgroup(requestId, subgroupOptions);
						opening = result.value;
						return result;
					}).catch(async (error: unknown) => {
						// The peer allowed no stream for it: the Subgroup is not
						// sent. The session forgets the stream, which was never
						// one, so that PUBLISH_DONE can be sent and counts right.
						if (
							error instanceof StreamNotOpenedError &&
							opening !== undefined
						) {
							const never = opening;
							await applyVoid((s) => s.subgroupNotOpened(never)).catch(
								() => {},
							);
						}
						throw error;
					});
					return subgroupWriter(publish, stream);
				}),
			done: (statusCode, errorReason) =>
				onPublish(publish, async () => {
					// WHY not left to the session: it refuses a second PUBLISH_DONE
					// as a request it no longer has, which reads as a closed
					// subgroup until `end` is set.
					if (publish.doneTaken) throw new PublishEndedError({ type: "done" });
					const wait: PeerStreamWait = { ended: deferred(), timer: undefined };
					try {
						await applyVoid((s) => {
							const outputs = s.publishDone(requestId, statusCode, errorReason);
							// WHY here, and only once the session took the call: a
							// second `done()` is refused by the session and must not
							// replace the wait of the first. The event for a
							// subscriber that had finished its direction is among
							// these outputs, so the wait is there before it is
							// handled.
							peerStreamWaits.set(requestId, wait);
							publish.doneTaken = true;
							return outputs;
						});
					} catch (error) {
						// PUBLISH_DONE could not be written: nobody waits.
						if (peerStreamWaits.get(requestId) === wait) {
							peerStreamWaits.delete(requestId);
						}
						throw error;
					}
					endPublish(requestId, { type: "done" });
					// The subscriber may have ended its direction meanwhile.
					if (peerStreamWaits.get(requestId) === wait) {
						startPeerStreamTimer(requestId, wait);
					}
					return wait.ended.promise;
				}),
		};
	};

	/** Ends a FETCH of this endpoint for whoever holds it, and has the session forget it. */
	const giveUpFetch = (
		requestId: bigint,
		type: "cancelled" | "no-answer-in-time" | "not-sent-in-time",
	): Promise<void> => {
		if (!fetches.has(requestId)) return Promise.resolve();
		endFetch(requestId, { type });
		// The session agrees: cancelling after the end does nothing.
		if (sessionClose !== undefined) return Promise.resolve();
		return applyVoid((s) => s.cancelRequest(requestId));
	};

	/** Registers a FETCH the session has just taken, so that what comes for it finds it. */
	const fetchFrom = (
		requestId: bigint,
		groupOrder: GroupOrder,
		answerWait: number,
	): Fetch => {
		const giveUp = (type: "cancelled" | "no-answer-in-time"): Promise<void> =>
			giveUpFetch(requestId, type);
		const cancel = (): Promise<void> => giveUp("cancelled");
		const active: ActiveFetch = {
			body: createObjectQueue<Uint8Array>(FETCH_BODY_HIGH_WATER_MARK, cancel),
			sent: () => {},
			response: deferred(),
			ended: deferred(),
			streamEnd: undefined,
		};
		let unanswered: ReturnType<typeof setTimeout> | undefined;
		// The FETCH is written: the time its answer gets begins (as in
		// `request`).
		active.sent = () => {
			if (!fetches.has(requestId) || answered.has(requestId)) return;
			unanswered = setTimeout(() => {
				if (answered.has(requestId)) return;
				// WHY the rejection is dropped: a reset that could not be
				// written ends the session, and `closed` and `ended` report
				// that.
				giveUp("no-answer-in-time").catch(() => {});
			}, answerWait);
		};
		// A caller that reads `body` and `ended` alone leaves this unhandled.
		active.response.promise
			.catch(() => {})
			.finally(() => {
				clearTimeout(unanswered);
				answered.delete(requestId);
			});
		fetches.set(requestId, active);
		return {
			requestId,
			groupOrder,
			response: active.response.promise,
			body: active.body.readable,
			ended: active.ended.promise,
			cancel,
		};
	};

	/**
	 * A request of the peer that waits for the application: what aborts when
	 * it goes, and the one way an answer reaches the session.
	 */
	const pendingPeerRequest = (
		requestId: bigint,
	): {
		signal: AbortSignal;
		/** Gives the session the answer, unless the request is gone or has one. */
		answer<T>(action: (session: Session) => SessionResult<T>): Promise<T>;
	} => {
		const gone = new AbortController();
		pendingPeerRequests.set(requestId, gone);
		const answer = async <T>(
			action: (session: Session) => SessionResult<T>,
		): Promise<T> => {
			gone.signal.throwIfAborted();
			if (pendingPeerRequests.get(requestId) !== gone) {
				throw new Error("the request was answered already");
			}
			try {
				return await apply((s) => {
					const result = action(s);
					// Once the session took the answer: one it refuses (a
					// parameter that is none of the message) leaves the request
					// waiting.
					pendingPeerRequests.delete(requestId);
					return result;
				});
			} catch (error) {
				if (!(error instanceof InactiveError)) throw error;
				// The session had ended the request already, and the event that
				// says so is still in the queue.
				await queue;
				gone.signal.throwIfAborted();
				throw error;
			}
		};
		return { signal: gone.signal, answer };
	};

	const peerSubscribeFrom = (
		event: EventOf<"peer-subscribe">,
	): PeerSubscribe => {
		const { requestId } = event;
		const { signal, answer } = pendingPeerRequest(requestId);
		return {
			requestId,
			trackNamespace: event.trackNamespace,
			trackName: event.trackName,
			parameters: event.parameters,
			signal,
			accept: (subscribeAnswer = {}) =>
				answer((s) => {
					const { value: trackAlias, outputs } = s.acceptSubscribe(
						requestId,
						subscribeAnswer,
					);
					// The Track is registered in the same step, so that an ending
					// right behind the SUBSCRIBE_OK finds it.
					return { value: publishedTrackFrom(event, trackAlias), outputs };
				}),
			reject: (errorCode, errorReason = "") =>
				answer((s) => ({
					value: undefined,
					outputs: s.rejectPeerRequest(requestId, errorCode, errorReason),
				})),
		};
	};

	/** The fetch stream of a FETCH the session has just accepted. */
	const fetchWriterFrom = (requestId: bigint): FetchWriter => {
		const fetch: ActiveServedFetch = { ended: deferred(), end: undefined };
		servedFetches.set(requestId, fetch);
		const bounded = boundedWrites(() =>
			fetch.end === undefined
				? undefined
				: new ServedFetchEndedError(fetch.end),
		);
		/** Runs a write; a FETCH that is over fails it with how it ended. */
		const writing = async (
			action: (session: Session) => SessionOutput[],
		): Promise<void> => {
			if (fetch.end !== undefined) throw new ServedFetchEndedError(fetch.end);
			try {
				await applyVoid(action);
			} catch (error) {
				// The application's mistake, and nothing was written.
				if (error instanceof RangeError) throw error;
				if (sessionClose !== undefined) {
					throw new SessionClosedError(sessionClose);
				}
				// The subscriber may have given the FETCH up, which is what
				// failed the write, or what the session knew already; the event
				// that says so may still be in the queue.
				await queue;
				if (fetch.end !== undefined) throw new ServedFetchEndedError(fetch.end);
				if (sessionClose !== undefined) {
					throw new SessionClosedError(sessionClose);
				}
				throw error;
			}
		};
		/** `close` and `reset`: a FETCH that is over needs neither. */
		const ending = async (
			end: ServedFetchEnd,
			action: (session: Session) => SessionOutput[],
		): Promise<void> => {
			if (fetch.end !== undefined) return;
			try {
				await applyVoid((s) => {
					const outputs = action(s);
					// Once the session took the call: the FETCH is over for the
					// application, whatever becomes of the writes.
					endServedFetch(requestId, end);
					return outputs;
				});
			} catch (error) {
				if (!(error instanceof InactiveError)) throw error;
			}
		};
		return {
			requestId,
			ended: fetch.ended.promise,
			write: (object) =>
				bounded(() => writing((s) => s.writeFetchObject(requestId, object))),
			endOfRange: (kind, location) =>
				bounded(() =>
					writing((s) => s.writeFetchRangeEnd(requestId, kind, location)),
				),
			async close() {
				// Nobody asks how the subscriber ends its direction of the
				// request stream, but the stream is not left open for it: the
				// wait of `PublishedTrack.done()`.
				const wait: PeerStreamWait = { ended: deferred(), timer: undefined };
				await ending({ type: "served" }, (s) => {
					const outputs = s.finishFetch(requestId);
					// WHY here: the event for a subscriber that had finished its
					// direction is among these outputs, and ends the wait before
					// a timer is started for it.
					peerStreamWaits.set(requestId, wait);
					return outputs;
				});
				// The wait is not there when the FETCH had ended before this
				// call, so that nothing was sent, and when the subscriber ended
				// its direction meanwhile.
				if (peerStreamWaits.get(requestId) === wait) {
					// The subscriber's turn first; the session finishes this
					// endpoint's direction when the subscriber ends its own.
					startServedFetchWait(requestId, wait);
				}
			},
			reset(code = StreamResetErrorCode.CANCELLED) {
				if (
					!Number.isInteger(code) ||
					code < 0 ||
					code > MAX_STREAM_ERROR_CODE
				) {
					return Promise.reject(
						new RangeError(
							`a stream is reset with a whole number from 0 to ${MAX_STREAM_ERROR_CODE}, not ${String(code)}`,
						),
					);
				}
				return ending({ type: "reset" }, (s) => s.resetFetch(requestId, code));
			},
		};
	};

	const peerFetchFrom = (event: EventOf<"peer-fetch">): PeerFetch => {
		const { requestId } = event;
		const { signal, answer } = pendingPeerRequest(requestId);
		return {
			requestId,
			trackNamespace: event.trackNamespace,
			trackName: event.trackName,
			startLocation: event.startLocation,
			endLocation: event.endLocation,
			groupOrder: event.parameters.groupOrder ?? "Ascending",
			fillTimeout: event.parameters.fillTimeout,
			parameters: event.parameters,
			signal,
			accept: (fetchAnswer) =>
				answer((s) => {
					const outputs = s.acceptFetch(requestId, fetchAnswer);
					// The writer is registered in the same step, so that a
					// cancellation right behind the FETCH_OK finds it.
					return { value: fetchWriterFrom(requestId), outputs };
				}),
			reject: (errorCode, errorReason = "") =>
				answer((s) => ({
					value: undefined,
					outputs: s.rejectPeerRequest(requestId, errorCode, errorReason),
				})),
		};
	};

	/**
	 * Hands a request of the peer to the application, or refuses it where
	 * nobody serves its kind.
	 */
	const handOver = <Request extends PeerSubscribe | PeerFetch>(
		request: Request,
		handler: ((request: Request) => void) | undefined,
		kind: string,
	): void => {
		if (handler === undefined) {
			// WHY through `request`, like an application's answer, and not
			// straight to the session: the session may have ended the request
			// in the read that brought it (a REQUEST_UPDATE right behind a
			// SUBSCRIBE, which §10.9 allows). An answer to a request that is
			// gone is no fault of anyone, and must not end the session.
			request
				.reject(
					RequestErrorCode.NOT_SUPPORTED,
					`this endpoint serves no ${kind}`,
				)
				.catch(() => {});
			return;
		}
		const failed = (error: unknown): void => {
			// The request went while the handler was at it, and the handler
			// let the rejection of its `accept` or `reject` through, or of a
			// write on what it accepted: that is how a request the peer ends
			// ends, and no error of the application.
			if (request.signal.aborted && error === request.signal.reason) return;
			if (endedByThePeer(error)) return;
			// The handler may have answered before it failed, and the request
			// may be gone.
			request
				.reject(
					RequestErrorCode.INTERNAL_ERROR,
					"the publisher failed to handle the request",
				)
				.catch(() => {});
			// WHY not the end of the session: the error is the application's,
			// and of one request. It is reported like an error in an event
			// listener.
			reportError(error);
		};
		try {
			const returned: unknown = handler(request);
			// An `async` handler fails by the promise it returns; so does one
			// that returns a thenable of another kind.
			if (isThenable(returned)) returned.then(undefined, failed);
		} catch (error) {
			failed(error);
		}
	};

	/** Tells whoever waits on a request how it ended, from the session's reason and what arrived before it. */
	const onRequestClosed = (event: EventOf<"request-closed">): void => {
		const { requestId, reason, resetCode } = event;
		switch (reason) {
			case "peer-fin":
				rejectPending(
					requestId,
					new RequestAbortedError({ type: "peer-closed" }),
				);
				endSubscription(requestId, { type: "peer-closed" });
				endPublish(requestId, { type: "peer-closed" });
				endFetch(requestId, { type: "peer-closed" });
				return;
			case "peer-reset":
				rejectPending(
					requestId,
					new RequestAbortedError({ type: "peer-reset", code: resetCode }),
				);
				endPendingPeerRequest(
					requestId,
					new RequestAbortedError({ type: "peer-reset", code: resetCode }),
				);
				endSubscription(requestId, { type: "peer-reset", code: resetCode });
				endPublish(requestId, { type: "peer-reset", code: resetCode });
				endServedFetch(requestId, { type: "peer-cancelled", code: resetCode });
				endFetch(requestId, { type: "peer-reset", code: resetCode });
				return;
			case "unsupported-track-property":
				// Happens on SUBSCRIBE_OK, where only the pending subscribe()
				// learns of it, and on FETCH_OK.
				rejectPending(
					requestId,
					new RequestAbortedError({ type: "unsupported-track-property" }),
				);
				endFetch(requestId, { type: "unsupported-track-property" });
				return;
			case "fetch-served": {
				const streamEnd = fetches.get(requestId)?.streamEnd;
				endFetch(
					requestId,
					streamEnd?.how === "reset"
						? { type: "stream-reset", code: streamEnd.resetCode }
						: { type: "served" },
				);
				return;
			}
			case "update-failed": {
				// A SUBSCRIBE of the peer that had no answer: no PUBLISH_DONE was
				// sent, so there is no wait for what follows one.
				if (
					endPendingPeerRequest(
						requestId,
						new RequestAbortedError({ type: "update-failed" }),
					)
				) {
					// It was refused with a REQUEST_ERROR.
					awaitPeerEndAfterRefusal(requestId);
					return;
				}
				endPublish(requestId, { type: "update-failed" });
				// The session sent PUBLISH_DONE UPDATE_FAILED itself. Nobody
				// asked how the subscriber ends its direction, but the stream is
				// not left open for it: the same wait, as in the Rust client.
				const wait: PeerStreamWait = { ended: deferred(), timer: undefined };
				peerStreamWaits.set(requestId, wait);
				startPeerStreamTimer(requestId, wait);
				return;
			}
			case "malformed-track":
				endSubscription(requestId, { type: "malformed-track" });
				return;
			case "publish-done": {
				const subscription = subscriptions.get(requestId);
				// A subscription cancelled by the application is gone already.
				if (subscription === undefined) return;
				const done = subscription.publishDone;
				if (done === undefined) {
					throw new Error(
						`request ${requestId} ended by PUBLISH_DONE, but no PUBLISH_DONE was seen`,
					);
				}
				endSubscription(requestId, {
					type: "publish-done",
					statusCode: done.statusCode,
					errorReason: done.errorReason,
				});
				return;
			}
		}
	};

	/** Acts on a session event. What is returned is backpressure from the application. */
	const onEvent = (event: SessionEvent): Backpressure | undefined => {
		options.diagnostics?.onEvent?.(event);
		switch (event.type) {
			case "setup":
				peerSetup.resolve(event.setupOptions);
				return undefined;
			case "go-away":
				goAway.resolve({
					newSessionUri: event.newSessionUri,
					// WHY no bound: a time in milliseconds that is past what a
					// Number holds whole is no time anybody waits for.
					timeoutMs: Number(event.timeout),
				});
				return undefined;
			case "publish-namespace-ok":
				pendingNamespaces.get(event.requestId)?.resolve();
				pendingNamespaces.delete(event.requestId);
				return undefined;
			case "publish-ok": {
				const { requestId } = event;
				const trackAlias = pendingPublishAliases.get(requestId);
				// The Track is registered here, in the same step as the response,
				// so that an ending right behind it finds the Track.
				if (trackAlias !== undefined) {
					pendingPublishes
						.get(requestId)
						?.resolve(publishedTrackFrom(event, trackAlias));
				}
				pendingPublishes.delete(requestId);
				pendingPublishAliases.delete(requestId);
				return undefined;
			}
			case "subscribe-ok":
				pendingSubscribes
					.get(event.requestId)
					?.resolve(subscriptionFrom(event));
				pendingSubscribes.delete(event.requestId);
				return undefined;
			case "new-group-request": {
				const publish = publishes.get(event.requestId);
				if (publish !== undefined) newGroupAsked(publish, event.group);
				return undefined;
			}
			case "update-ok":
				subscriptions
					.get(event.requestId)
					?.updates.shift()
					?.resolve(event.parameters);
				return undefined;
			case "update-error": {
				// One REQUEST_ERROR may answer every update that waits (§10.9.1).
				const error = new RequestFailedError({
					errorCode: event.errorCode,
					retryInterval: event.retryInterval,
					errorReason: event.errorReason,
				});
				const waiting = subscriptions.get(event.requestId)?.updates ?? [];
				for (const update of waiting.splice(0)) update.reject(error);
				return undefined;
			}
			case "request-error": {
				const error = new RequestFailedError(event);
				rejectPending(event.requestId, error);
				endFetch(event.requestId, { type: "rejected", error });
				return undefined;
			}
			case "fetch-ok":
				fetches.get(event.requestId)?.response.resolve({
					endOfTrack: event.endOfTrack,
					endLocation: event.endLocation,
					parameters: event.parameters,
					trackProperties: event.trackProperties,
				});
				return undefined;
			case "fetch-data": {
				const fetch = fetches.get(event.requestId);
				if (fetch === undefined) return undefined;
				// The fetch stream is not read further until `body` has room.
				return { settled: fetch.body.push(event.data) };
			}
			case "fetch-stream-ended": {
				const fetch = fetches.get(event.requestId);
				if (fetch === undefined) return undefined;
				fetch.streamEnd = event;
				fetch.body.end(
					event.how === "reset"
						? new FetchStreamResetError(event.resetCode)
						: undefined,
				);
				return undefined;
			}
			case "request-closed":
				onRequestClosed(event);
				return undefined;
			case "peer-direction-ended":
				endPeerStreamWait(
					event.requestId,
					event.how === "fin"
						? { type: "fin" }
						: { type: "reset", code: event.resetCode },
				);
				return undefined;
			case "publish-done": {
				const { requestId } = event;
				const subscription = subscriptions.get(requestId);
				if (subscription === undefined) return undefined;
				subscription.publishDone = event;
				clearTimeout(subscription.publishDoneTimer);
				subscription.publishDoneTimer = setTimeout(
					() => applyInput((s) => s.expireSubscription(requestId)),
					publishDoneTimeoutMs,
				);
				return undefined;
			}
			case "object": {
				const subscription = subscriptions.get(event.requestId);
				if (subscription === undefined) return undefined;
				return {
					settled: subscription.objects.push({
						groupId: event.groupId,
						subgroupId: event.subgroupId,
						publisherPriority: event.publisherPriority,
						object: event.object,
					}),
				};
			}
			case "peer-subscribe":
				handOver(
					peerSubscribeFrom(event),
					serveHandlers.subscribe,
					"SUBSCRIBE",
				);
				return undefined;
			case "peer-fetch":
				handOver(peerFetchFrom(event), serveHandlers.fetch, "FETCH");
				return undefined;
			case "peer-request-rejected":
				awaitPeerEndAfterRefusal(event.requestId);
				return undefined;
			case "subgroup-abandoned":
			case "peer-stream-timed-out":
				// Reported through `options.diagnostics` only.
				return undefined;
		}
	};

	const perform = async (
		output: SessionOutput,
	): Promise<Backpressure | undefined> => {
		switch (output.type) {
			case "close-session": {
				// The error says what the peer is told: the reason as it is sent.
				const reason = closeReason(output.reason);
				teardown({
					type: "error",
					error: new MoqtError(output.code, reason),
				});
				transport.close({ closeCode: output.code, reason });
				return undefined;
			}
			case "event":
				return onEvent(output.event);
			default:
				return io.perform(output);
		}
	};

	/**
	 * Queues outputs behind the ones already queued and resolves with their
	 * backpressure promises once all of them were performed.
	 *
	 * A session whose outputs were not all performed is out of step with its
	 * transport, so a failure here ends the session.
	 */
	const enqueue = (outputs: SessionOutput[]): Promise<Promise<void>[]> => {
		const run = queue.then(async () => {
			const waits: Promise<void>[] = [];
			for (const output of outputs) {
				if (sessionClose !== undefined) break;
				const wait = await perform(output);
				if (wait !== undefined) waits.push(wait.settled);
			}
			return waits;
		});
		// Later outputs still run in order; this failure is reported below.
		queue = run.catch(() => {});
		return run.catch((error: unknown) => {
			fatal(toError(error));
			throw error;
		});
	};

	/** Runs a `Session` method for the application and performs its outputs. */
	const apply = async <T>(
		action: (session: Session) => SessionResult<T>,
	): Promise<T> => {
		if (sessionClose !== undefined) throw new SessionClosedError(sessionClose);
		// The session is called synchronously, so outputs are queued in the order
		// the session produced them even when callers overlap.
		const { value, outputs } = action(session);
		// The application waits for its writes and sees them fail.
		await Promise.all(await enqueue(outputs));
		return value;
	};

	const applyVoid = (
		action: (session: Session) => SessionOutput[],
	): Promise<void> => apply((s) => ({ value: undefined, outputs: action(s) }));

	/** Reports input from the transport, or a timer, to the session and performs its outputs. */
	async function applyInput(
		action: (session: Session) => SessionOutput[],
	): Promise<void> {
		if (sessionClose !== undefined) return;
		try {
			const outputs = action(session);
			// WHY here and not where the event is handled: outputs are
			// performed in one line, and an answer waits there behind a
			// request whose stream the peer gives no credit for yet. The
			// answer has come when the session has read it.
			for (const output of outputs) {
				if (output.type === "event" && isAnswer(output.event)) {
					answered.add(output.event.requestId);
				}
			}
			const waits = await enqueue(outputs);
			// The source of this input is not read further until its outputs were
			// taken. A write that fails here answers a peer that has left the
			// stream, which is not an error of this input.
			await Promise.allSettled(waits);
		} catch (error) {
			// `action` threw something that is not the peer's doing, or an output
			// could not be performed.
			fatal(toError(error));
		}
	}

	io.acceptPeerStreams();
	transport.closed.then(
		(info) =>
			teardown({ type: "peer", code: info.closeCode, reason: info.reason }),
		(error: unknown) =>
			teardown({
				type: "error",
				error: new Error(`the transport session failed: ${String(error)}`),
			}),
	);

	/**
	 * Sends a request and waits for its response. `pending` is where the
	 * response is delivered; `giveUp` is what the session does with a
	 * request that got no answer in time.
	 */
	const request = <T>(
		pending: Map<bigint, Deferred<T>>,
		parameters: MessageParameters | undefined,
		action: (session: Session) => SessionResult<bigint>,
		giveUp: (requestId: bigint) => Promise<void> = (requestId) =>
			applyVoid((s) => s.cancelRequest(requestId)),
	): Promise<T> => {
		const response = deferred<T>();
		let unanswered: ReturnType<typeof setTimeout> | undefined;
		let sent: bigint | undefined;
		apply((s) => {
			const result = action(s);
			sent = result.value;
			// Registered before the request is sent, so the response finds it.
			pending.set(sent, response);
			return result;
		}).then(
			(requestId) => {
				// WHY from here: the time is the peer's to answer in, and it
				// begins when the request is written. A request whose stream
				// the peer gives no credit for waits before this, and would be
				// given up before the peer had it.
				if (pending.get(requestId) !== response || answered.has(requestId)) {
					return;
				}
				unanswered = setTimeout(
					() => {
						if (pending.get(requestId) !== response) return;
						if (answered.has(requestId)) return;
						rejectPending(
							requestId,
							new RequestAbortedError({ type: "no-answer-in-time" }),
						);
						// WHY the rejection is dropped: what could not be
						// written ends the session, and `closed` reports that.
						giveUp(requestId).catch(() => {});
					},
					answerWaitMs(answerTimeoutMs, parameters),
				);
			},
			async (error: unknown) => {
				// The peer allowed no stream for it: the request was not sent.
				// The session forgets it, which resets a stream that is not
				// there and does nothing more.
				if (error instanceof StreamNotOpenedError && sent !== undefined) {
					const requestId = sent;
					rejectPending(
						requestId,
						new RequestAbortedError({ type: "not-sent-in-time" }),
					);
					applyVoid((s) => s.cancelRequest(requestId)).catch(() => {});
					return;
				}
				// WHY one turn of the timers before the error is given: where the
				// peer closes the session as the request is being written, the
				// write fails 0.3 ms before `closed` of the transport settles, and
				// with a NetworkError that does not say it is the session's
				// (Chromium 156, 4 of 10 against a relay that closes at SETUP).
				// The request then fails with how the session ended, as the
				// requests after it do.
				// WHY not until `closed` settles: a write may fail for the stream
				// alone, and the request would never be answered.
				if (sessionClose === undefined) {
					await Promise.race([
						closed.promise,
						new Promise((resolve) => setTimeout(resolve, 0)),
					]);
				}
				response.reject(
					sessionClose === undefined
						? toError(error)
						: new SessionClosedError(sessionClose),
				);
			},
		);
		return response.promise.finally(() => {
			clearTimeout(unanswered);
			if (sent !== undefined) answered.delete(sent);
		});
	};

	/**
	 * Ends a PUBLISH that got no answer in time: PUBLISH_DONE and a FIN, and
	 * the subscriber has the time of `startPeerStreamTimer` to end its
	 * direction, as after `done()`.
	 *
	 * WHY PUBLISH_DONE, where every other request is reset: a PUBLISH that
	 * waits for its answer is a subscription in the state Pending
	 * (Publisher), and §5.1 has "The publisher terminates a subscription in
	 * the Pending (Publisher) or Established states by sending PUBLISH_DONE
	 * and closing the stream"; §11.4.1: "When possible, Publishers SHOULD
	 * send a PUBLISH_DONE when terminating a subscription instead of
	 * abruptly terminating the associated control stream".
	 * WHY INTERNAL_ERROR: §10.11 has no code for a time that ran out
	 * (EXPIRED is "the timeout specified in SUBSCRIBE_OK"), and this one is
	 * "An implementation specific or generic error". The session counts the
	 * streams itself: none, so Stream Count is 0.
	 */
	const endUnansweredPublish = (requestId: bigint): Promise<void> => {
		const wait: PeerStreamWait = { ended: deferred(), timer: undefined };
		return applyVoid((s) => {
			const outputs = s.publishDone(
				requestId,
				PUBLISH_DONE_INTERNAL_ERROR,
				NO_ANSWER_IN_TIME,
			);
			peerStreamWaits.set(requestId, wait);
			startPeerStreamTimer(requestId, wait);
			return outputs;
		});
	};

	const setupTimer = setTimeout(
		() =>
			fatal(
				new MoqtError(
					SessionErrorCode.CONTROL_MESSAGE_TIMEOUT,
					"the peer sent no SETUP in time",
				),
				SessionErrorCode.CONTROL_MESSAGE_TIMEOUT,
			),
		setupTimeoutMs,
	);
	let peerSetupOptions: KeyValuePair[];
	try {
		await applyVoid((s) => s.start());
		peerSetupOptions = await peerSetup.promise;
	} catch (error) {
		fatal(toError(error));
		throw error;
	} finally {
		clearTimeout(setupTimer);
	}

	/** The parameters of a request with the connection's token before its own, where the connection has one. */
	const authorized = (
		parameters: MessageParameters | undefined,
	): MessageParameters | undefined =>
		options.authorizationToken === undefined
			? parameters
			: {
					...parameters,
					authorizationToken: [
						options.authorizationToken,
						...(parameters?.authorizationToken ?? []),
					],
				};

	return {
		serve(handlers) {
			serveHandlers = handlers;
		},
		peerSetupOptions,
		closed: closed.promise,
		goAway: goAway.promise,
		publishNamespace(trackNamespace, parameters) {
			return request(pendingNamespaces, parameters, (s) =>
				s.publishNamespace({
					trackNamespace,
					parameters: authorized(parameters),
				}),
			);
		},
		publish(asked) {
			const publishRequest = {
				...asked,
				parameters: authorized(asked.parameters),
			};
			return request(
				pendingPublishes,
				asked.parameters,
				(s) => {
					const result = s.publish(publishRequest);
					pendingPublishAliases.set(result.value, publishRequest.trackAlias);
					return result;
				},
				endUnansweredPublish,
			);
		},
		subscribe(asked) {
			return request(pendingSubscribes, asked.parameters, (s) =>
				s.subscribe({ ...asked, parameters: authorized(asked.parameters) }),
			);
		},
		async fetch(asked, fetchOptions = {}) {
			const fetchRequest = {
				...asked,
				parameters: authorized(asked.parameters),
			};
			const { signal } = fetchOptions;
			signal?.throwIfAborted();
			let registered: Fetch | undefined;
			const handle = await apply((s) => {
				const { value, outputs } = s.fetch(fetchRequest);
				// Registered before the request is sent, so that its answer and
				// its stream find it.
				registered = fetchFrom(
					value,
					fetchRequest.parameters?.groupOrder ?? "Ascending",
					answerWaitMs(answerTimeoutMs, fetchRequest.parameters),
				);
				return { value: registered, outputs };
			}).catch(async (error: unknown) => {
				// The peer allowed no stream for it: the FETCH was not sent.
				if (error instanceof StreamNotOpenedError && registered !== undefined) {
					await giveUpFetch(registered.requestId, "not-sent-in-time");
					throw new RequestAbortedError({ type: "not-sent-in-time" });
				}
				throw error;
			});
			fetches.get(handle.requestId)?.sent();
			if (signal !== undefined) {
				// WHY the rejection is dropped: `cancel()` rejects when the reset
				// could not be written, which ends the session, and `closed` and
				// `ended` report that. Nobody holds the promise of this call.
				const onAbort = (): void => {
					handle.cancel().catch(() => {});
				};
				signal.addEventListener("abort", onAbort, { once: true });
				void handle.ended.then(() =>
					signal.removeEventListener("abort", onAbort),
				);
				// It may have aborted while the FETCH was written.
				if (signal.aborted) onAbort();
			}
			return handle;
		},
		close(code = SessionErrorCode.NO_ERROR, reason = "") {
			// Before the session is touched: a code that cannot be sent leaves it
			// open.
			const sentCode = closeCode(code);
			if (sessionClose !== undefined) return;
			const sentReason = closeReason(reason);
			teardown({ type: "local", code: sentCode, reason: sentReason });
			transport.close({ closeCode: sentCode, reason: sentReason });
		},
	};
}
