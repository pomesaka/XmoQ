// The limits of a line (decided on 2026-10-24), as a participant keeps
// to them before it sends: the gatherer drops a line over them
// (`crates/chat/src/gather.rs`), and a sender that knows the limits can say
// so to whoever typed the line instead of losing it without a word.
// The defaults and the ranges are those of `crates/chat-tracks/src/limits.rs`.

import { isWellFormed, MOST_EMOJI_UNITS } from "./payloads.ts";

/** What a participant keeps to; the gatherer's flags of the same name change them there. */
export interface ChatLimits {
	/** The most bytes of UTF-8 the text of a line may have (`--text-bytes`). */
	textBytes: number;
	/** The most lines one person may say in any second (`--lines-per-second`). */
	linesPerSecond: number;
	/** The most reactions one person may say in any second (`--reactions-per-second`, decided on 2026-10-26). */
	reactionsPerSecond: number;
}

/** The defaults of the gatherer. */
export const DEFAULT_LIMITS: Readonly<ChatLimits> = {
	textBytes: 512,
	linesPerSecond: 5,
	reactionsPerSecond: 5,
};

/**
 * How far a setting may take them, as the gatherer's flags may: from 1 to
 * 8,192 bytes, and to 1,000 lines or reactions a second. WHY the gatherer bounds them is
 * at `TEXT_BYTES_MOST` and `LINES_PER_SECOND_MOST` of the Rust side.
 */
export const LIMITS_MOST: Readonly<ChatLimits> = {
	textBytes: 8_192,
	linesPerSecond: 1_000,
	reactionsPerSecond: 1_000,
};

/** The limits with what `given` sets; a `RangeError` for a value that is no whole number from 1 to its most. */
export function chatLimits(given: Partial<ChatLimits> = {}): ChatLimits {
	const limits = { ...DEFAULT_LIMITS, ...given };
	for (const key of [
		"textBytes",
		"linesPerSecond",
		"reactionsPerSecond",
	] as const) {
		const value = limits[key];
		if (!Number.isSafeInteger(value) || value < 1 || value > LIMITS_MOST[key]) {
			throw new RangeError(
				`${key} is a whole number from 1 to ${LIMITS_MOST[key]}, not ${value}`,
			);
		}
	}
	return limits;
}

/** The span the rate is counted over, in milliseconds. */
const SPAN_MS = 1_000;

/**
 * The lines one person said in the last second, as the gatherer counts them
 * (`Window` of `crates/chat/src/rate.rs`): no more than the limit in any
 * second, and a line that is kept back is not counted. It holds the time of
 * `most` lines at the most.
 */
export class RateWindow {
	readonly #most: number;
	/** When each line of the last second was taken, the oldest first. */
	readonly #taken: number[] = [];

	constructor(most: number) {
		this.#most = most;
	}

	/** Whether a line at `nowMs` (a clock that does not go back) is taken; one that is, is counted. */
	take(nowMs: number): boolean {
		while (this.#taken.length > 0 && nowMs - (this.#taken[0] ?? 0) >= SPAN_MS) {
			this.#taken.shift();
		}
		if (this.#taken.length >= this.#most) return false;
		this.#taken.push(nowMs);
		return true;
	}
}

/** Why a line is kept back before it is sent. */
export type KeptBack =
	/** Longer than `textBytes`. */
	| "too-long"
	/** Over `linesPerSecond`. */
	| "too-fast"
	/** A string with a surrogate that has no partner, which no reader takes. */
	| "not-well-formed"
	/** Of a reaction: not of 1 to `MOST_EMOJI_UNITS` UTF-16 units. */
	| "not-an-emoji";

const encoder = new TextEncoder();

/**
 * Why `text` is to be kept back, or undefined where it may be sent; a line
 * that may be sent is counted in `window`. The length is checked before the
 * rate, as the gatherer does, so a line that is too long does not count.
 */
export function keepBack(
	text: string,
	limits: ChatLimits,
	window: RateWindow,
	nowMs: number,
): KeptBack | undefined {
	if (!isWellFormed(text)) return "not-well-formed";
	if (encoder.encode(text).length > limits.textBytes) return "too-long";
	if (!window.take(nowMs)) return "too-fast";
	return undefined;
}

/**
 * Why a reaction is to be kept back, or undefined where it may be sent; one
 * that may be sent is counted in `window`, as a line is in `keepBack`.
 */
export function keepBackReaction(
	emoji: string,
	window: RateWindow,
	nowMs: number,
): KeptBack | undefined {
	if (!isWellFormed(emoji)) return "not-well-formed";
	if (emoji.length < 1 || emoji.length > MOST_EMOJI_UNITS)
		return "not-an-emoji";
	if (!window.take(nowMs)) return "too-fast";
	return undefined;
}
