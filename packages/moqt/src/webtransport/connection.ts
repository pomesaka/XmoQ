import {
	InactiveError,
	MoqtError,
	SessionErrorCode,
	StreamResetErrorCode,
} from "../core/error.ts";
import type { KeyValuePair } from "../core/key-value-pair.ts";
import type { SessionLimits } from "../core/limits.ts";
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
	FetchRequest,
	PublishRequest,
	SubscribeRequest,
} from "../core/requests.ts";
import { createSession, type Session } from "../core/session.ts";
import type {
	SessionEvent,
	SessionOutput,
	SessionResult,
} from "../core/session-output.ts";
import type { SubgroupObject } from "../core/subgroup.ts";
import type { TrackNamespace } from "../core/track.ts";
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
	type SessionClose,
	SessionClosedError,
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
	 * 64 KiB each, for a FETCH that is decoded whole before it is refused.
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

/** A subscription this endpoint started with PUBLISH. */
export interface PublishedTrack {
	readonly requestId: bigint;
	readonly trackAlias: bigint;
	/** Parameters of the PUBLISH_OK. */
	readonly parameters: MessageParameters;
	/** Resolves once the subscription has ended, with how. Never rejects. */
	readonly ended: Promise<PublishEnd>;
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
 * publisher that never answers leaves it open, where it takes up one of the
 * requests the session may have: nothing here ends it after a time, so give
 * the call a `signal` or call `cancel()`.
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

export interface MoqtConnection {
	/** Setup Options of the peer's SETUP. */
	readonly peerSetupOptions: readonly KeyValuePair[];
	/** Resolves once the session has ended, with how. Never rejects. */
	readonly closed: Promise<SessionClose>;
	/**
	 * The three requests reject with `RequestFailedError`,
	 * `RequestAbortedError` or `SessionClosedError`.
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
}

/** What the connection keeps for a FETCH, from the moment it is sent. */
interface ActiveFetch {
	body: ObjectQueue<Uint8Array>;
	response: Deferred<FetchResponse>;
	ended: Deferred<FetchEnd>;
	/** How the publisher ended the fetch stream, once it has. */
	streamEnd: EventOf<"fetch-stream-ended"> | undefined;
}

/** A `done()` that waits for the subscriber to end its direction of the request stream. */
interface PeerStreamWait {
	ended: Deferred<PeerStreamEnd>;
	timer: ReturnType<typeof setTimeout> | undefined;
}

/** What the connection keeps for a PUBLISH, from its PUBLISH_OK on. */
interface ActivePublish {
	ended: Deferred<PublishEnd>;
	/** Set once the subscription has ended. */
	end: PublishEnd | undefined;
	/** The session took a PUBLISH_DONE for it; `end` follows when it is written. */
	doneTaken: boolean;
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
	const session: Session = createSession({
		implementation: options.implementation,
		limits: options.limits,
	});
	const pendingNamespaces = new Map<bigint, Deferred<void>>();
	const pendingPublishes = new Map<bigint, Deferred<PublishedTrack>>();
	const pendingSubscribes = new Map<bigint, Deferred<Subscription>>();
	/** Track Alias of each PUBLISH that is waiting for its response. */
	const pendingPublishAliases = new Map<bigint, bigint>();
	const subscriptions = new Map<bigint, ActiveSubscription>();
	const publishes = new Map<bigint, ActivePublish>();
	const fetches = new Map<bigint, ActiveFetch>();
	const peerStreamWaits = new Map<bigint, PeerStreamWait>();
	/** Serializes outputs so that writes reach each stream in the order the session produced them. */
	let queue: Promise<unknown> = Promise.resolve();
	let sessionClose: SessionClose | undefined;
	const closed = deferred<SessionClose>();
	const peerSetup = deferred<KeyValuePair[]>();
	// The session can end before anything awaits this promise; the failure is
	// reported by the `await` below when it is reached, and by `closed`.
	peerSetup.promise.catch(() => {});

