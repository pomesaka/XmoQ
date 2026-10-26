// A LOC video track published over a connection: the chunks of an encoder
// go through the sender of this package (`src/core/video-sender.ts`), and
// what it says is written to the subscription a relay made. No WebCodecs:
// a chunk comes in as numbers and bytes.

import {
	LONGEST_TIMER_MS,
	type PeerFetch,
	type PeerSubscribe,
	PublishDoneCode,
	PublishEndedError,
	type PublishedTrack,
	RequestErrorCode,
	SessionClosedError,
	type SubgroupWriter,
	sameFullTrackName,
} from "@xmoq/moqt";
import {
	createVideoSender,
	DEFAULT_MAX_PAYLOAD_LENGTH,
	type VideoChunk,
	type VideoSenderStep,
	type VideoSkipReason,
	type VideoStreamMapping,
} from "../core/video-sender.ts";

/** The Track Property DYNAMIC_GROUPS (transport §12.6). */
const DYNAMIC_GROUPS = 0x30n;

/**
 * How long a Group lasts at least before a NEW_GROUP_REQUEST begins the
 * next one, in milliseconds of the chunks' time. Decided on 2026-10-21.
 * Transport §10.2.13 lets the publisher delay a request "for example,
 * achieving a minimum duration for each Group". WHY a least
 * length at all: every new Group costs a key frame, and viewers that come
 * one after the other would else have one each.
 */
export const DEFAULT_SHORTEST_GROUP_MS = 500;

/**
 * How many Objects may wait to be written before the Group is cut short.
 * **Provisional, not measured**: what the receiver of this package holds,
 * and well below what the relay of this repository lets wait for one
 * subscriber (256 streams, 4,096 Objects).
 */
export const DEFAULT_MAX_PENDING_OBJECTS = 32;
/** How many bytes of payload may wait to be written. **Provisional, not measured**: the relay's limit for one subscriber. */
export const DEFAULT_MAX_PENDING_BYTES = 4 * 1024 * 1024;
/**
 * How long `end` waits for what is queued to be written, in milliseconds.
 * Provisional: the time `@xmoq/moqt` gives a peer to end a request stream.
 */
export const DEFAULT_END_TIMEOUT_MS = 3000;

export interface VideoTrackPublisherOptions {
	trackNamespace: readonly Uint8Array[];
	trackName: Uint8Array;
	/** The Group of the first key chunk (`VideoSenderOptions.firstGroup`). */
	firstGroup: bigint;
	/** The wall-clock time of the moment the encoder counts from (`VideoSenderOptions.epoch`). */
	epoch: bigint;
	/** How the Objects go onto streams; the sender's default when left out (`VideoSenderOptions.mapping`). */
	mapping?: VideoStreamMapping;
	/** The longest payload that is sent (`VideoSenderOptions.maxPayloadLength`). */
	maxPayloadLength?: number;
	/** `DEFAULT_MAX_PENDING_OBJECTS` when left out. */
	maxPendingObjects?: number;
	/** `DEFAULT_MAX_PENDING_BYTES` when left out. Not below the longest payload that is sent. */
	maxPendingBytes?: number;
	/** How long `end` waits for what is queued to be written; 3000 ms when left out. */
	endTimeoutMs?: number;
	/**
	 * With it, the SUBSCRIBE_OK says DYNAMIC_GROUPS = 1 (transport §12.6),
	 * and a NEW_GROUP_REQUEST of the subscriber (§10.2.13) makes `push` ask
	 * for a key chunk, which begins a Group: once the Group that is being
	 * sent is `shortestGroupMs` long (`DEFAULT_SHORTEST_GROUP_MS` when left
	 * out). **Left out, the track has no dynamic Groups** and a request is
	 * taken without effect, as §10.2.13 has it of such a publisher.
	 */
	dynamicGroups?: { shortestGroupMs?: number };
}

/** Why a chunk was not sent, besides the reasons of the sender. */
export type VideoPushSkipReason =
	| VideoSkipReason
	/** Nobody is subscribed, or the subscriber came after the key chunk of this Group. */
	| "no-subscriber"
	/** More waits to be written than may: the Group is cut short. */
	| "behind";

