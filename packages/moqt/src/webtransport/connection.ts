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
import type { MessageParameters } from "../core/parameters.ts";
import type { PublishRequest, SubscribeRequest } from "../core/requests.ts";
import { createSession, type Session } from "../core/session.ts";
import type {
	SessionEvent,
	SessionOutput,
	SessionResult,
} from "../core/session-output.ts";
import type { SubgroupObject } from "../core/subgroup.ts";
import type { TrackNamespace } from "../core/track.ts";
import type { Diagnostics } from "./diagnostics.ts";
import {
	type PublishEnd,
	PublishEndedError,
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
 * §10.11 asks for at least the larger of the two delivery timeouts. Neither
 * is implemented, so this is a fixed value for now.
 */
const DEFAULT_PUBLISH_DONE_TIMEOUT_MS = 5_000;

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
	/** How long to wait for the peer's SETUP before closing with CONTROL_MESSAGE_TIMEOUT. */
	setupTimeoutMs?: number;
	/**
	 * How long a subscription waits for late streams after PUBLISH_DONE when
	 * its Stream Count is not reached (§10.11).
	 */
	publishDoneTimeoutMs?: number;
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
	 * subscription is over, with `SubgroupClosedError` when only this stream is
	 * (it was closed, reset, or stopped by the peer), and with the transport's
	 * error when the write itself fails. In each case `reset()` is safe to call.
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
	 * Sends PUBLISH_DONE and closes the request stream (§10.11). Every
	 * subgroup stream has to be closed or reset first; otherwise this rejects
	 * with an `Error` and nothing is sent. Rejects with `PublishEndedError`
	 * once the subscription is over.
	 */
	done(statusCode: bigint, errorReason: string): Promise<void>;
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
	/** Closes the session (§3.5). */
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

/** What the connection keeps for a PUBLISH, from its PUBLISH_OK on. */
interface ActivePublish {
	ended: Deferred<PublishEnd>;
	/** Set once the subscription has ended. */
	end: PublishEnd | undefined;
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

	const endPublish = (requestId: bigint, end: PublishEnd): void => {
		const publish = publishes.get(requestId);
		if (publish === undefined) return;
		publishes.delete(requestId);
		publish.end = end;
		publish.ended.resolve(end);
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
		closed.resolve(close);
	}

	/** Ends the session because this endpoint cannot go on. */
	function fatal(
		error: Error,
		code: number = SessionErrorCode.INTERNAL_ERROR,
	): void {
		if (sessionClose !== undefined) return;
		teardown({ type: "error", error });
		transport.close({ closeCode: code, reason: error.message });
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
	 * Track goes on fails with `SubgroupClosedError`.
	 */
	const onPublish = async <T>(
		publish: ActivePublish,
		call: () => Promise<T>,
	): Promise<T> => {
		if (publish.end !== undefined) throw new PublishEndedError(publish.end);
		try {
			return await call();
		} catch (error) {
			if (!(error instanceof InactiveError)) throw error;
			// The session had already ended the subscription or the stream, and the
			// event that says so may still be in the queue. Once the queue is
			// through, `publish.end` tells which of the two it was.
			await queue;
			if (publish.end !== undefined) throw new PublishEndedError(publish.end);
			throw new SubgroupClosedError(error.message);
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
	): SubgroupWriter => ({
		write: (object) =>
			onPublish(publish, () => applyVoid((s) => s.writeObject(stream, object))),
		close: () =>
			onPublishIfOpen(publish, () => applyVoid((s) => s.closeSubgroup(stream))),
		reset: (code = StreamResetErrorCode.CANCELLED) => {
			// Before anything is done to the stream: a code the browser cannot
			// send must not leave the stream half reset.
			if (!Number.isInteger(code) || code < 0 || code > MAX_STREAM_ERROR_CODE) {
				return Promise.reject(
					new RangeError(
						`a stream is reset with a whole number from 0 to ${MAX_STREAM_ERROR_CODE}, not ${code}`,
					),
				);
			}
			return onPublishIfOpen(publish, () =>
				applyVoid((s) => s.resetSubgroup(stream, code)),
			);
		},
	});

	const publishedTrackFrom = (
		event: EventOf<"publish-ok">,
		trackAlias: bigint,
	): PublishedTrack => {
		const { requestId } = event;
		const publish: ActivePublish = { ended: deferred(), end: undefined };
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
					await applyVoid((s) =>
						s.publishDone(requestId, statusCode, errorReason),
					);
					endPublish(requestId, { type: "done" });
				}),
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
				return;
			case "peer-reset":
				rejectPending(
					requestId,
					new RequestAbortedError({ type: "peer-reset", code: resetCode }),
				);
				endSubscription(requestId, { type: "peer-reset", code: resetCode });
				endPublish(requestId, { type: "peer-reset", code: resetCode });
				return;
			case "unsupported-track-property":
				// Happens on SUBSCRIBE_OK, so only the pending subscribe() learns of it.
				rejectPending(
					requestId,
					new RequestAbortedError({ type: "unsupported-track-property" }),
				);
				return;
			case "update-failed":
				endPublish(requestId, { type: "update-failed" });
				return;
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
			case "request-error":
				rejectPending(event.requestId, new RequestFailedError(event));
				return undefined;
			case "request-closed":
				onRequestClosed(event);
				return undefined;
			case "publish-done": {
				const { requestId } = event;
				const subscription = subscriptions.get(requestId);
				if (subscription === undefined) return undefined;
				subscription.publishDone = event;
				clearTimeout(subscription.publishDoneTimer);
				subscription.publishDoneTimer = setTimeout(
					() => applyInput((s) => s.expireSubscription(requestId)),
					options.publishDoneTimeoutMs ?? DEFAULT_PUBLISH_DONE_TIMEOUT_MS,
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
			case "peer-request-rejected":
				// Reported through `options.diagnostics` only.
				return undefined;
		}
	};

	const perform = async (
		output: SessionOutput,
	): Promise<Backpressure | undefined> => {
		switch (output.type) {
			case "close-session":
				teardown({
					type: "error",
					error: new MoqtError(output.code, output.reason),
				});
				transport.close({ closeCode: output.code, reason: output.reason });
				return undefined;
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
		options.setupTimeoutMs ?? DEFAULT_SETUP_TIMEOUT_MS,
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
		close(code = SessionErrorCode.NO_ERROR, reason = "") {
			if (sessionClose !== undefined) return;
			teardown({ type: "local", code, reason });
			transport.close({ closeCode: code, reason });
		},
	};
}
