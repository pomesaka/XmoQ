// Publishes the catalog track of a broadcast over a connection of
// @xmoq/moqt (draft-ietf-moq-msf-01 §5). Section numbers are of that draft
// unless they say "transport", which is draft-ietf-moq-transport-18.

import {
	type PeerFetch,
	type PeerSubscribe,
	type PublishedTrack,
	RequestErrorCode,
	SubgroupClosedError,
	type SubgroupWriter,
	type SubscribeAnswer,
	type TrackNamespace,
} from "@xmoq/moqt";
import {
	DEFAULT_END_TIMEOUT_MS,
	endedByThePeer,
	publishTrack,
	tellEnd,
	wholeNumber,
} from "@xmoq/tracks";
import type { Catalog } from "../core/catalog.ts";
import {
	answerCatalogFetch,
	type CatalogPublication,
	firstPublication,
	nextPublication,
} from "../core/catalog-publication.ts";
import type { CatalogLocation } from "../core/catalog-track.ts";

/** §5: "The catalog track MUST have a case-sensitive Track Name of "catalog"." */
const CATALOG_TRACK_NAME = new TextEncoder().encode("catalog");

/**
 * The Publisher Priority of the catalog Object on a fetch stream, where an
 * Object has to have one; the middle of 0 to 255. On a subgroup stream none
 * is given.
 */
const FETCH_PUBLISHER_PRIORITY = 128;

export interface CatalogPublisherOptions {
	/** The namespace of the catalog track: the one the broadcast is in. */
	namespace: TrackNamespace;
	/** The first catalog. */
	catalog: Catalog;
	/**
	 * The Group of the first catalog; the time in milliseconds since the
	 * Unix epoch when left out, which is what §6.1 suggests for a publisher
	 * that does not know what it published before. Every catalog after the
	 * first is in the next Group.
	 *
	 * A page that is loaded again has to start above where it was (§6.1,
	 * MUST). With the time, it does while the clock does not go back; nothing
	 * here can tell when it did.
	 */
	firstGroup?: bigint;
	/**
	 * How long `end` waits for a subscriber to take the catalog that is
	 * being written to it, in milliseconds; `DEFAULT_END_TIMEOUT_MS` of
	 * `@xmoq/tracks` (3000) when left out. A whole number from 0 to
	 * 2^31 - 1, or a `RangeError`.
	 */
	endTimeoutMs?: number;
}

/**
 * The publisher of one catalog track. It keeps the newest catalog, answers
 * the SUBSCRIBE and the FETCH of a peer for the track, and sends every
 * update to the subscriptions it accepted.
 *
 * It takes no connection. The application announces the namespace, and
 * hands the requests of the peer on from the one pair of handlers a
 * connection has:
 *
 *     connection.serve({
 *       subscribe: (request) => {
 *         if (!catalog.subscribe(request)) media.subscribe(request);
 *       },
 *       fetch: (request) => {
 *         if (!catalog.fetch(request)) media.fetch(request);
 *       },
 *     });
 *
 * §11.2: "An MSF publisher MUST publish a catalog track object before
 * publishing any media track objects." The publisher has its catalog from
 * the moment it is made: make it before a media Object is sent.
 */
export interface CatalogPublisher {
	/**
	 * Takes a SUBSCRIBE of the peer when it is for this catalog track, and
	 * says whether it was. One that is taken is accepted with the Location of
	 * the newest catalog as its LARGEST_OBJECT, and gets every catalog after
	 * that; the newest itself is not sent again, a subscriber gets it with a
	 * Joining FETCH (§5). `moqlivemock`'s publisher sends it on the
	 * subscription as well, "for backward compatibility with subscribe-only
	 * clients"; that is not done here.
	 *
	 * A subscription with FORWARD 0 is accepted and sent nothing (transport
	 * §5.1: "The publisher does not send Objects if the Forward State is
	 * 0"). The filter of the SUBSCRIBE is not looked at.
	 */
	subscribe(request: PeerSubscribe): boolean;
	/**
	 * Takes a FETCH of the peer when it is for this catalog track, and says
	 * whether it was. `answerCatalogFetch` has what it is answered with.
	 */
	fetch(request: PeerFetch): boolean;
	/**
	 * Publishes a catalog that replaces the one before, as Object 0 of the
	 * next Group, on a stream of its own for every subscription. Throws a
	 * `RangeError`, and publishes nothing, for a catalog that may not follow
	 * the one before (`nextPublication`), and after `end`.
	 *
	 * A subscriber that is slow to take a catalog gets the newest one next:
	 * the ones between are not sent to it, and at most one waits for it.
	 */
	update(catalog: Catalog): void;
	/**
	 * The catalog that is published, as a reader gets it, and where. A copy
	 * each time: changing it changes nothing of what is published, and it
	 * can be given to `update`.
	 */
	readonly current: { readonly catalog: Catalog; readonly at: CatalogLocation };
	/**
	 * Ends the track: every subscription gets the newest catalog, where it
	 * does not have it yet, and then PUBLISH_DONE (TRACK_ENDED). So the last
	 * catalog of a broadcast can be published and the track ended right
	 * after (§11.3 has a publisher end with a catalog that says so).
	 *
	 * A subscriber that does not take that catalog within `endTimeoutMs`
	 * has the stream of it reset, and gets its PUBLISH_DONE all the same.
	 * One whose stream does not even open within that time again (it grants
	 * none) is not waited for, which is reported with `reportError`; it is
	 * told when the stream has opened after all. How long a PUBLISH_DONE
	 * takes is not counted.
	 * A request for the track that comes after `end` was called is refused
	 * with DOES_NOT_EXIST. Resolves when the subscriptions were told; never
	 * rejects. A second call waits for the first.
	 */
	end(): Promise<void>;
}