export interface VideoPushResult {
	/** Why the chunk is not sent; absent for one that is. */
	skipped?: VideoPushSkipReason;
	/**
	 * Whether the encoder is to make its next chunk a key chunk: a Group
	 * was cut short, and nothing is sent before the next key chunk. True
	 * once for each time it is needed, and not again before a key chunk
	 * came and what waited was written down to half of what may wait: a
	 * subscriber that stays behind does not get a key frame every frame.
	 *
	 * True as well, once, with the first chunk after a subscriber came that
	 * is no key chunk: the subscriber gets nothing before one. And once for
	 * a NEW_GROUP_REQUEST, with `dynamicGroups`, when the Group is long
	 * enough.
	 *
	 * **Never true while nobody is subscribed**: a key frame would be for
	 * nobody. So it is once for each subscription at least, where a Group
	 * was cut before the subscriber came.
	 */
	askForKeyFrame: boolean;
}

export interface VideoTrackPublisherStats {
	/** Objects handed to the transport. */
	written: number;
	/** Chunks that were not sent, by the reason. */
	skipped: Partial<Record<VideoPushSkipReason, number>>;
	/** Times a write failed for another reason than that the subscriber went. */
	writeFailures: number;
	/** Objects that wait to be written now. */
	pendingObjects: number;
	/** NEW_GROUP_REQUESTs of the subscriber that asked for a Group after the one being sent. */
	newGroupsAskedFor: number;
	/** Of them, those for which a key chunk was asked for; the others were met by a key chunk that came first. */
	newGroupsBegun: number;
}

export interface VideoTrackPublisher {
	/**
	 * For the `subscribe` handler of `MoqtConnection.serve`. False for
	 * another track. **One subscription at a time**: a relay makes one for
	 * all its viewers. A second one while the first is open is refused with
	 * DUPLICATE_SUBSCRIPTION; a relay that comes again after its
	 * subscription ended is accepted.
	 *
	 * The subscriber gets nothing of the Group that is being sent when it
	 * comes: its first Object is the next key chunk, which `push` asks for.
	 */
	subscribe(request: PeerSubscribe): boolean;
	/**
	 * For the `fetch` handler. False for another track. A FETCH for this
	 * track is refused with NOT_SUPPORTED: no frame is kept here.
	 */
	fetch(request: PeerFetch): boolean;
	/** Takes the next chunk of the encoder. Does not wait for the write, and does not throw. */
	push(chunk: VideoChunk): VideoPushResult;
	/**
	 * Ends the track: what is queued is written, and the subscription gets
	 * PUBLISH_DONE (TRACK_ENDED). A SUBSCRIBE after it is refused with
	 * DOES_NOT_EXIST, and a chunk is not sent. Never rejects.
	 *
	 * **How long it takes at most**: `endTimeoutMs` for what is queued. What
	 * is not written by then is not written: its stream is reset, and what
	 * is under way then has `endTimeoutMs` once more to end. After that
	 * `PublishedTrack.done` of `@xmoq/moqt` waits for the subscriber to end
	 * its direction of the request stream, 3 seconds at most. A SUBSCRIBE
	 * that is being answered when `end` is called gets its PUBLISH_DONE once
	 * the answer is out, and `end` waits `endTimeoutMs` at most for that.
	 *
	 * **Where a stream could not be opened or reset in that time,
	 * PUBLISH_DONE is not sent** (transport §10.11 has it wait for the
	 * streams), and the failure is reported with `reportError`.
	 */
	end(): Promise<void>;
	/** What was counted so far; a copy. */
	stats(): VideoTrackPublisherStats;
}

/** Whether a call on the subscription failed because the subscriber ended it, or the session is over. */
const endedByThePeer = (error: unknown): boolean =>
	error instanceof PublishEndedError || error instanceof SessionClosedError;

/** A subgroup stream, and the Group it is of. */
interface OpenStream {
	groupId: bigint;
	writer: SubgroupWriter;
}

interface Subscriber {
	track: PublishedTrack;
	/** Whether it has been sent a key chunk: nothing goes to it before one. */
	started: boolean;
	/** Whether the encoder was asked for the key chunk it starts with. */
	askedToStart: boolean;
	/**
	 * FORWARD 0 (transport §5.1): nothing is sent, and it is told of the end.
	 * As the SUBSCRIBE had it: `PublishedTrack` has its parameters and no
	 * later ones.
	 */
	silent: boolean;
	/**
	 * The stream that was opened for a Group, with "stream-per-group", until
	 * the step that finishes or resets it. It can have been reset before,
	 * by a write that failed.
	 */
	open: OpenStream | undefined;
	/**
	 * The stream the last Object was written to, or is being written to
	 * now. A reset of it does nothing where that write is over and the
	 * stream finished or reset.
	 */
	writing: OpenStream | undefined;
	/**
	 * The Groups that were cut short: what is queued of them is not written.
	 * A Group below the one the queue has come to is forgotten.
	 */
	cut: Set<bigint>;
	/** Objects that wait to be written to it. */
	pendingObjects: number;
	/** Their bytes of payload. */
	pendingBytes: number;
	/** Settles when everything queued for it was written or given up. */
	queue: Promise<void>;
}

