// When a client asks again: a track on the same session after its
// subscription ended or was refused (`createAskAgain`), and the next session
// after one ended (`createNextSession`). No I/O and no timer: these say how
// long, and the caller waits. The rule on the same session is also the
// Rust clients' (`crates/again`), and both read
// `fixtures/ask-again/same-session.tsv`; the rule of the next session is
// also `NextSession` of `xmoq-again`, and both read `next-session.tsv`.
//
// WHY here, in @xmoq/moqt, and not in a package of its own: the loop that
// follows a GOAWAY (`keepMoving`, `src/webtransport`) needs the connection's
// GOAWAY and the REQUEST_ERROR of this package, and a package of layer 0
// cannot depend on another of layer 0 (`scripts/ci/check-layers.sh`). WHY
// NOT in each package that asks again (@xmoq/media, @xmoq/chat,
// @xmoq/transcript, @xmoq/meeting): the rule would be in several places,
// and they would drift. The rules are the application's and not the
// protocol's: the session itself hands a Retry Interval on
// (`RequestFailedError.retryWaitMs`) and does not wait by it.

/**
 * **Provisional**: the pause after a subscription ended while the session
 * goes on, and after a refusal that names no time. Long enough that a relay
 * which refuses at once is not asked many times a second; a relay that
 * holds the request (RENDEZVOUS_TIMEOUT) has had its own wait before.
 */
export const DEFAULT_ASK_AGAIN_DELAY_MS = 1000;

/**
 * **Provisional**: the shortest pause after a refusal that names its own
 * (the Retry Interval of a REQUEST_ERROR, transport §10.6.2). It doubles
 * with every such refusal that follows another without a subscription in
 * between, up to `MAX_MIN_ASK_AGAIN_DELAY_MS`.
 *
 * WHY any, and why it grows: a peer that says "at once" and then refuses at
 * once would be asked in a loop, and ten times a second for as long as it
 * went on with a pause that stayed as it began.
 */
export const MIN_ASK_AGAIN_DELAY_MS = 100;

/** **Provisional**: what the shortest pause grows to. */
export const MAX_MIN_ASK_AGAIN_DELAY_MS = 5000;

/**
 * **Provisional**: the longest pause a refusal is let name. WHY: the value
 * is the peer's, and a timer takes one of 24 days; a client that is told
 * to come back that much later has in effect been told to go.
 */
export const MAX_NAMED_ASK_AGAIN_DELAY_MS = 30_000;

/** The pauses of `createAskAgain`, in milliseconds (`Pauses` of `crates/again`). */
export interface AskAgainPauses {
	/** After an end, and after a refusal that names no time. */
	afterEndMs: number;
	/** What the shortest pause after a refusal that names a time begins at. */
	shortestMs: number;
	/** What it grows to. */
	mostShortestMs: number;
	/** The longest a refusal is let name. */
	mostNamedMs: number;
}

/** How long to wait before the next SUBSCRIBE for one track, on one session. */
export interface AskAgain {
	/** The track is subscribed to: the shortest pause begins again. */
	subscribed(): void;
	/** Its subscription ended while the session goes on: the pause. */
	afterEnd(): number;
	/**
	 * It was refused, with the wait its Retry Interval names
	 * (`RequestFailedError.retryWaitMs`; undefined for none, also for a
	 * refusal that is no REQUEST_ERROR): the pause.
	 *
	 * WHY a pause, and no "do not ask", where the Retry Interval says the
	 * request SHOULD NOT be retried (0): whether to ask again is the
	 * caller's to say. A relay that gives every refusal a 0 (moqtail's does)
	 * would otherwise have no client wait at it.
	 */
	afterRefusal(namedMs: number | undefined): number;
}

