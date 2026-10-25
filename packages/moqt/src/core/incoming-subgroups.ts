import { StreamResetErrorCode } from "./error.ts";
import type { SessionLimits } from "./limits.ts";
import type { SessionEvent, SessionOutput } from "./session-output.ts";
import {
	createSubgroupStreamDecoder,
	type SubgroupHeader,
	type SubgroupObject,
	type SubgroupStreamDecoder,
} from "./subgroup.ts";

export interface IncomingSubgroupsOptions {
	limits: SessionLimits;
	/** Whether a SUBSCRIBE is waiting for its SUBSCRIBE_OK, which could still establish an unknown Track Alias. */
	hasPendingSubscribe(): boolean;
	/** §2.4.2: an Object showed the Track to be malformed; its subscription has to be cancelled. */
	onMalformedTrack(requestId: bigint, out: SessionOutput[]): void;
	/**
	 * A subscription that received PUBLISH_DONE has nothing left to wait for:
	 * every stream it announced has ended (§10.11), or its Track Alias was given
	 * to another subscription.
	 */
	onDrained(requestId: bigint, out: SessionOutput[]): void;
}

/** What a Track Alias is bound to. */
export type AliasState =
	/** No subscription uses it. */
	| "free"
	/** An Established subscription uses it. */
	| "established"
	/** A subscription that received PUBLISH_DONE still uses it for late streams. */
	| "draining";

/**
 * The subscriber's side of data streams. It owns the table of Track Aliases:
 * which subscription an alias belongs to, how many of its streams ended and
 * whether PUBLISH_DONE arrived. It decodes the subgroup streams the peer
 * opens and holds the Objects that arrive before their alias is established.
 */
export interface IncomingSubgroups {
	aliasState(trackAlias: bigint): AliasState;
	/**
	 * Binds a Track Alias that is free or draining to a subscription. A
	 * draining subscription ends at that point: §11.1 forbids an alias shared
	 * by two Established subscriptions, and one that received PUBLISH_DONE is
	 * Terminated for its publisher (§5.1), which may use the alias again at
	 * once (§5.1.1).
	 *
	 * `maxObjectPayloadLength` is the subscription's limit, at most the
	 * connection's; it applies to each Object of the alias's streams whose
	 * Object Payload Length is read from now on.
	 */
	establish(
		trackAlias: bigint,
		requestId: bigint,
		maxObjectPayloadLength: number,
		out: SessionOutput[],
	): void;
	/**
	 * Remembers a free Track Alias as one whose streams are to be dropped,
	 * without ever binding it (§11.1).
	 */
	retire(trackAlias: bigint): void;
	/** PUBLISH_DONE arrived with this Stream Count. */
	expectStreams(
		trackAlias: bigint,
		streamCount: bigint,
		out: SessionOutput[],
	): void;
	/**
	 * The subscription ended. Its open streams are stopped with `code`
	 * (§10.11) and Objects for the alias are dropped from now on.
	 */
	release(trackAlias: bigint, code: number, out: SessionOutput[]): void;
	open(peerStream: number): void;
	/** Bytes of a subgroup stream. Throws the `MoqtError` that ends the session. */
	receive(
		peerStream: number,
		data: Uint8Array,
		fin: boolean,
		out: SessionOutput[],
	): void;
	/** The peer reset the stream. */
	reset(peerStream: number, out: SessionOutput[]): void;
	/**
	 * Drops what is held of a stream whose Subgroup Header has not fully
	 * arrived, and says whether there was such a stream. One that has its
	 * header, or was abandoned, is left as it is.
	 *
	 * WHY one call that asks and drops: whoever asks does so to stop the
	 * stream, and a stream that was stopped and not dropped would stay here
	 * for as long as the session lasts.
	 */
	dropBeforeHeader(peerStream: number): boolean;
	/**
	 * The stream, whose Subgroup Header is whole, sent nothing for the idle
	 * time (`watch-peer-stream-idle`). It is stopped as one whose Objects are
	 * lost: what it held is dropped, it counts as ended for PUBLISH_DONE
	 * (§10.11), and its subscription goes on (§11.4.3: "Resets and
	 * STOP_SENDING on SUBSCRIBE data streams have no impact on other
	 * Subgroups in the Group or the subscription"). Does nothing for a stream
	 * that has ended or was stopped before, so a time reported late does no
	 * harm.
	 */
	idle(peerStream: number, out: SessionOutput[]): void;
	/** A SUBSCRIBE left Pending: deliver or drop what was held for it. */
	resolveHeld(out: SessionOutput[]): void;
	clear(): void;
}