interface Subscriber {
	track: PublishedTrack;
	/** The Group of the newest catalog the subscriber has, or was told of. */
	sentGroup: bigint;
	/** Whether the loop that writes catalogs to it is running. */
	busy: boolean;
	/** Settles when the loop that ran last, or runs, has ended. */
	writing: Promise<void>;
	/** The stream a catalog is being written to. */
	stream: SubgroupWriter | undefined;
}

export function createCatalogPublisher(
	options: CatalogPublisherOptions,
): CatalogPublisher {
	let publication: CatalogPublication = firstPublication(
		options.catalog,
		options.firstGroup ?? BigInt(Date.now()),
		options.namespace,
	);
	const endTimeoutMs = wholeNumber(
		"endTimeoutMs",
		options.endTimeoutMs ?? DEFAULT_END_TIMEOUT_MS,
		0,
	);
	const subscribers = new Set<Subscriber>();
	/** The subscriptions with FORWARD 0: nothing is sent on them but PUBLISH_DONE. */
	const silent = new Set<PublishedTrack>();
	/** Set by `end`, which this settles with. */
	let ending: Promise<void> | undefined;

	/**
	 * A failure that is nobody's doing on the other side: it is reported
	 * like an error of a handler of `MoqtConnection.serve`, which this stands
	 * in for.
	 */
	const fault = (error: unknown): void => {
		reportError(error);
	};

	/** Writes the newest catalog to a subscriber until it has it. */
	const write = async (subscriber: Subscriber): Promise<void> => {
		try {
			// The newest when the turn comes: what was published meanwhile and
			// replaced is not written.
			while (
				subscribers.has(subscriber) &&
				subscriber.sentGroup < publication.at.group
			) {
				const { at, payload } = publication;
				try {
					subscriber.stream = await subscriber.track.openSubgroup({
						groupId: at.group,
						// §5: "All catalog updates ... MUST be mapped to MOQT
						// sub-group 0."
						subgroupId: { mode: "Zero" },
						properties: false,
						// The Group is this one Object.
						endOfGroup: true,
					});
					// `end` gave the subscriber up while the stream was opened.
					if (!subscribers.has(subscriber)) {
						await subscriber.stream.reset().catch(() => {});
						return;
					}
					await subscriber.stream.write({ objectId: at.object, payload });
					await subscriber.stream.close();
				} catch (error) {
					// The subscription is over: `track.ended` takes the subscriber
					// away, and the loop ends with that.
					if (endedByThePeer(error)) return;
					// WHY the stream is reset for anything else: a stream that is
					// neither closed nor reset keeps PUBLISH_DONE from being sent.
					// A reset of one that is closed already does nothing.
					await subscriber.stream?.reset().catch(() => {});
					// The subscriber stopped this one stream, or `end` gave it up:
					// the catalog of this Group is not sent to it again, and the
					// next one goes on a stream of its own. Anything else is a
					// fault.
					if (!(error instanceof SubgroupClosedError)) fault(error);
				}
				subscriber.stream = undefined;
				subscriber.sentGroup = at.group;
			}
		} finally {
			// WHY in this function, and not when its promise has settled: this
			// runs in the same turn as the loop's last look at the newest
			// catalog. A turn later, an `update` between the two would find
			// the loop still marked as running, and its catalog would not be
			// written.
			subscriber.busy = false;
		}
	};

	/** Has the newest catalog written to a subscriber that does not have it, unless that is under way. */
	const send = (subscriber: Subscriber): void => {
		// WHY not chained onto the write that is under way: the loop takes
		// the newest catalog when it comes round, so a subscriber that takes
		// none has one write waiting, however many updates there are.
		if (subscriber.busy) return;
		subscriber.busy = true;
		subscriber.writing = write(subscriber);
	};

	/** Resolves with "late" after the time `end` gives a subscriber. */
	const late = (): { over: Promise<"late">; stop: () => void } => {
		let timer: ReturnType<typeof setTimeout> | undefined;
		const over = new Promise<"late">((resolve) => {
			timer = setTimeout(() => resolve("late"), endTimeoutMs);
		});
		return { over, stop: () => clearTimeout(timer) };
	};

	/** Ends a subscription once it has the newest catalog, or its time for that is over. */
	const tell = async (subscriber: Subscriber): Promise<void> => {
		const done = (): Promise<void> => tellEnd(subscriber.track);
		send(subscriber);
		const first = late();
		const outcome = await Promise.race([subscriber.writing, first.over]);
		first.stop();
		// Before the reset: the loop ends when it finds the subscriber gone,
		// and does not begin another catalog.
		subscribers.delete(subscriber);
		if (outcome !== "late") {
			await done();
			return;
		}
		// The write that is stuck fails with this.
		await subscriber.stream?.reset().catch(() => {});
		// WHY a second time, and for the write alone: a stream that could not
		// even be opened (the subscriber grants none) has nothing to reset.
		// How long PUBLISH_DONE takes is not counted: `done` has a time of
		// its own for a subscriber that does not end the request stream.
		const second = late();
		const written = await Promise.race([subscriber.writing, second.over]);
		second.stop();
		if (written === "late") {
			fault(
				new Error(
					"a subscription of the catalog track could not be told of its end in time: its stream does not open",
				),
			);
			// Told when the stream has opened after all, and was reset.
			void subscriber.writing.then(done);
			return;
		}
		await done();
	};

	/** A subscription that is sent catalogs, from the Group its SUBSCRIBE_OK said. */
	const subscriberOf = (
		track: PublishedTrack,
		answer: SubscribeAnswer,
	): Subscriber => ({
		track,
		sentGroup: answer.parameters?.largestObject?.group ?? publication.at.group,
		busy: false,
		writing: Promise.resolve(),
		stream: undefined,
	});

	const published = publishTrack({
		trackNamespace: options.namespace,
		trackName: CATALOG_TRACK_NAME,
		subscribers: "many",
		answer: () => ({ parameters: { largestObject: publication.at } }),
		subscribed(track, request, answer) {
			// Transport §5.1: "The publisher does not send Objects if the
			// Forward State is 0". The subscription is kept for its
			// PUBLISH_DONE alone.
			if (request.parameters.forward === false) {
				silent.add(track);
				void track.ended.then(() => silent.delete(track));
				return;
			}
			const subscriber = subscriberOf(track, answer);
			subscribers.add(subscriber);
			void track.ended.then(() => subscribers.delete(subscriber));
			// A catalog that was published while the answer was on its way.
			send(subscriber);
		},
		subscribedLate(track, request, answer) {
			if (request.parameters.forward === false) {
				void tellEnd(track);
				return;
			}
			// The track ended while the answer was on its way: the subscription
			// gets a catalog that was published in that time, and its
			// PUBLISH_DONE. `end` does not wait for it.
			const subscriber = subscriberOf(track, answer);
			subscribers.add(subscriber);
			void tell(subscriber);
		},
		fetch(request) {
			// What is published now: a catalog that replaces it while the
			// stream is written does not change the answer.
			const { at, payload } = publication;
			const answer = answerCatalogFetch(publication, request);
			if (answer.type === "invalid-range") {
				request.reject(RequestErrorCode.INVALID_RANGE).catch(() => {});
				return;
			}
			request.accept({ endLocation: answer.endLocation }).then(
				async (writer) => {
					try {
						for (const piece of answer.pieces) {
							if (piece === "catalog") {
								await writer.write({
									groupId: at.group,
									subgroupId: 0n,
									objectId: at.object,
									publisherPriority: FETCH_PUBLISHER_PRIORITY,
									properties: [],
									payload,
								});
							} else {
								await writer.endOfRange("Unknown", piece.unknownThrough);
							}
						}
						await writer.close();
					} catch (error) {
						// The subscriber gave the FETCH up.
						if (endedByThePeer(error)) return;
						// Transport §5.2: a FETCH that is not served to its end is
						// given up by resetting both of its streams.
						await writer.reset().catch(() => {});
						fault(error);
					}
				},
				(error: unknown) => {
					if (request.signal.aborted) return;
					request.reject(RequestErrorCode.INTERNAL_ERROR).catch(() => {});
					fault(error);
				},
			);
		},
	});

	return {
		subscribe: (request) => published.subscribe(request),
		fetch: (request) => published.fetch(request),

		update(catalog) {
			if (published.ended) {
				throw new RangeError("the catalog track has ended");
			}
			publication = nextPublication(publication, catalog);
			for (const subscriber of subscribers) send(subscriber);
		},

		get current() {
			// A copy: the catalog the rules are kept against is not the
			// caller's to change.
			return {
				catalog: structuredClone(publication.catalog),
				at: { ...publication.at },
			};
		},

		end() {
			ending ??= (async () => {
				// The answers on their way are not waited for: each gets the
				// last catalog and its PUBLISH_DONE when it comes
				// (`subscribedLate`).
				void published.end();
				const told = [...subscribers].map(tell);
				const quiet = [...silent].map(tellEnd);
				silent.clear();
				await Promise.all([...told, ...quiet]);
			})();
			return ending;
		},
	};
}
