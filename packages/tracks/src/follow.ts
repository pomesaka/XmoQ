// A followed track: a track of one numbered item a Group (its Object 0, the
// number its Group ID), read with what came before. The SUBSCRIBE waits for
// a publisher that may come later, a Joining FETCH brings the items before,
// each item is handed on once, and the subscription is made again after it
// ends, for as long as the session lasts. The chat's `lines` and a
// transcript's `speech` are such tracks; what an item says is the caller's
// to read. Section numbers are of draft-ietf-moq-transport-18.

import {
	createAskAgain,
	decodeFetchBody,
	type MoqtConnection,
	RequestFailedError,
	type Subscription,
	type SubscriptionEnd,
	type TrackNamespace,
	waitFor,
} from "@xmoq/moqt";
import { type FollowGap, followPlan } from "./core/follow-plan.ts";
import { SeenNumbers } from "./core/seen-numbers.ts";

/**
 * The RENDEZVOUS_TIMEOUT (§10.2.6) of the SUBSCRIBE of a followed track,
 * whose publisher may come after the reader. **Provisional.** WHY 30 s: the
 * value decided for a followed track (`docs/architecture.md`, "The roles of
 * an endpoint"), the most the own relay holds a SUBSCRIBE
 * (`MAX_RENDEZVOUS_MS` of `xmoq-relay-core`), and the Rust readers' too
 * (`RENDEZVOUS_MS` of `xmoq-subscriber`; `fixtures/followed-track/`).
 */
export const FOLLOW_RENDEZVOUS_MS = 30_000;

/**
 * How long the Joining FETCH may take, from its sending to its last item.
 * **Provisional**. WHY a bound: a FETCH that never ends would hold what it
 * read for as long as the session lasts.
 */
export const FOLLOW_FETCH_TIMEOUT_MS = 15_000;

/** How an item came. */
export type ItemCame = "live" | "history";

/** How the following goes. */
export interface FollowStatus {
	/** Whether the track is subscribed to. */
	following: boolean;
	/** How the last subscription ended, or why it was refused; undefined before. */
	lastEnd: string | undefined;
}

export interface FollowTrackOptions {
	trackNamespace: TrackNamespace;
	trackName: Uint8Array;
	/** How many of the newest items are fetched where none was had; 0 fetches none. */
	recent: number;
	/** How many of the items it missed are fetched at the most where some were had; 0 fetches none. */
	catchUp: number;
	/** The number of the newest item the caller already has, from a reader before (a session before). */
	lastHad?: bigint;
	/** How many numbers are remembered, so that an item is handed on once (`SeenNumbers`). */
	remembered: number;
	/** The longest payload of an item, live or fetched: a longer one ends its stream, and the subscription goes on. */
	maxPayloadLength: number;
	/** `FOLLOW_RENDEZVOUS_MS` when left out. */
	rendezvousMs?: number;
	/** `FOLLOW_FETCH_TIMEOUT_MS` when left out. */
	fetchTimeoutMs?: number;
	/** Each item, once, by its number; items can come out of their order (a live one before the history). */
	take(number: bigint, payload: Uint8Array, how: ItemCame): void;
	/** Items that were not fetched, told before the items after them. */
	onGap?(gap: FollowGap): void;
	onStatus?(status: FollowStatus): void;
	/** Stops the following: the subscription is cancelled, and none is made after. */
	signal?: AbortSignal;
}

export interface FollowedTrack {
	status(): FollowStatus;
	/** The number of the newest item that came, an item the caller read or not; undefined before. */
	had(): bigint | undefined;
}

const describedEnd = (end: SubscriptionEnd): string => {
	switch (end.type) {
		case "publish-done":
			return `PUBLISH_DONE 0x${end.statusCode.toString(16)} ${end.errorReason}`;
		case "peer-reset":
			return `reset ${end.code ?? "without a code"}`;
		case "session-closed":
			return "the session closed";
		default:
			return end.type;
	}
};

/**
 * Follows the track on `connection` until `signal` is aborted or the
 * connection closes. WHY it ends with the connection: a SUBSCRIBE on a closed
 * connection is refused at once, and asking again by the pause of a refusal
 * would go on for as long as the reader is kept; the next session's reader is
 * the caller's (`keepSessions` of `@xmoq/moqt`).
 *
 * The Joining FETCH of each subscription asks for the items after the newest
 * had (`followPlan`); a FETCH that fails is no failure of the following, the
 * items that come live are there all the same.
 */