type ObjectEvent = Extract<SessionEvent, { type: "object" }>;
/** An Object whose subscription is not known yet. */
type HeldObject = Omit<ObjectEvent, "requestId">;

interface SubgroupStream {
	decoder: SubgroupStreamDecoder;
	header: SubgroupHeader | undefined;
	/** Resolved from the header, or from the first Object in FirstObjectId mode. */
	subgroupId: bigint | undefined;
	/** Set once the Track Alias matched an established subscription. */
	requestId: bigint | undefined;
	/** Objects held until the Track Alias is established. */
	held: HeldObject[];
	heldBytes: number;
	/** The stream was stopped; what still arrives on it is dropped. */
	abandoned: boolean;
}

/** What is known about the subscription a Track Alias belongs to. */
interface AliasBinding {
	requestId: bigint;
	/** The longest Object Payload Length taken for the subscription. */
	maxObjectPayloadLength: number;
	/** Streams of this subscription that ended, however they ended. */
	streamsEnded: bigint;
	/** PUBLISH_DONE arrived. */
	draining: boolean;
	/** Stream Count of PUBLISH_DONE, when it is exact. */
	streamsExpected: bigint | undefined;
}

/** Why a stream is stopped, with the Stream Reset Error Code that says so (§3.3.3). */
interface StopReason {
	reason: string;
	code: number;
}

/** §10.11: a publisher that cannot count its streams sends this Stream Count. */
const STREAM_COUNT_UNKNOWN = (1n << 62n) - 1n;

/**
 * Track Aliases of ended subscriptions that are remembered, so that late
 * Objects are dropped at once instead of being held as an unknown alias
 * (§11.1). Provisional; the Rust implementation uses the same number.
 */
const RETIRED_ALIAS_LIMIT = 64;

/**
 * Unknown Track Aliases for which streams that ended without an Object are
 * counted, until a SUBSCRIBE_OK establishes the alias. Provisional; the Rust
 * implementation uses the same number.
 */
const EARLY_CLOSED_ALIAS_LIMIT = 64;

/**
 * Bytes charged for a held Object on top of its content, so that Objects
 * with no payload still use up `maxHeldSubgroupBytes`.
 */
const HELD_OBJECT_OVERHEAD = 64;
const HELD_PROPERTY_OVERHEAD = 16;

/**
 * What a Track Alias is to the Objects of its streams, for the rule of
 * `arrivalOf` (`fixtures/early-objects/`, which the Rust implementation reads
 * too: `crates/moqt/src/early.rs`).
 */
export type EarlyAlias = "established" | "draining" | "retired" | "unknown";

/** What becomes of an Object: to its subscription, held, or its stream stopped. */
export type Arrival =
	| { type: "deliver" }
	| { type: "hold" }
	| ({ type: "stop" } & StopReason);

const subscriptionEnded: StopReason = {
	reason: "the subscription ended",
	code: StreamResetErrorCode.CANCELLED,
};
const unknownAlias: StopReason = {
	reason: "unknown Track Alias",
	code: StreamResetErrorCode.CANCELLED,
};

/**
 * What becomes of an Object of an alias in the state `alias`, while
 * `heldBytes` are held for the session and the Object is counted at
 * `objectBytes` (`heldBytesOf`).
 *
 * §11.4.2: "If an endpoint receives a subgroup with an unknown Track Alias,
 * it MAY abandon the stream, or choose to buffer it for a brief period to
 * handle reordering with the control message that establishes the Track
 * Alias." WHY held while a SUBSCRIBE waits for its answer: the SUBSCRIBE_OK
 * and the first Objects come on different streams, and a relay that answers
 * a SUBSCRIBE it held for a publisher (RENDEZVOUS_TIMEOUT) passes that
 * publisher's first Object on at once; abandoned, it is lost. WHY NOT
 * without one waiting: no answer can then give the alias. WHY NOT for a
 * time: the SUBSCRIBE's own wait for its answer bounds it.
 */
