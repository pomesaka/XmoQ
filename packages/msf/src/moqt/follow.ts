// Reads the catalog track of a broadcast over a connection of @xmoq/moqt
// (draft-ietf-moq-msf-01 §5). Section numbers are of that draft.

import {
	decodeFetchBody,
	type MoqtConnection,
	type Subscription,
	type SubscriptionEnd,
	type TrackNamespace,
} from "@xmoq/moqt";
import { checkMaxLength, DEFAULT_MAX_CATALOG_LENGTH } from "../core/catalog.ts";
import {
	type CatalogLocation,
	type CatalogState,
	catalogStateAfter,
} from "../core/catalog-track.ts";

/** §5: "The catalog track MUST have a case-sensitive Track Name of "catalog"." */
const CATALOG_TRACK_NAME = new TextEncoder().encode("catalog");

/**
 * How long the Joining FETCH has, from when it is sent to the end of its
 * fetch stream, when no other time is given. Provisional.
 *
 * WHY a time: a FETCH that gets no answer stays open (`Fetch` of
 * @xmoq/moqt), and the Objects of the subscription wait behind it here.
 * WHY 15 seconds: longer than the own relay waits for the publisher it
 * asks, 5 seconds for the answer and 5 for the fetch stream to open
 * (docs/moqt-implementation-notes.md, the row "A FETCH that does not come
 * to its end"), so a relay that is going to answer has begun to by then.
 */
const DEFAULT_FETCH_TIMEOUT_MS = 15_000;

export interface FollowCatalogOptions {
	/** The namespace of the catalog track: the one the broadcast is in. */
	namespace: TrackNamespace;
	/**
	 * Ends the reading when it aborts: the subscription is cancelled, and
	 * the FETCH with it. With a signal that has aborted already nothing is
	 * sent, and `followCatalog` rejects with its reason.
	 */
	signal?: AbortSignal;
	/** The most bytes of a catalog Object that are read; `readCatalog` has the default. */
	maxLength?: number;
	/** How long the Joining FETCH has, in milliseconds; 15000 when left out. */
	fetchTimeoutMs?: number;
	/**
	 * The RENDEZVOUS_TIMEOUT of the SUBSCRIBE for the catalog track, in
	 * milliseconds (draft-ietf-moq-transport-18 §10.2.6): a relay that has
	 * no publisher of the broadcast holds the SUBSCRIBE that long for one
	 * to appear, and `followCatalog` resolves when one has. Where none
	 * does, it rejects with the relay's REQUEST_ERROR, which is `TIMEOUT`
	 * (0x2) from a relay that held it. Left out, the parameter is not sent,
	 * and a relay without a publisher refuses at once. `@xmoq/moqt` waits
	 * for the answer that much longer than for another request.
	 *
	 * WHY on this SUBSCRIBE alone: once the catalog track is answered the
	 * broadcast is there, and its tracks are subscribed to without a wait.
	 */
	rendezvousTimeoutMs?: number;
}

/**
 * What became of the Joining FETCH. §5 asks for it, and the catalog can come
 * by the subscription all the same, so this is how one that did not work is
 * seen.
 */
export type JoiningFetchOutcome =
	/**
	 * None was sent: the SUBSCRIBE_OK had no LARGEST_OBJECT, so nothing was
	 * published on the track (draft-ietf-moq-transport-18 §10.2.11: "If
	 * omitted from a message, the sending endpoint has not published or
	 * received any Objects in the Track"), and a Joining FETCH for such a
	 * track is refused (§10.12.2: "If no Objects have been published for the
	 * track the publisher MUST respond with a REQUEST_ERROR with error code
	 * INVALID_RANGE").
	 */
	| { type: "not-sent" }
	/** It was answered and its fetch stream read to the end, with this many Objects. */
	| { type: "served"; objects: number }
	/**
	 * It was refused, took too long, was given up with the reading, or its
	 * fetch stream could not be read; `error` is what it failed with. The
	 * Objects that came before are counted and were taken.
	 */
	| { type: "failed"; objects: number; error: unknown };

