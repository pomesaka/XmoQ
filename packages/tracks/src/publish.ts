// A published track: a track this endpoint is the publisher of, as the
// requests of the peer see it. It takes the SUBSCRIBE and the FETCH for its
// Full Track Name, answers them while the track goes on, refuses them after
// its end, and tells a subscription of the end with PUBLISH_DONE. What goes
// on the subgroup streams, and how long the end waits for it, is the
// caller's: the video and the sound of @xmoq/media, the catalog of
// @xmoq/msf, and the tracks of a participant of @xmoq/chat. Section numbers
// are of draft-ietf-moq-transport-18.

import {
	LONGEST_TIMER_MS,
	type PeerFetch,
	type PeerSubscribe,
	PublishDoneCode,
	PublishEndedError,
	type PublishedTrack,
	RequestErrorCode,
	ServedFetchEndedError,
	SessionClosedError,
	type SubscribeAnswer,
	sameFullTrackName,
	type TrackNamespace,
} from "@xmoq/moqt";
import type { ServedTrack } from "./serve.ts";

/**
 * How long the end of a published track waits for what is under way, in
 * milliseconds, where the caller gives no other time. **Provisional**: the
 * time `@xmoq/moqt` gives a peer to end a request stream, for the same
 * reason, a peer that does not do its part. Not chosen by a measurement.
 */
export const DEFAULT_END_TIMEOUT_MS = 3000;

/**
 * Whether a call on a request, a subscription or a fetch stream failed
 * because the peer ended it, or the session is over: nothing is left to do
 * then, and it is no fault of anyone here.
 */
export const endedByThePeer = (error: unknown): boolean =>
	error instanceof PublishEndedError ||
	error instanceof ServedFetchEndedError ||
	error instanceof SessionClosedError;

/**
 * `value`, where it is a whole number from `least` to the longest time a
 * timer takes (`LONGEST_TIMER_MS`, a signed 32-bit number of milliseconds);
 * a `RangeError` that names `name` for anything else, NaN and a fraction
 * among them. For the times and the counts of a publisher's options.
 */
export function wholeNumber(
	name: string,
	value: number,
	least: number,
): number {
	if (!Number.isInteger(value) || value < least || value > LONGEST_TIMER_MS) {
		throw new RangeError(
			`${name} is a whole number from ${least} to ${LONGEST_TIMER_MS}, not ${String(value)}`,
		);
	}
	return value;
}

/**
 * Ends a subscription with PUBLISH_DONE (TRACK_ENDED, §10.11). Never
 * rejects: a failure for another reason than that the peer ended the
 * subscription, or the session, is reported with `reportError`, the global
 * of HTML for an error nobody is there to take, as `@xmoq/moqt` reports a
 * failure of a handler of `serve`. WHY reported and not passed over: a
 * PUBLISH_DONE that was not sent leaves the subscriber waiting for the end
 * of a track that has ended.
 */
export function tellEnd(track: PublishedTrack): Promise<void> {
	return track.done(PublishDoneCode.TRACK_ENDED, "").then(
		() => {},
		(error: unknown) => {
			if (!endedByThePeer(error)) reportError(error);
		},
	);
}

export interface PublishTrackOptions {
	trackNamespace: TrackNamespace;
	trackName: Uint8Array;
	/**
	 * How many subscriptions it has at once. `one`: a relay makes one for all
	 * its viewers, and a second SUBSCRIBE while the first is open or being
	 * answered is refused with DUPLICATE_SUBSCRIPTION; one that comes after
	 * the first ended is accepted. `many`: every SUBSCRIBE is accepted.
	 */
	subscribers: "one" | "many";
	/**
	 * The SUBSCRIBE_OK, made when a SUBSCRIBE is accepted. §10.2.11: "If
	 * Objects have been published on this Track the Publisher MUST include"
	 * LARGEST_OBJECT; it is the caller's to know where it is.
	 */
	answer(): SubscribeAnswer;
	/**
	 * A subscription was accepted while the track goes on. `answer` is the
	 * SUBSCRIBE_OK it was accepted with. The subscription is the caller's from
	 * here on, to the PUBLISH_DONE that `tellEnd` sends.
	 */
	subscribed(
		track: PublishedTrack,
		request: PeerSubscribe,
		answer: SubscribeAnswer,
	): void;
	/**
	 * A subscription that was accepted after the track ended, its answer
	 * having been on the way: `tellEnd` when left out. The catalog gives it
	 * the last catalog first.
	 */
	subscribedLate?(
		track: PublishedTrack,
		request: PeerSubscribe,
		answer: SubscribeAnswer,
	): void;
	/**
	 * Answers a FETCH for the track while it goes on. Left out, a FETCH is
	 * refused with NOT_SUPPORTED: nothing of the track is kept to answer one.
	 */
	fetch?(request: PeerFetch): void;
}