export function arrivalOf(
	alias: EarlyAlias,
	subscribePending: boolean,
	heldBytes: number,
	objectBytes: number,
	maxHeldBytes: number,
): Arrival {
	switch (alias) {
		case "established":
		case "draining":
			return { type: "deliver" };
		// §11.1: Objects of an ended subscription are dropped at once.
		case "retired":
			return { type: "stop", ...subscriptionEnded };
		case "unknown":
			if (!subscribePending) return { type: "stop", ...unknownAlias };
			if (heldBytes + objectBytes > maxHeldBytes) {
				return {
					type: "stop",
					reason: "too many Objects before SUBSCRIBE_OK",
					code: StreamResetErrorCode.INTERNAL_ERROR,
				};
			}
			return { type: "hold" };
	}
}

/** What becomes of what is held for an alias once something changed: released to its subscription, kept, or its stream stopped. */
export type Settle =
	| { type: "release" }
	| { type: "keep" }
	| ({ type: "stop" } & StopReason);

/** What becomes of Objects held for an alias that is now in the state `alias`. */
export function settleOf(alias: EarlyAlias, subscribePending: boolean): Settle {
	const arrival = arrivalOf(alias, subscribePending, 0, 0, 0);
	switch (arrival.type) {
		case "deliver":
			return { type: "release" };
		case "hold":
			return { type: "keep" };
		case "stop":
			return arrival;
	}
}

/** The bytes a held Object is counted at against `maxHeldSubgroupBytes`. */
export function heldBytesOf(object: SubgroupObject): number {
	let bytes = HELD_OBJECT_OVERHEAD + object.payload.length;
	for (const { value } of object.properties) {
		bytes += HELD_PROPERTY_OVERHEAD;
		if (typeof value !== "bigint") bytes += value.length;
	}
	return bytes;
}

/** Drops the oldest key of an insertion-ordered collection that grew past `limit`. */
function dropOldestOver<Key>(
	collection: {
		size: number;
		keys(): Iterable<Key>;
		delete(key: Key): boolean;
	},
	limit: number,
): void {
	if (collection.size <= limit) return;
	for (const oldest of collection.keys()) {
		collection.delete(oldest);
		return;
	}
}