export interface CatalogFollower {
	/**
	 * What is known of the catalog, each time it changes. A state that was
	 * not read when the next one came is replaced by it: a reader that is
	 * slow gets the newest, and at most one is held. The first state comes
	 * when the first Object has: nothing comes for a track that has none.
	 *
	 * Closes when the subscription has ended, however it ended; `ended` says
	 * how. Cancelling it cancels the subscription and the Joining FETCH.
	 *
	 * The end of the subscription gives the Joining FETCH up as well: what
	 * of it was not read is not delivered. A track that ended while its
	 * catalog was being fetched closes this without a state; `joiningFetch`
	 * then says that the FETCH failed.
	 */
	readonly states: ReadableStream<CatalogState>;
	/** Resolves once the subscription to the catalog track has ended, with how. Never rejects. */
	readonly ended: Promise<SubscriptionEnd>;
	/** Resolves once the Joining FETCH is over, or was not sent, with how. Never rejects. */
	readonly joiningFetch: Promise<JoiningFetchOutcome>;
}

/**
 * Subscribes to the catalog track of `namespace` and reads it as §5 asks:
 * "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining
 * FETCH (offset = 0) in order to obtain the latest complete catalog along
 * with all subsequent catalog objects".
 *
 * Resolves when the SUBSCRIBE was accepted, and rejects with what
 * `MoqtConnection.subscribe` rejects with, or with a `RangeError` for a
 * `maxLength` that is no length, before anything is sent.
 *
 * The FETCH is sent after the SUBSCRIBE_OK, and only where that has a
 * LARGEST_OBJECT (`JoiningFetchOutcome`, "not-sent", has why that keeps to
 * the sentence above). A FETCH that is refused, fails or takes too long is
 * given up, and the reading goes on with the subscription alone: the first
 * catalog is then the one of the next Group the publisher starts.
 * `joiningFetch` of the result says which it was.
 *
 * Every call starts from nothing: the Locations of an earlier subscription
 * are not compared, so a publisher that starts again at a lower Group
 * (§6.1 has it use a larger one; the catalog of `mlmpub` is Object 0 of
 * Group 0 after every start) is followed all the same.
 */
