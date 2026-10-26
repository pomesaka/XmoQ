// A reader of a transcript over a connection of @xmoq/moqt: it subscribes
// to the track `speech` the transcriber (`crates/transcriber`) publishes,
// and fetches the latest pieces that came before. Section numbers are of
// draft-ietf-moq-transport-18.

import {
	createAskAgain,
	decodeFetchBody,
	type MoqtConnection,
	RequestFailedError,
	type Subscription,
	waitFor,
} from "@xmoq/moqt";
import { SPEECH_TRACK, transcriptNamespace } from "../core/names.ts";
import { type Malformed, readSpeech, type Speech } from "../core/payloads.ts";

/** Pieces fetched when the reader begins, where it is not told otherwise: the caption of a page and the two before. **Provisional**. */
export const LATEST_PIECES = 3;

/**
 * The longest payload of a piece: 15 s of speech is under 1 KB of text, and
 * the transcriber writes nothing longer. 64 KiB, as a line of the chat.
 */
const MAX_PIECE_PAYLOAD = 64 * 1024;

/**
 * How long the SUBSCRIBE asks the relay to wait for the transcript to be
 * published (RENDEZVOUS_TIMEOUT, §10.2.6): the transcriber publishes when
 * its session begins, which may be after the page's. As `@xmoq/chat` waits
 * for a room. **Provisional**.
 */
const RENDEZVOUS_MS = 10_000;

/** How long the Joining FETCH has, as `@xmoq/chat` gives its own. **Provisional**. */
const FETCH_TIMEOUT_MS = 15_000;

/** How many piece numbers are remembered, so that a piece is handed on once. **Provisional**. */
const SEEN_PIECES = 200;

/** A piece as the reader hands it on. */
export type ReceivedSpeech = Speech & {
	/** Whether it came on the subscription or by the Joining FETCH. */
	how: "live" | "history";
};

/** What the reader is doing. */
export interface TranscriptStatus {
	/** Whether a subscription to `speech` is established. */
	following: boolean;
	/** How the last subscription ended or was refused, for a person to read. */
	lastEnd?: string;
}

export interface FollowTranscriptOptions {
	/** The prefix the transcriber listens under, each field: the transcript is of `xmoq/transcript/<listened>`. */
	listened: readonly string[];
	/** Pieces fetched when each subscription begins (`LATEST_PIECES`); 0 fetches none. */
	latest?: number;
	onSpeech(speech: ReceivedSpeech): void;
	/** A payload that is no piece; it is passed over. */
	onMalformed?(number: bigint, why: Malformed): void;
	onStatus?(status: TranscriptStatus): void;
}

export interface TranscriptReader {
	status(): TranscriptStatus;
	/** Ends the subscription, and subscribes no more. */
	stop(): void;
}

/**
 * Follows the transcript of `listened` for as long as the connection lasts
 * or until `stop`: the latest pieces by a Joining FETCH, then every piece
 * live. A subscription that ends or is refused is made again after the
 * pause of `createAskAgain` (@xmoq/moqt): 1 s, or the Retry Interval of a
 * refusal. Each piece is handed on once, whichever way it came.
 */
export function followTranscript(
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "close">,
	options: FollowTranscriptOptions,
): TranscriptReader {
	const stop = new AbortController();
	const status: TranscriptStatus = { following: false };
	const changed = (next: Partial<TranscriptStatus>): void => {
		Object.assign(status, next);
		options.onStatus?.({ ...status });
	};
	const latest = options.latest ?? LATEST_PIECES;
	const seen: bigint[] = [];
	const take = (
		number: bigint,
		payload: Uint8Array,
		how: ReceivedSpeech["how"],
	): void => {
		if (seen.includes(number)) return;
		seen.push(number);
		if (seen.length > SEEN_PIECES) seen.shift();
		const read = readSpeech(number, payload);
		if (!read.ok) {
			options.onMalformed?.(number, read.why);
			return;
		}
		options.onSpeech({ ...read.value, how });
	};

	const history = async (subscription: Subscription): Promise<void> => {
		// §10.12.2: a Joining FETCH for a track whose SUBSCRIBE_OK said that
		// nothing was published is refused with INVALID_RANGE.
		if (latest <= 0 || subscription.parameters.largestObject === undefined) {
			return;
		}
		const fetch = await connection.fetch(
			{
				range: {
					// §10.12.2.1: from {Joining Location.Group - Joining Start, 0}
					// to the Joining Location, `latest` Groups.
					fetchType: "RelativeJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: BigInt(latest - 1),
				},
			},
			{
				signal: AbortSignal.any([
					stop.signal,
					AbortSignal.timeout(FETCH_TIMEOUT_MS),
				]),
			},
		);
		await fetch.response;
		const reader = decodeFetchBody(connection, fetch, {
			limits: { maxObjectPayloadLength: MAX_PIECE_PAYLOAD },
		}).getReader();
		let objects = 0;
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			if (next.value.type !== "object") continue;
			const { groupId, objectId, payload } = next.value.object;
			if (objectId !== 0n) continue;
			take(groupId, payload, "history");
			objects += 1;
			// A relay that sends more than was asked for is not read further.
			if (objects >= latest) {
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
			// A piece is Object 0 of its Group, and a status says no piece.
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
					trackNamespace: transcriptNamespace(options.listened),
					trackName: new TextEncoder().encode(SPEECH_TRACK),
					parameters: { rendezvousTimeout: BigInt(RENDEZVOUS_MS) },
					maxObjectPayloadLength: MAX_PIECE_PAYLOAD,
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
			again.subscribed();
			changed({ following: true });
			// WHY a failed FETCH is no failure of the reader: the pieces that
			// come live are there all the same.
			const fetched = history(subscription).catch(() => {});
			await live(subscription).catch(() => {});
			const end = await subscription.ended;
			await fetched;
			stop.signal.removeEventListener("abort", cancel);
			changed({ following: false, lastEnd: end.type });
			await waitFor(again.afterEnd(), stop.signal);
		}
	};
	void follow();

	return {
		status: () => ({ ...status }),
		stop: () => stop.abort(),
	};
}