export function createAskAgain(pauses: Partial<AskAgainPauses> = {}): AskAgain {
	const afterEndMs = pauses.afterEndMs ?? DEFAULT_ASK_AGAIN_DELAY_MS;
	const shortestMs = pauses.shortestMs ?? MIN_ASK_AGAIN_DELAY_MS;
	const mostShortestMs = pauses.mostShortestMs ?? MAX_MIN_ASK_AGAIN_DELAY_MS;
	const mostNamedMs = pauses.mostNamedMs ?? MAX_NAMED_ASK_AGAIN_DELAY_MS;
	let shortest = shortestMs;
	return {
		subscribed() {
			shortest = shortestMs;
		},
		afterEnd: () => afterEndMs,
		afterRefusal(namedMs) {
			if (namedMs === undefined) return afterEndMs;
			const ms = Math.min(Math.max(namedMs, shortest), mostNamedMs);
			shortest = Math.min(shortest * 2, mostShortestMs);
			return ms;
		},
	};
}

/**
 * **Provisional**: the least time from one try at a session to the next,
 * as the terminal viewer has it (`BETWEEN_TRIES` of `xmoq-watch`). Long
 * enough that a relay that is gone is not asked many times a second, and
 * short for a person who waits.
 */
export const DEFAULT_RECONNECT_DELAY_MS = 1000;

/**
 * The pause from one try at a session to the next, where the next is not
 * the one made at once after a GOAWAY: `firstMs` at first, twice the one
 * before after each try, up to `mostMs`, and `firstMs` again after a
 * session that lasted `wentWellMs` from its try (never where left out).
 * `Pause` of `xmoq-again`, whose roles have their own.
 */
export interface Pause {
	firstMs: number;
	mostMs: number;
	wentWellMs?: number;
}

/**
 * **Provisional**: the pages' pause, `Pause::VIEWER` of `xmoq-again`:
 * `DEFAULT_RECONNECT_DELAY_MS` from one try to the next, however often.
 * WHY it does not grow: a person waits for the picture. The roles whose
 * pause grows (the gatherer, the transcriber) are Rust's alone.
 */
export const VIEWER_PAUSE: Pause = {
	firstMs: DEFAULT_RECONNECT_DELAY_MS,
	mostMs: DEFAULT_RECONNECT_DELAY_MS,
};

/** How a session ended, for the pause before the next. */
export type SessionEnding =
	/** A GOAWAY ended it (transport §10.4), or one on the stream of what it played. */
	| "moved"
	/** It was made, and ended. */
	| "ended"
	/** It could not be made, or broke with an error. */
	| "failed";

/** The pause before the next session (`createNextSession`). */
export interface NextSession {
	/** The session that was tried last ended so at `nowMs`: the pause before the next try, which begins after it. */
	after(ending: SessionEnding, nowMs: number): number;
}

/**
 * The pause before each next try at a session: what is left of the
 * `pause` of this try since the last try began, so that a session that
 * lasted longer is followed at once; and none after one that a GOAWAY
 * ended, where the try before was no such one, which leaves the pause as it
 * was. Tested with `fixtures/ask-again/next-session.tsv`, as `NextSession`
 * of `xmoq-again` is.
 *
 * WHY from the try, and not from the end: a session that broke after an
 * hour is one break, and its next is made at once; one that cannot be made
 * is tried once in a pause. WHY none after a GOAWAY: what was played or
 * published stands still until the next session, and the relay that sent
 * it may close the old one before its Timeout is over (transport §10.4:
 * "This is a hint"). WHY not twice in a row: a relay that sends GOAWAY on
 * every session would have the next made as fast as it can. WHY the same
 * after a session that ended and one that failed: a session ends when its
 * relay goes, and the next one fails until the relay is back; the two are
 * one break.
 */
export function createNextSession(
	pause: Pause,
	firstTriedMs: number,
): NextSession {
	let tried = firstTriedMs;
	/** The try that was last began at once, after a GOAWAY. */
	let atOnce = false;
	/** The pause of the next try that is not made at once. */
	let nextMs = pause.firstMs;
	return {
		after(ending, nowMs) {
			atOnce = ending === "moved" && !atOnce;
			const lasted = Math.max(0, nowMs - tried);
			let wait = 0;
			if (!atOnce) {
				if (pause.wentWellMs !== undefined && lasted >= pause.wentWellMs) {
					nextMs = pause.firstMs;
				}
				wait = Math.max(0, nextMs - lasted);
				nextMs = Math.min(nextMs * 2, pause.mostMs);
			}
			tried = nowMs + wait;
			return wait;
		},
	};
}