	const io = createStreamIo({
		transport,
		maxPeerUniStreams,
		maxPeerBidiStreams,
		firstBytesTimeoutMs,
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
			void applyInput((s) => s.expirePublishDonePeerEnd(requestId));
		}, peerStreamEndTimeoutMs);
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
				fetch.response.reject(new RequestAbortedError({ type: "cancelled" }));
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
		};
		subscriptions.set(requestId, active);
		return {
			requestId,
			trackAlias: event.trackAlias,
			parameters: event.parameters,
			trackProperties: event.trackProperties,
			objects: active.objects.readable,
			ended: active.ended.promise,
			cancel: cancelRequest,
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
			const closedHere = resetHere() && transport.isStreamError(error);
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
		return {
			write: (object) =>
				onPublish(
					publish,
					() => applyVoid((s) => s.writeObject(stream, object)),
					() => resetHere,
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

	const publishedTrackFrom = (
		event: EventOf<"publish-ok">,
		trackAlias: bigint,
	): PublishedTrack => {
		const { requestId } = event;
		const publish: ActivePublish = {
			ended: deferred(),
			end: undefined,
			doneTaken: false,
		};
		publishes.set(requestId, publish);
		return {
			requestId,
			trackAlias,
			parameters: event.parameters,
			ended: publish.ended.promise,
			openSubgroup: (subgroupOptions) =>
				onPublish(publish, async () =>
					subgroupWriter(
						publish,
						await apply((s) => s.openSubgroup(requestId, subgroupOptions)),
					),
				),
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

	/** Registers a FETCH the session has just taken, so that what comes for it finds it. */
	const fetchFrom = (requestId: bigint, groupOrder: GroupOrder): Fetch => {
		const cancel = (): Promise<void> => {
			if (!fetches.has(requestId)) return Promise.resolve();
			endFetch(requestId, { type: "cancelled" });
			// The session agrees: cancelling after the end does nothing.
			if (sessionClose !== undefined) return Promise.resolve();
			return applyVoid((s) => s.cancelRequest(requestId));
		};
		const active: ActiveFetch = {
			body: createObjectQueue<Uint8Array>(FETCH_BODY_HIGH_WATER_MARK, cancel),
			response: deferred(),
			ended: deferred(),
			streamEnd: undefined,
		};
		// A caller that reads `body` and `ended` alone leaves this unhandled.
		active.response.promise.catch(() => {});
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
				endSubscription(requestId, { type: "peer-reset", code: resetCode });
				endPublish(requestId, { type: "peer-reset", code: resetCode });
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
			case "publish-done-peer-ended":
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
			case "subgroup-abandoned":
			case "peer-stream-timed-out":
			case "peer-request-rejected":
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
			const waits = await enqueue(action(session));
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

	/** Sends a request and waits for its response. `pending` is where the response is delivered. */
	const request = <T>(
		pending: Map<bigint, Deferred<T>>,
		action: (session: Session) => SessionResult<bigint>,
	): Promise<T> => {
		const response = deferred<T>();
		apply((s) => {
			const result = action(s);
			// Registered before the request is sent, so the response finds it.
			pending.set(result.value, response);
			return result;
		}).catch(response.reject);
		return response.promise;
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

	return {
		peerSetupOptions,
		closed: closed.promise,
		publishNamespace(trackNamespace, parameters) {
			return request(pendingNamespaces, (s) =>
				s.publishNamespace({ trackNamespace, parameters }),
			);
		},
		publish(publishRequest) {
			return request(pendingPublishes, (s) => {
				const result = s.publish(publishRequest);
				pendingPublishAliases.set(result.value, publishRequest.trackAlias);
				return result;
			});
		},
		subscribe(subscribeRequest) {
			return request(pendingSubscribes, (s) => s.subscribe(subscribeRequest));
		},
		async fetch(fetchRequest, fetchOptions = {}) {
			const { signal } = fetchOptions;
			signal?.throwIfAborted();
			const handle = await apply((s) => {
				const { value, outputs } = s.fetch(fetchRequest);
				// Registered before the request is sent, so that its answer and
				// its stream find it.
				return {
					value: fetchFrom(
						value,
						fetchRequest.parameters?.groupOrder ?? "Ascending",
					),
					outputs,
				};
			});
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