/** A published track: a `ServedTrack` for `serveTracks`, and its end. */
export interface PublishingTrack extends ServedTrack {
	/** Whether `end` was called. */
	readonly ended: boolean;
	/**
	 * Ends the track: a SUBSCRIBE or FETCH for it from now on is refused with
	 * DOES_NOT_EXIST, and a SUBSCRIBE whose answer is on the way gets
	 * `subscribedLate` once it is accepted. Resolves once the answers that
	 * were on the way are settled; never rejects. The caller waits on it no
	 * longer than it waits for the rest of its end.
	 */
	end(): Promise<void>;
}

/**
 * The requests of the peer for one track this endpoint publishes.
 *
 * **An answer that cannot be sent is refused**: `accept` of `@xmoq/moqt`
 * rejects with a `RangeError` for a SUBSCRIBE_OK that does not fit its
 * fields, sends nothing, and the request still waits for an answer. It is
 * refused with INTERNAL_ERROR, and the failure reported with `reportError`.
 * WHY: the peer would wait for an answer until it gives up. A request that
 * went before it was answered needs none.
 */
export function publishTrack(options: PublishTrackOptions): PublishingTrack {
	/** The subscription of a track of `one`, from its answer to its end. */
	let held: PublishedTrack | undefined;
	/** The SUBSCRIBEs whose answer is on the way. */
	const answering = new Set<Promise<void>>();
	let ended = false;

	const isThisTrack = (request: PeerSubscribe | PeerFetch): boolean =>
		sameFullTrackName(request, {
			trackNamespace: options.trackNamespace,
			trackName: options.trackName,
		});

	return {
		get ended() {
			return ended;
		},

		subscribe(request) {
			if (!isThisTrack(request)) return false;
			if (ended) {
				request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				return true;
			}
			if (
				options.subscribers === "one" &&
				(held !== undefined || answering.size > 0)
			) {
				request
					.reject(
						RequestErrorCode.DUPLICATE_SUBSCRIPTION,
						"the track has its one subscription",
					)
					.catch(() => {});
				return true;
			}
			const answer = options.answer();
			const answered: Promise<void> = request.accept(answer).then(
				(track) => {
					if (ended) {
						if (options.subscribedLate === undefined) void tellEnd(track);
						else options.subscribedLate(track, request, answer);
						return;
					}
					if (options.subscribers === "one") {
						held = track;
						void track.ended.then(() => {
							if (held === track) held = undefined;
						});
					}
					options.subscribed(track, request, answer);
				},
				(error: unknown) => {
					// The request went before it was answered.
					if (request.signal.aborted) return;
					// The answer was not sent, and the request waits for one.
					request.reject(RequestErrorCode.INTERNAL_ERROR).catch(() => {});
					reportError(error);
				},
			);
			answering.add(answered);
			void answered.finally(() => answering.delete(answered));
			return true;
		},

		fetch(request) {
			if (!isThisTrack(request)) return false;
			if (ended) {
				request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
			} else if (options.fetch === undefined) {
				request.reject(RequestErrorCode.NOT_SUPPORTED).catch(() => {});
			} else {
				options.fetch(request);
			}
			return true;
		},

		async end() {
			ended = true;
			await Promise.allSettled(answering);
		},
	};
}