export function createIncomingSubgroups(
	options: IncomingSubgroupsOptions,
): IncomingSubgroups {
	const { limits } = options;
	const streams = new Map<number, SubgroupStream>();
	const bindings = new Map<bigint, AliasBinding>();
	/** Track Aliases of ended subscriptions, oldest first. */
	const retired = new Set<bigint>();
	/**
	 * Streams that ended while their Track Alias was still unknown and that
	 * hold Objects. `maxHeldSubgroupBytes` bounds them, as every held Object
	 * counts.
	 */
	let finishedHeld: SubgroupStream[] = [];
	/**
	 * Per unknown Track Alias, the streams that ended and hold nothing: a FIN
	 * without an Object, or a reset. Only their number matters, for the Stream
	 * Count of PUBLISH_DONE (§10.11), so they are a count and not a list: a
	 * peer can open and end such streams without limit while a SUBSCRIBE is
	 * Pending.
	 */
	const earlyClosed = new Map<bigint, bigint>();
	let heldBytes = 0;

	const dropHeld = (stream: SubgroupStream): void => {
		heldBytes -= stream.heldBytes;
		stream.held = [];
		stream.heldBytes = 0;
	};

	const abandonedEvent = (
		stream: SubgroupStream,
		reason: string,
	): SessionOutput => ({
		type: "event",
		event: {
			type: "subgroup-abandoned",
			trackAlias: stream.header?.trackAlias,
			reason,
		},
	});

	const reportIfDrained = (
		binding: AliasBinding,
		out: SessionOutput[],
	): void => {
		if (
			binding.streamsExpected !== undefined &&
			binding.streamsEnded >= binding.streamsExpected
		) {
			options.onDrained(binding.requestId, out);
		}
	};

	/** Counts a routed stream as ended, and reports when PUBLISH_DONE's Stream Count is reached. */
	const countEnded = (stream: SubgroupStream, out: SessionOutput[]): void => {
		if (stream.header === undefined || stream.requestId === undefined) return;
		const binding = bindings.get(stream.header.trackAlias);
		if (binding?.requestId !== stream.requestId) return;
		binding.streamsEnded += 1n;
		reportIfDrained(binding, out);
	};

	/** Counts a stream that ended with nothing held, for an alias no subscription has yet. */
	const countEarlyClosed = (trackAlias: bigint): void => {
		earlyClosed.set(trackAlias, (earlyClosed.get(trackAlias) ?? 0n) + 1n);
		dropOldestOver(earlyClosed, EARLY_CLOSED_ALIAS_LIMIT);
	};

	const retire = (trackAlias: bigint): void => {
		earlyClosed.delete(trackAlias);
		// Deleted first, so that an alias retired again becomes the newest.
		retired.delete(trackAlias);
		retired.add(trackAlias);
		dropOldestOver(retired, RETIRED_ALIAS_LIMIT);
	};

	/** Stops reading a stream the peer still has open and drops what it held. */
	const stop = (
		peerStream: number,
		stream: SubgroupStream,
		code: number,
		out: SessionOutput[],
	): void => {
		dropHeld(stream);
		stream.abandoned = true;
		out.push({ type: "stop-sending", peerStream, code });
	};

	/** Stops a stream whose Objects are lost to the application, and says so. */
	const abandon = (
		peerStream: number,
		stream: SubgroupStream,
		{ reason, code }: StopReason,
		out: SessionOutput[],
	): void => {
		stop(peerStream, stream, code, out);
		out.push(abandonedEvent(stream, reason));
		countEnded(stream, out);
	};

	/**
	 * WHY INTERNAL_ERROR: no code of §3.3.3 is for a stream that is kept open
	 * without bytes. DELIVERY_TIMEOUT is for "A delivery timeout (Section 8)",
	 * which is a time the publisher keeps for an Object, and EXCESSIVE_LOAD
	 * says "The endpoint is overloaded", which it is not. INTERNAL_ERROR is
	 * "An implementation specific error", and the code a stream that does not
	 * say what it is gets (`peerStreamTimedOut` in session.ts); the Rust
	 * implementation sends the same (docs/moqt-implementation-notes.md).
	 */
	const idleStream: StopReason = {
		reason: "no bytes within the idle time",
		code: StreamResetErrorCode.INTERNAL_ERROR,
	};

	/** What a Track Alias is now, for `arrivalOf`; a draining one is bound as an established one is. */
	const earlyAlias = (trackAlias: bigint): EarlyAlias =>
		bindings.has(trackAlias)
			? "established"
			: retired.has(trackAlias)
				? "retired"
				: "unknown";

	/** Returns why to abandon the stream, or nothing when it can go on. */
	const onHeader = (
		stream: SubgroupStream,
		header: SubgroupHeader,
	): StopReason | undefined => {
		stream.header = header;
		if (header.subgroupId.mode === "Zero") stream.subgroupId = 0n;
		if (header.subgroupId.mode === "Explicit") {
			stream.subgroupId = header.subgroupId.value;
		}
		stream.requestId = bindings.get(header.trackAlias)?.requestId;
		const arrival = arrivalOf(
			earlyAlias(header.trackAlias),
			options.hasPendingSubscribe(),
			0,
			0,
			limits.maxHeldSubgroupBytes,
		);
		return arrival.type === "stop" ? arrival : undefined;
	};

	/** Delivers the Object, or holds it. Returns why to abandon the stream when holding it went over the limit. */
	const onObject = (
		stream: SubgroupStream,
		object: SubgroupObject,
		out: SessionOutput[],
	): StopReason | undefined => {
		const { header } = stream;
		if (header === undefined) {
			throw new Error("the decoder produced an Object before the header");
		}
		// SUBGROUP_ID_MODE 0b01 (§11.4.2)
		stream.subgroupId ??= object.objectId;
		const event: HeldObject = {
			type: "object",
			trackAlias: header.trackAlias,
			groupId: header.groupId,
			subgroupId: stream.subgroupId,
			publisherPriority: header.publisherPriority,
			object,
		};
		// A stream is stopped when its subscription ends (`release`), so one that
		// still delivers belongs to a live subscription.
		if (stream.requestId !== undefined) {
			out.push({
				type: "event",
				event: { ...event, requestId: stream.requestId },
			});
			return undefined;
		}
		const bytes = heldBytesOf(object);
		const arrival = arrivalOf(
			"unknown",
			options.hasPendingSubscribe(),
			heldBytes,
			bytes,
			limits.maxHeldSubgroupBytes,
		);
		if (arrival.type === "stop") return arrival;
		stream.held.push(event);
		stream.heldBytes += bytes;
		heldBytes += bytes;
		return undefined;
	};

	/** Delivers what a stream held if its Track Alias is established now. */
	const deliverHeld = (
		stream: SubgroupStream,
		out: SessionOutput[],
	): boolean => {
		if (stream.header === undefined) return false;
		const requestId = bindings.get(stream.header.trackAlias)?.requestId;
		if (requestId === undefined) return false;
		stream.requestId = requestId;
		for (const event of stream.held) {
			out.push({ type: "event", event: { ...event, requestId } });
		}
		dropHeld(stream);
		return true;
	};

	const release = (
		trackAlias: bigint,
		code: number,
		out: SessionOutput[],
	): void => {
		const binding = bindings.get(trackAlias);
		if (binding === undefined) return;
		bindings.delete(trackAlias);
		retire(trackAlias);
		// §10.11: "The subscriber SHOULD send STOP_SENDING on all streams related
		// to the subscription when it deletes subscription state." Otherwise each
		// stream would go on collecting a payload nobody will read.
		for (const [peerStream, stream] of streams) {
			if (stream.requestId === binding.requestId && !stream.abandoned) {
				stop(peerStream, stream, code, out);
			}
		}
	};

	return {
		aliasState(trackAlias) {
			const binding = bindings.get(trackAlias);
			if (binding === undefined) return "free";
			return binding.draining ? "draining" : "established";
		},

		establish(trackAlias, requestId, maxObjectPayloadLength, out) {
			const existing = bindings.get(trackAlias);
			if (existing !== undefined) {
				options.onDrained(existing.requestId, out);
				release(trackAlias, StreamResetErrorCode.CANCELLED, out);
			}
			retired.delete(trackAlias);
			bindings.set(trackAlias, {
				requestId,
				maxObjectPayloadLength,
				// Streams that ended before this SUBSCRIBE_OK count too (§10.11).
				streamsEnded: earlyClosed.get(trackAlias) ?? 0n,
				draining: false,
				streamsExpected: undefined,
			});
			earlyClosed.delete(trackAlias);
		},

		retire,

		expectStreams(trackAlias, streamCount, out) {
			const binding = bindings.get(trackAlias);
			if (binding === undefined) return;
			binding.draining = true;
			// Without an exact count only the caller's timer ends the wait.
			if (streamCount === STREAM_COUNT_UNKNOWN) return;
			binding.streamsExpected = streamCount;
			reportIfDrained(binding, out);
		},

		release,

		open(peerStream) {
			streams.set(peerStream, {
				// WHY looked up for each Object and not once for the stream: the
				// stream is opened before its header says its Track Alias, and the
				// alias can be bound after that, by a SUBSCRIBE_OK that comes
				// while the stream is read. Until it is, the connection's limit
				// holds. WHY NOT also a check of the subscription's limit on the
				// Objects taken before: they are already collected, and the ones
				// held are bounded by `maxHeldSubgroupBytes`, so refusing them
				// would hold no less; the limit is there to stop collecting a
				// payload, which only a check where its length is read does.
				decoder: createSubgroupStreamDecoder(
					limits,
					({ trackAlias }) =>
						bindings.get(trackAlias)?.maxObjectPayloadLength ??
						limits.maxObjectPayloadLength,
				),
				header: undefined,
				subgroupId: undefined,
				requestId: undefined,
				held: [],
				heldBytes: 0,
				abandoned: false,
			});
		},

		receive(peerStream, data, fin, out) {
			const stream = streams.get(peerStream);
			if (stream === undefined) return;
			if (fin) streams.delete(peerStream);
			if (stream.abandoned) return;

			const result = stream.decoder.push(data);
			const { failure } = result;
			// Checked before the items, so that a broken rule in this chunk closes
			// the session wherever the chunk happened to be cut.
			if (failure?.scope === "session") throw failure.error;

			for (const item of result.items) {
				if (item.type === "header") {
					out.push({ type: "peer-stream-identified", kind: "uni", peerStream });
				}
				const stopReason =
					item.type === "header"
						? onHeader(stream, item.header)
						: onObject(stream, item.object, out);
				if (stopReason !== undefined) {
					abandon(peerStream, stream, stopReason, out);
					return;
				}
				// Not before the header is whole: until then the time a stream
				// has to say what it is runs, and it is the shorter one by
				// default (10 against 30 seconds).
				if (item.type === "header" && !fin) {
					out.push({ type: "watch-peer-stream-idle", peerStream });
				}
				// Delivering an Object can end the subscription and stop this stream.
				if (stream.abandoned) return;
			}
			if (failure !== undefined) {
				const { requestId } = stream;
				const malformed = failure.scope === "track";
				abandon(
					peerStream,
					stream,
					{
						reason: failure.reason,
						code: malformed
							? StreamResetErrorCode.MALFORMED_TRACK
							: StreamResetErrorCode.INTERNAL_ERROR,
					},
					out,
				);
				if (malformed && requestId !== undefined) {
					options.onMalformedTrack(requestId, out);
				}
				return;
			}
			if (!fin) return;
			const finError = stream.decoder.finish();
			if (finError !== undefined) throw finError;
			const { header } = stream;
			if (header === undefined) return;
			// A stream that ended while its Track Alias is unknown still counts for
			// PUBLISH_DONE (§10.11) once a SUBSCRIBE_OK establishes the alias.
			if (stream.requestId !== undefined) countEnded(stream, out);
			else if (stream.held.length > 0) finishedHeld.push(stream);
			else countEarlyClosed(header.trackAlias);
		},

		idle(peerStream, out) {
			const stream = streams.get(peerStream);
			if (stream === undefined || stream.abandoned) return;
			if (stream.header === undefined) return;
			abandon(peerStream, stream, idleStream, out);
		},

		dropBeforeHeader(peerStream) {
			const stream = streams.get(peerStream);
			if (stream === undefined || stream.abandoned) return false;
			if (stream.header !== undefined) return false;
			streams.delete(peerStream);
			return true;
		},

		reset(peerStream, out) {
			const stream = streams.get(peerStream);
			if (stream === undefined) return;
			streams.delete(peerStream);
			if (stream.abandoned) return;
			dropHeld(stream);
			if (stream.requestId !== undefined) countEnded(stream, out);
			else if (stream.header !== undefined) {
				countEarlyClosed(stream.header.trackAlias);
			}
		},

		resolveHeld(out) {
			const stillPending = options.hasPendingSubscribe();
			for (const [peerStream, stream] of streams) {
				if (stream.abandoned || stream.requestId !== undefined) continue;
				if (stream.header === undefined) continue;
				if (deliverHeld(stream, out)) continue;
				const settle = settleOf(
					earlyAlias(stream.header.trackAlias),
					stillPending,
				);
				if (settle.type === "stop") abandon(peerStream, stream, settle, out);
			}
			const finished = finishedHeld;
			finishedHeld = [];
			for (const stream of finished) {
				if (deliverHeld(stream, out)) {
					countEnded(stream, out);
				} else if (stillPending) {
					finishedHeld.push(stream);
				} else {
					dropHeld(stream);
					out.push(abandonedEvent(stream, unknownAlias.reason));
				}
			}
			// With no SUBSCRIBE Pending, nothing can claim the counted streams.
			if (!stillPending) earlyClosed.clear();
		},

		clear() {
			streams.clear();
			bindings.clear();
			retired.clear();
			earlyClosed.clear();
			finishedHeld = [];
			heldBytes = 0;
		},
	};
}