/**
 * Publishes a video track to the one subscription it has.
 *
 * - The order on the wire is the order of the chunks: every step of the
 *   sender is queued behind the one before it, so a write waits for the
 *   one before, as `SubgroupWriter` asks.
 * - **A write that fails cuts the Group short**, for whatever reason but
 *   that the subscriber went: the stream is reset, what was queued of the
 *   Group is not written, and nothing is sent before the next key chunk.
 * - **So does a subscriber that does not take what is sent**: when more
 *   Objects or bytes wait than may, the chunk is not sent and the Group
 *   ends there. **What waits of that Group is not written either**, and a
 *   write of it that is under way is given up by resetting its stream. WHY
 *   not written first and cut after: the subscriber is behind by what
 *   waits, and frames of a Group that ends without its last ones would
 *   hold the next key frame back by as much again.
 * - Either way `push` asks for a key chunk, so that the picture comes back
 *   before the encoder would make one by itself.
 *
 * Throws a `RangeError` for options the sender refuses, for a limit that
 * is no whole number from 1, and for `maxPendingBytes` below the longest
 * payload that is sent: a chunk of that length could never be queued.
 */
export function createVideoTrackPublisher(
	options: VideoTrackPublisherOptions,
): VideoTrackPublisher {
	const sender = createVideoSender({
		firstGroup: options.firstGroup,
		epoch: options.epoch,
		...(options.mapping === undefined ? {} : { mapping: options.mapping }),
		...(options.maxPayloadLength === undefined
			? {}
			: { maxPayloadLength: options.maxPayloadLength }),
	});
	const maxPendingObjects =
		options.maxPendingObjects ?? DEFAULT_MAX_PENDING_OBJECTS;
	const maxPendingBytes = options.maxPendingBytes ?? DEFAULT_MAX_PENDING_BYTES;
	const endTimeoutMs = options.endTimeoutMs ?? DEFAULT_END_TIMEOUT_MS;
	const shortestGroupMs =
		options.dynamicGroups === undefined
			? undefined
			: (options.dynamicGroups.shortestGroupMs ?? DEFAULT_SHORTEST_GROUP_MS);
	for (const [name, value, least] of [
		["maxPendingObjects", maxPendingObjects, 1],
		["maxPendingBytes", maxPendingBytes, 1],
		["endTimeoutMs", endTimeoutMs, 0],
		["dynamicGroups.shortestGroupMs", shortestGroupMs ?? 0, 0],
	] as const) {
		if (!Number.isInteger(value) || value < least || value > LONGEST_TIMER_MS) {
			throw new RangeError(
				`${name} is a whole number from ${least} to ${LONGEST_TIMER_MS}, not ${value}`,
			);
		}
	}

	const maxPayloadLength =
		options.maxPayloadLength ?? DEFAULT_MAX_PAYLOAD_LENGTH;
	if (maxPayloadLength > maxPendingBytes) {
		throw new RangeError(
			`maxPendingBytes (${maxPendingBytes}) is not below the longest payload that is sent (${maxPayloadLength})`,
		);
	}

	// WHY what waits is counted for each subscriber, and not here: a write
	// to a subscription that is over can stay under way, and would take from
	// what the next subscriber may have waiting.
	const stats: Omit<VideoTrackPublisherStats, "pendingObjects"> = {
		written: 0,
		skipped: {},
		writeFailures: 0,
		newGroupsAskedFor: 0,
		newGroupsBegun: 0,
	};
	/** The one subscription, while there is one. */
	let subscriber: Subscriber | undefined;
	/** The Location of the last Object the sender made, for LARGEST_OBJECT. */
	let largest: { group: bigint; object: bigint } | undefined;
	/** A Group was cut short, and no key chunk has come since. */
	let needsKey = false;
	/** The encoder was asked for the key chunk that `needsKey` waits for. */
	let asked = false;
	/** The subscriber asked for a new Group, and none has begun since. */
	let newGroupWanted = false;
	/** The time of the key chunk the Group that is being sent began with. */
	let groupBegan: number | undefined;
	/** A SUBSCRIBE is being answered, and has the one place; settles when that is done with. */
	let answer: Promise<void> | undefined;
	let ended: Promise<void> | undefined;

	const isThisTrack = (request: PeerSubscribe | PeerFetch): boolean =>
		sameFullTrackName(request, {
			trackNamespace: options.trackNamespace,
			trackName: options.trackName,
		});

	const skip = (because: VideoPushSkipReason): void => {
		stats.skipped[because] = (stats.skipped[because] ?? 0) + 1;
	};

	/**
	 * Nothing more of the Group that is being sent, until a key chunk; what
	 * is queued of it is not written.
	 */
	const cutGroup = (to: Subscriber): void => {
		// With `needsKey` the sender is in no Group: the last one ended before.
		const group = needsKey ? undefined : largest?.group;
		needsKey = true;
		if (group !== undefined) {
			to.cut.add(group);
			// The write that holds the queue up: it fails with this, and what
			// is queued behind it is passed over.
			if (to.writing?.groupId === group) {
				to.writing.writer.reset().catch(() => {});
			}
		}
		for (const step of sender.endGroup("write-failed")) queueStep(to, step);
	};

	/** PUBLISH_DONE for a subscription; does not reject. */
	const tellEnd = (track: PublishedTrack): Promise<void> =>
		track.done(PublishDoneCode.TRACK_ENDED, "").then(
			() => {},
			(error: unknown) => {
				// `reportError` is the global of HTML for an error nobody is
				// there to take: it goes where an uncaught one goes.
				if (!endedByThePeer(error)) reportError(error);
			},
		);

	/** Does one step of the sender for the subscriber; rejects as the transport does. */
	const write = async (
		to: Subscriber,
		step: VideoSenderStep,
	): Promise<void> => {
		if (step.type === "skipped") return;
		if (step.type === "close" || step.type === "reset") {
			const open = to.open;
			if (open?.groupId !== step.groupId) return;
			to.open = undefined;
			if (step.type === "close") await open.writer.close();
			else await open.writer.reset();
			return;
		}
		// WHY forgotten here: the queue is in the order of the Groups, so
		// nothing of a Group below this one is queued any more, and a track
		// that runs for long would else keep every Group that was ever cut.
		for (const group of to.cut) {
			if (group < step.groupId) to.cut.delete(group);
		}
		// Its stream is not open: the write that was under way on it failed
		// with the reset that cut the Group, or it was never opened.
		if (to.cut.has(step.groupId)) return;
		let writer = to.open?.groupId === step.groupId ? to.open.writer : undefined;
		if (step.opensStream) {
			writer = await to.track.openSubgroup({
				groupId: step.groupId,
				// A stream that is finished after its one Object is the
				// sender's stream for every Object: the Subgroup ID is the
				// Object ID, which the header says without a field
				// (transport §11.4.2). Else it is the stream of a Group,
				// Subgroup 0.
				subgroupId: step.closesStream
					? { mode: "FirstObjectId" }
					: { mode: "Zero" },
				properties: true,
				// WHY never: whether the Group ends with this stream is not
				// known before the next key chunk has come.
				endOfGroup: false,
			});
			// WHY looked at again: the stream counts as open from the call
			// on, and nobody else has its writer. A stream that is left
			// open keeps PUBLISH_DONE from being sent.
			if (subscriber !== to || to.cut.has(step.groupId)) {
				await writer.reset();
				return;
			}
			if (!step.closesStream) to.open = { groupId: step.groupId, writer };
		}
		// The stream of its Group is gone: it could not be opened.
		if (writer === undefined) return;
		to.writing = { groupId: step.groupId, writer };
		try {
			await writer.write({
				objectId: step.objectId,
				payload: step.payload,
				properties: step.properties,
			});
			stats.written += 1;
			if (step.closesStream) await writer.close();
		} catch (error) {
			// A stream that is neither closed nor reset keeps PUBLISH_DONE
			// from being sent; a reset of one that is closed does nothing.
			// Where the subscription is over, its streams are gone with it.
			if (!endedByThePeer(error)) await writer.reset().catch(() => {});
			throw error;
		}
	};

	/** Queues a step behind the ones before it. */
	const queueStep = (to: Subscriber, step: VideoSenderStep): void => {
		const bytes = step.type === "object" ? step.payload.length : 0;
		if (step.type === "object") {
			to.pendingObjects += 1;
			to.pendingBytes += bytes;
		}
		to.queue = to.queue
			.then(() => (subscriber === to ? write(to, step) : undefined))
			.catch((error: unknown) => {
				if (subscriber !== to) return;
				if (endedByThePeer(error)) {
					// `track.ended` takes the subscriber away.
					return;
				}
				// A write of a Group that was cut failed with the reset that
				// gave it up.
				if (step.type === "object" && to.cut.has(step.groupId)) return;
				stats.writeFailures += 1;
				if (step.type !== "object") return;
				to.cut.add(step.groupId);
				// WHY only where the sender is still in that Group: a key
				// chunk that came meanwhile began a Group that is whole.
				if (largest?.group === step.groupId) cutGroup(to);
			})
			.finally(() => {
				if (step.type === "object") {
					to.pendingObjects -= 1;
					to.pendingBytes -= bytes;
				}
			});
	};

	return {
		subscribe(request) {
			if (!isThisTrack(request)) return false;
			if (ended !== undefined) {
				request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				return true;
			}
			if (subscriber !== undefined || answer !== undefined) {
				request
					.reject(
						RequestErrorCode.DUPLICATE_SUBSCRIPTION,
						"the track has its one subscription",
					)
					.catch(() => {});
				return true;
			}
			// From here on, so that a second SUBSCRIBE that comes while this
			// one is answered is refused.
			const answered = request
				.accept({
					// §10.2.11: a publisher that has published Objects of the
					// Track says how far it is.
					...(largest === undefined
						? {}
						: { parameters: { largestObject: largest } }),
					...(shortestGroupMs === undefined
						? {}
						: { trackProperties: [{ type: DYNAMIC_GROUPS, value: 1n }] }),
				})
				.then(
					(track) => {
						// The track ended while the answer was sent.
						if (ended !== undefined) return tellEnd(track);
						const accepted: Subscriber = {
							track,
							started: false,
							askedToStart: false,
							// Transport §5.1: "The publisher does not send Objects
							// if the Forward State is 0".
							silent: request.parameters.forward === false,
							open: undefined,
							writing: undefined,
							cut: new Set(),
							pendingObjects: 0,
							pendingBytes: 0,
							queue: Promise.resolve(),
						};
						subscriber = accepted;
						if (shortestGroupMs !== undefined) {
							track.onNewGroupRequest((group) => {
								if (subscriber !== accepted) return;
								// §10.2.13: "a value of 0 or a value larger than the
								// current Group" asks for a new one; a smaller one
								// was met already.
								const current = needsKey ? undefined : largest?.group;
								if (group !== 0n && current !== undefined && group <= current)
									return;
								stats.newGroupsAskedFor += 1;
								newGroupWanted = true;
							});
						}
						void track.ended.then(() => {
							if (subscriber !== accepted) return;
							subscriber = undefined;
							newGroupWanted = false;
						});
					},
					(error: unknown) => {
						// The request went before it was answered.
						if (request.signal.aborted) return;
						// The answer was not sent, and the request waits for one.
						request.reject(RequestErrorCode.INTERNAL_ERROR).catch(() => {});
						reportError(error);
					},
				)
				.finally(() => {
					answer = undefined;
				});
			answer = answered;
			return true;
		},

		fetch(request) {
			if (!isThisTrack(request)) return false;
			request
				.reject(
					ended === undefined
						? RequestErrorCode.NOT_SUPPORTED
						: RequestErrorCode.DOES_NOT_EXIST,
				)
				.catch(() => {});
			return true;
		},

		push(chunk) {
			if (ended !== undefined) {
				skip("no-subscriber");
				return { skipped: "no-subscriber", askForKeyFrame: false };
			}
			const to = subscriber;
			// More waits than may: this chunk is not sent, and its Group ends.
			const behind =
				to !== undefined &&
				!to.silent &&
				(to.pendingObjects >= maxPendingObjects ||
					to.pendingBytes + chunk.data.length > maxPendingBytes);
			let skipped: VideoPushSkipReason | undefined;
			if (behind) {
				cutGroup(to);
				skipped = "behind";
			} else {
				for (const step of sender.push(chunk)) {
					if (step.type === "skipped") {
						skipped = step.because;
						// A chunk the sender does not send ends its Group as well.
						if (step.because !== "no-key-frame") needsKey = true;
					}
					if (step.type === "object") {
						largest = { group: step.groupId, object: step.objectId };
						if (step.objectId === 0n) {
							needsKey = false;
							asked = false;
							newGroupWanted = false;
							groupBegan = chunk.timestamp;
							if (to !== undefined) to.started = true;
						}
						if (to === undefined || to.silent || !to.started) {
							skipped = "no-subscriber";
							continue;
						}
					}
					if (to !== undefined && !to.silent) queueStep(to, step);
				}
			}
			if (skipped !== undefined) skip(skipped);
			// Asked once, and only when there is room again for what is asked
			// for.
			// Not without a subscriber: the one that comes has a key chunk
			// asked for itself, below.
			const afterACut =
				to !== undefined &&
				!to.silent &&
				needsKey &&
				!asked &&
				(to?.pendingObjects ?? 0) * 2 <= maxPendingObjects &&
				(to?.pendingBytes ?? 0) * 2 <= maxPendingBytes;

			// WHY asked for a subscriber that has nothing yet: it came in the
			// middle of a Group, of which it gets nothing, and would wait for
			// the key chunk the encoder makes by itself, up to its whole
			// interval. Once for a subscription: a relay makes one for all its
			// viewers.
			const toStart =
				to !== undefined && !to.silent && !to.started && !to.askedToStart;
			if (toStart) to.askedToStart = true;
			// WHY not where a Group was cut or the subscriber has not started:
			// a key chunk is asked for then anyway. WHY counted in the chunks'
			// time: it is the length of the Group, however late the encoder.
			const newGroup =
				newGroupWanted &&
				!needsKey &&
				to?.started === true &&
				shortestGroupMs !== undefined &&
				(groupBegan === undefined ||
					chunk.timestamp - groupBegan >= shortestGroupMs * 1000);
			if (newGroup) {
				newGroupWanted = false;
				stats.newGroupsBegun += 1;
			}
			const askForKeyFrame = afterACut || toStart || newGroup;
			// The key chunk that is asked for a subscriber that came is the
			// one a Group that was cut waits for as well.
			if (askForKeyFrame && needsKey) asked = true;
			return skipped === undefined
				? { askForKeyFrame }
				: { skipped, askForKeyFrame };
		},

		end() {
			ended ??= (async () => {
				let timer: ReturnType<typeof setTimeout> | undefined;
				// One timer at a time: the one before has fired, or its wait
				// is over.
				const timeIsOver = (): Promise<"late"> =>
					new Promise((resolve) => {
						clearTimeout(timer);
						timer = setTimeout(() => resolve("late"), endTimeoutMs);
					});
				try {
					const to = subscriber;
					if (to === undefined) {
						// A SUBSCRIBE that is being answered is told of the end
						// when its answer is out.
						if (answer !== undefined) {
							await Promise.race([answer, timeIsOver()]);
						}
						return;
					}
					if (!to.silent) {
						for (const step of sender.endGroup("track-ended"))
							queueStep(to, step);
					}
					if ((await Promise.race([to.queue, timeIsOver()])) === "late") {
						// WHY before anything is waited for: the write that is
						// stuck fails with the reset below, and the step behind
						// it would begin at once, and could open a stream,
						// before the reset has settled here.
						subscriber = undefined;
						// A stream that is neither closed nor reset keeps
						// PUBLISH_DONE from being sent. The stream of the write
						// that stands still is the only one there can be: with
						// a stream for a Group, it is that stream.
						await to.writing?.writer.reset().catch(() => {});
						// What was under way ends with that; an `openSubgroup`
						// that was, with its stream reset when it has come
						// (`write`). WHY waited for: a stream that is opened and
						// not yet reset keeps PUBLISH_DONE from being sent too.
						await Promise.race([to.queue, timeIsOver()]);
					}
					subscriber = undefined;
					await tellEnd(to.track);
				} finally {
					clearTimeout(timer);
				}
			})();
			return ended;
		},

		stats: () => ({
			...stats,
			skipped: { ...stats.skipped },
			pendingObjects: subscriber?.pendingObjects ?? 0,
		}),
	};
}
