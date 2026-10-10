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
	 */
	establish(trackAlias: bigint, requestId: bigint, out: SessionOutput[]): void;
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

function heldBytesOf(object: SubgroupObject): number {
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

	const subscriptionEnded: StopReason = {
		reason: "the subscription ended",
		code: StreamResetErrorCode.CANCELLED,
	};
	const unknownAlias: StopReason = {
		reason: "unknown Track Alias",
		code: StreamResetErrorCode.CANCELLED,
	};

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
		if (stream.requestId !== undefined) return undefined;
		// §11.1: Objects of an ended subscription are dropped at once.
		if (retired.has(header.trackAlias)) return subscriptionEnded;
		// With no SUBSCRIBE Pending, no SUBSCRIBE_OK can establish this alias.
		if (!options.hasPendingSubscribe()) return unknownAlias;
		return undefined;
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
		stream.held.push(event);
		stream.heldBytes += bytes;
		heldBytes += bytes;
		if (heldBytes > limits.maxHeldSubgroupBytes) {
			return {
				reason: "too many Objects before SUBSCRIBE_OK",
				code: StreamResetErrorCode.INTERNAL_ERROR,
			};
		}
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

		establish(trackAlias, requestId, out) {
			const existing = bindings.get(trackAlias);
			if (existing !== undefined) {
				options.onDrained(existing.requestId, out);
				release(trackAlias, StreamResetErrorCode.CANCELLED, out);
			}
			retired.delete(trackAlias);
			bindings.set(trackAlias, {
				requestId,
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
				decoder: createSubgroupStreamDecoder(limits),
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
				if (retired.has(stream.header.trackAlias)) {
					abandon(peerStream, stream, subscriptionEnded, out);
				} else if (!stillPending) {
					abandon(peerStream, stream, unknownAlias, out);
				}
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