export function followTrack(
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "closed" | "close">,
	options: FollowTrackOptions,
): FollowedTrack {
	const stop = new AbortController();
	options.signal?.addEventListener("abort", () => stop.abort(), {
		once: true,
	});
	if (options.signal?.aborted) stop.abort();
	void connection.closed.then(() => stop.abort());
	const status: FollowStatus = { following: false, lastEnd: undefined };
	const changed = (change: Partial<FollowStatus>): void => {
		Object.assign(status, change);
		options.onStatus?.({ ...status });
	};
	let seen = new SeenNumbers(options.remembered);
	/**
	 * The largest number that came, an item or not: the next subscription's
	 * FETCH catches up from the one after it. WHY not only the items the
	 * caller read: an Object that is no item was had all the same, and
	 * fetching it again would bring it again.
	 */
	let had = options.lastHad;
	const take = (number: bigint, payload: Uint8Array, how: ItemCame): void => {
		if (had === undefined || number > had) had = number;
		if (seen.take(number)) options.take(number, payload, how);
	};

	const history = async (subscription: Subscription): Promise<void> => {
		// §10.12.2: a Joining FETCH for a track whose SUBSCRIBE_OK said that
		// nothing was published is refused with INVALID_RANGE; a track
		// without items has nothing to fetch (`followPlan`).
		const largest = subscription.parameters.largestObject?.group;
		// WHY decided here, before the first `await`: the live items that
		// come next would move `had` past what this FETCH is to bring.
		const plan = followPlan(had, largest, options.recent, options.catchUp);
		if (plan.forget) {
			seen = new SeenNumbers(options.remembered);
			had = undefined;
		}
		if (plan.fetch === undefined || largest === undefined) return;
		const { start, gap } = plan.fetch;
		if (gap !== undefined) options.onGap?.(gap);
		const most = largest - start + 1n;
		const fetch = await connection.fetch(
			{
				range: {
					// An Absolute Joining FETCH from the Group `followPlan`
					// gives (§10.12.2.1), to the Joining Location: why
					// Absolute is at `followStart`.
					fetchType: "AbsoluteJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: start,
				},
			},
			{
				signal: AbortSignal.any([
					stop.signal,
					AbortSignal.timeout(
						options.fetchTimeoutMs ?? FOLLOW_FETCH_TIMEOUT_MS,
					),
				]),
			},
		);
		await fetch.response;
		const reader = decodeFetchBody(connection, fetch, {
			limits: { maxObjectPayloadLength: options.maxPayloadLength },
		}).getReader();
		let objects = 0n;
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			if (next.value.type !== "object") continue;
			const { groupId, objectId, payload } = next.value.object;
			if (objectId !== 0n) continue;
			take(groupId, payload, "history");
			objects += 1n;
			// A relay or a publisher that sends more than was asked for is not
			// read further.
			if (objects >= most) {
				await reader.cancel();
				return;
			}
		}
	};

	const live = async (subscription: Subscription): Promise<void> => {
		const reader = subscription.objects.getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			const { groupId, object } = next.value;
			// An item is Object 0 of its Group, and a status says no item.
			if (object.status !== "Normal" || object.objectId !== 0n) continue;
			take(groupId, object.payload, "live");
		}
	};

	const follow = async (): Promise<void> => {
		const again = createAskAgain();
		while (!stop.signal.aborted) {
			let subscription: Subscription;
			try {
				subscription = await connection.subscribe({
					trackNamespace: options.trackNamespace,
					trackName: options.trackName,
					parameters: {
						rendezvousTimeout: BigInt(
							options.rendezvousMs ?? FOLLOW_RENDEZVOUS_MS,
						),
					},
					maxObjectPayloadLength: options.maxPayloadLength,
				});
			} catch (error) {
				changed({ lastEnd: `refused: ${String(error).slice(0, 120)}` });
				await waitFor(
					again.afterRefusal(
						error instanceof RequestFailedError ? error.retryWaitMs : undefined,
					),
					stop.signal,
				);
				continue;
			}
			const cancel = (): void => {
				void subscription.cancel().catch(() => {});
			};
			// The signal can have aborted while the SUBSCRIBE waited for its
			// answer: the subscription made after it is cancelled at once.
			if (stop.signal.aborted) {
				cancel();
				return;
			}
			stop.signal.addEventListener("abort", cancel, { once: true });
			// A GOAWAY on its request stream (§10.4: "the endpoint SHOULD
			// re-issue that specific request on a session at the specified
			// URI"): it is cancelled, and asked for again on this session after
			// the pause of an end, as the sound's subscription is
			// (`keepTrackOnSession`) and the Rust readers' (`xmoq-subscriber`),
			// also where a URI is named (the implementation notes, GOAWAY,
			// received).
			void subscription.goAway.then(cancel);
			again.subscribed();
			changed({ following: true });
			const fetched = history(subscription).catch(() => {});
			await live(subscription).catch(() => {});
			const end = await subscription.ended;
			await fetched;
			stop.signal.removeEventListener("abort", cancel);
			changed({ following: false, lastEnd: describedEnd(end) });
			await waitFor(again.afterEnd(), stop.signal);
		}
	};
	void follow();

	return {
		status: () => ({ ...status }),
		had: () => had,
	};
}
