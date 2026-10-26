// A reader of a transcript over a connection of @xmoq/moqt: it follows the
// track `speech` the transcriber (`crates/transcriber`) publishes, with the
// latest pieces that came before (`followTrack` of @xmoq/tracks).

import type { MoqtConnection } from "@xmoq/moqt";
import { followTrack } from "@xmoq/tracks";
import { SPEECH_TRACK, transcriptNamespace } from "../core/names.ts";
import { type Malformed, readSpeech, type Speech } from "../core/payloads.ts";

/** Pieces fetched when the reader begins, where it is not told otherwise: the caption of a page and the two before. **Provisional**. */
export const LATEST_PIECES = 3;

/**
 * How many of the pieces it missed the reader fetches at the most, when its
 * subscription is made again: from the piece after the last one had, as the
 * agent does (`SPEECH_CATCH_UP` of `crates/agent`, 16). **Provisional.**
 * WHY the same as the agent: the two follow the same track, and a page that
 * shows fewer passes over what it does not show.
 */
const CATCH_UP_PIECES = 16;

/**
 * The longest payload of a piece: 15 s of speech is under 1 KB of text, and
 * the transcriber writes nothing longer. 64 KiB, as a line of the chat.
 */
export const MAX_PIECE_PAYLOAD = 64 * 1024;

/** How many piece numbers are remembered, so that a piece is handed on once. **Provisional**. */
export const SEEN_PIECES = 200;

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
	/** Pieces fetched when the reader begins with none had (`LATEST_PIECES`); 0 fetches none. */
	latest?: number;
	/**
	 * The number of the newest piece the caller already has, from a reader
	 * before (a session before): the first FETCH then catches up from the
	 * piece after it, as a subscription made again does.
	 */
	lastHad?: bigint;
	onSpeech(speech: ReceivedSpeech): void;
	/** A payload that is no piece; it is passed over. */
	onMalformed?(number: bigint, why: Malformed): void;
	onStatus?(status: TranscriptStatus): void;
}

export interface TranscriptReader {
	status(): TranscriptStatus;
	/** The number of the newest piece that came, read or not; undefined before. */
	had(): bigint | undefined;
	/** Ends the subscription, and subscribes no more. */
	stop(): void;
}

/**
 * Follows the transcript of `listened` for as long as the connection lasts
 * or until `stop`: the latest pieces by a Joining FETCH, then every piece
 * live, and after a subscription made again, the pieces after the last one
 * had (Absolute Joining FETCH, as the agent asks). Each piece is handed on
 * once, whichever way it came.
 */
export function followTranscript(
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "close" | "closed">,
	options: FollowTranscriptOptions,
): TranscriptReader {
	const stop = new AbortController();
	const followed = followTrack(connection, {
		trackNamespace: transcriptNamespace(options.listened),
		trackName: new TextEncoder().encode(SPEECH_TRACK),
		recent: options.latest ?? LATEST_PIECES,
		catchUp: CATCH_UP_PIECES,
		...(options.lastHad === undefined ? {} : { lastHad: options.lastHad }),
		remembered: SEEN_PIECES,
		maxPayloadLength: MAX_PIECE_PAYLOAD,
		take(number, payload, how) {
			const read = readSpeech(number, payload);
			if (!read.ok) {
				options.onMalformed?.(number, read.why);
				return;
			}
			options.onSpeech({ ...read.value, how });
		},
		onStatus: ({ following, lastEnd }) =>
			options.onStatus?.(
				lastEnd === undefined ? { following } : { following, lastEnd },
			),
		signal: stop.signal,
	});
	return {
		status: () => {
			const { following, lastEnd } = followed.status();
			return lastEnd === undefined ? { following } : { following, lastEnd };
		},
		had: () => followed.had(),
		stop: () => stop.abort(),
	};
}