export async function followCatalog(
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "close">,
	options: FollowCatalogOptions,
): Promise<CatalogFollower> {
	const { signal } = options;
	// WHY here: `readCatalog` would throw it for the first Object, where
	// nobody is left to catch it.
	if (options.maxLength !== undefined) checkMaxLength(options.maxLength);
	const { rendezvousTimeoutMs } = options;
	if (
		rendezvousTimeoutMs !== undefined &&
		(!Number.isSafeInteger(rendezvousTimeoutMs) || rendezvousTimeoutMs < 0)
	) {
		throw new RangeError(
			`rendezvousTimeoutMs is a whole number of milliseconds from 0, not ${rendezvousTimeoutMs}`,
		);
	}
	signal?.throwIfAborted();
	// The one thing that ends the reading: the caller's signal, the
	// cancelling of `states`, and the end of the subscription all go through
	// it, so that the FETCH ends with whichever comes.
	const stop = new AbortController();
	const stopBySignal = (): void => stop.abort(signal?.reason);
	signal?.addEventListener("abort", stopBySignal, { once: true });
	const maxLength = options.maxLength ?? DEFAULT_MAX_CATALOG_LENGTH;
	let subscription: Subscription;
	try {
		subscription = await connection.subscribe({
			trackNamespace: options.namespace,
			trackName: CATALOG_TRACK_NAME,
			// An Object that is longer than a catalog that is read ends its
			// stream before the whole of it has come, as on the Joining FETCH.
			// WHY at least 1: `@xmoq/moqt` takes a limit from 1, and a
			// `maxLength` of 0 is one this takes; an Object of 1 byte is no
			// catalog either, and `readCatalog` says so.
			maxObjectPayloadLength: Math.max(maxLength, 1),
			...(options.rendezvousTimeoutMs === undefined
				? {}
				: {
						parameters: {
							rendezvousTimeout: BigInt(options.rendezvousTimeoutMs),
						},
					}),
		});
	} catch (error) {
		signal?.removeEventListener("abort", stopBySignal);
		throw error;
	}
	void subscription.ended.then(() => {
		signal?.removeEventListener("abort", stopBySignal);
		stop.abort();
	});
	const cancel = (): void => {
		// Rejects when the reset cannot be written, which ends the session
		// and with it the subscription.
		void subscription.cancel().catch(() => {});
	};
	// The signal can have aborted while the SUBSCRIBE was under way.
	if (stop.signal.aborted) cancel();
	else stop.signal.addEventListener("abort", cancel, { once: true });

	let state: CatalogState = {};
	/** The state nobody has read yet. */
	let unread: CatalogState | undefined;
	let over = false;
	let wake: (() => void) | undefined;
	const woken = (): void => {
		wake?.();
		wake = undefined;
	};
	const take = (location: CatalogLocation, payload: Uint8Array): void => {
		const next = catalogStateAfter(state, location, payload, options.maxLength);
		if (next === state) return;
		state = next;
		unread = next;
		woken();
	};

	/** Reads the Joining FETCH to its end; `count` has how many Objects it had, as they come. */
	const fetchBefore = async (count: { objects: number }): Promise<void> => {
		const timeout = AbortSignal.timeout(
			options.fetchTimeoutMs ?? DEFAULT_FETCH_TIMEOUT_MS,
		);
		const fetch = await connection.fetch(
			{
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: 0n,
				},
			},
			{ signal: AbortSignal.any([stop.signal, timeout]) },
		);
		// §5.2 of the transport: "some Objects could be received even if the
		// FETCH results in error". What a FETCH that was refused brings is
		// not taken.
		await fetch.response;
		const reader = decodeFetchBody(connection, fetch, {
			// An Object of the fetch stream that is longer than a catalog that
			// is read ends the FETCH before the whole of it has come.
			limits: { maxObjectPayloadLength: maxLength },
		}).getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			// The reading was given up while this was on its way: with the
			// subscription over, what the FETCH still has is not delivered.
			stop.signal.throwIfAborted();
			// An End of Range indicator says what is not there; the Objects
			// after it are read on.
			if (next.value.type !== "object") continue;
			const { groupId, objectId, payload } = next.value.object;
			count.objects += 1;
			take({ group: groupId, object: objectId }, payload);
		}
	};

	const joined = async (): Promise<JoiningFetchOutcome> => {
		if (subscription.parameters.largestObject === undefined) {
			return { type: "not-sent" };
		}
		const count = { objects: 0 };
		try {
			await fetchBefore(count);
			return { type: "served", objects: count.objects };
		} catch (error) {
			return { type: "failed", objects: count.objects, error };
		}
	};
	const joiningFetch = joined();

	const read = async (): Promise<void> => {
		// WHY the FETCH is read to its end first: what it brings is older than
		// what the subscription brings, and a reader is to see the catalog
		// that was there before the one that replaces it. WHY a failure of it
		// is no failure here: without the FETCH the catalog comes later, with
		// the next Group; `joiningFetch` says that it failed.
		await joiningFetch;
		const reader = subscription.objects.getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			const { groupId, object } = next.value;
			// An Object that only says something does not exist is no catalog.
			if (object.status !== "Normal") continue;
			take({ group: groupId, object: object.objectId }, object.payload);
		}
	};
	// `objects` fails for a subscription that was cut off; `ended` has why.
	void read()
		.catch(() => {})
		.then(() => {
			over = true;
			woken();
		});

	const states = new ReadableStream<CatalogState>(
		{
			async pull(controller) {
				if (unread === undefined && !over) {
					await new Promise<void>((resolve) => {
						wake = resolve;
					});
				}
				if (unread !== undefined) {
					controller.enqueue(unread);
					unread = undefined;
				} else {
					controller.close();
				}
			},
			cancel: () => stop.abort(),
		},
		// WHY 0: with the default of 1 the stream would take a state before
		// anyone asked, and a newer one could no longer replace it.
		new CountQueuingStrategy({ highWaterMark: 0 }),
	);
	return { states, ended: subscription.ended, joiningFetch };
}
