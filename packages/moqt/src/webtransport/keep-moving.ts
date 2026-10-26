// One session after the other, following GOAWAY (transport §10.4): when a
// session ends, or cannot be made, the next is made after the pause of
// `createNextSession`, and a GOAWAY ends the session and has the next made
// where it says. The players and publishers of @xmoq/media, the chat's
// participant and the transcript's reader go on by it. Why here, at the top
// of `src/core/again.ts`.

import {
	createNextSession,
	DEFAULT_RECONNECT_DELAY_MS,
	type SessionEnding,
} from "../core/again.ts";
import { RequestErrorCode } from "../core/error.ts";
import { DEFAULT_ANSWER_WAIT_MS } from "../core/limits.ts";
import type { GoAwayNotice, MoqtConnection } from "./connection.ts";
import { RequestFailedError, type SessionClose } from "./endings.ts";

/** What `keepMoving` needs of a connection. */
export type MovingConnection = Pick<
	MoqtConnection,
	"goAway" | "closed" | "close"
>;

/**
 * Resolves after `ms` milliseconds, or at once where `signal` is or gets
 * aborted. Never rejects. WHY the listener is taken off when the time is
 * over: a signal that lasts as long as the page would otherwise gather one
 * for every pause.
 */
export const waitFor = (ms: number, signal: AbortSignal): Promise<void> =>
	new Promise((resolve) => {
		if (signal.aborted) {
			resolve();
			return;
		}
		const timer = setTimeout(done, ms);
		function done(): void {
			clearTimeout(timer);
			signal.removeEventListener("abort", done);
			resolve();
		}
		signal.addEventListener("abort", done, { once: true });
	});

/**
 * The GOAWAY behind a refusal with GOING_AWAY, or undefined where `error`
 * is no such refusal or no GOAWAY comes. Transport §10.4: requests "that
 * arrive after the GOAWAY, MUST be rejected with REQUEST_ERROR using error
 * code GOING_AWAY"; the GOAWAY travels on the control stream and the refusal
 * on the request's, between which QUIC keeps no order, so the refusal can be
 * read first. WHY a limit, the time an answer gets (`waitMs`,
 * `DEFAULT_ANSWER_WAIT_MS` by default): GOING_AWAY also comes from a peer
 * that received a GOAWAY (§10.6.2) and sends none. The Rust clients wait in
 * the same way (`client::go_away_behind` of `xmoq-moqt-io`); both are tested
 * with `fixtures/ask-again/go-away-behind.tsv`.
 */
export async function goAwayBehind(
	connection: Pick<MoqtConnection, "goAway" | "closed">,
	error: unknown,
	waitMs: number = DEFAULT_ANSWER_WAIT_MS,
): Promise<GoAwayNotice | undefined> {
	if (
		!(error instanceof RequestFailedError) ||
		error.errorCode !== RequestErrorCode.GOING_AWAY
	) {
		return undefined;
	}
	const given = new AbortController();
	try {
		return await Promise.race([
			connection.goAway,
			connection.closed.then(() => undefined),
			waitFor(waitMs, given.signal).then(() => undefined),
		]);
	} finally {
		given.abort();
	}
}

/**
 * **Provisional**: how long the next session waits, after a GOAWAY ended
 * the one before, for the `run` of that one to end on its closed connection.
 * WHY wait: a `run` that is still under way when the next begins keeps the
 * state of the caller (who is in a meeting, what plays) and changes it
 * after the next has begun. WHY a bound: a `run` that does not end when its
 * connection closes would hold the move; the requests of a closed
 * connection reject at once, so a second is long.
 */
export const RUN_END_WAIT_MS = 1000;

/** What `connect` gives: a connection, and whatever else `run` needs of that session (a token, say). */
export interface MovingSession {
	readonly connection: MovingConnection;
}

/** How one session of `keepMoving` ended. */
export type MovingOutcome<R> =
	/** `run` resolved with `result`, and no GOAWAY came for it. */
	| { type: "done"; result: R }
	/** A GOAWAY ended the session; every `connect` from here is handed it until another comes. */
	| { type: "moved"; goAway: GoAwayNotice }
	/** `connect` or `run` rejected, with no GOAWAY behind it. */
	| { type: "failed"; error: unknown };

export interface KeepMovingOptions<S extends MovingSession, R> {
	/**
	 * Makes a connection; called anew for every session, so that a caller
	 * can ask anew where to go and for tokens. `goAway` is the last GOAWAY
	 * that ended a session: transport §10.4 has "The client MUST use this
	 * URI for the new session if provided" (`newSessionUri`, empty where
	 * the current one is reused). **It stays the same for every try until
	 * another GOAWAY comes**, also after a session or a try at the new URI
	 * failed. WHY: the relay that sent it is going away and refuses what
	 * comes (`GOING_AWAY`); a viewer that went back to it after one failure
	 * at the new URI was left there (an interop run, 2026-10-24).
	 */
	connect(goAway?: GoAwayNotice): Promise<S>;
	/**
	 * What is done on the session; resolves when that is over (the session
	 * ended, or what it played did). A GOAWAY on the connection ends it, and
	 * the session is closed under it. `signal` is the caller's.
	 */
	run(session: S, signal: AbortSignal): Promise<R>;
	/** The GOAWAY a result of `run` stands for, where it is one (one on the stream of what was played). */
	movedBy?(result: R): GoAwayNotice | undefined;
	/** The least time from one try at a session to the next: `DEFAULT_RECONNECT_DELAY_MS` when left out; none once after a GOAWAY (`createNextSession`). */
	againMs?: number;
	/** Aborting it closes the session and ends the trying. */
	signal?: AbortSignal;
	/** How a pause is waited: `waitFor` when left out. For a test that counts the pauses. */
	wait?(ms: number, signal: AbortSignal): Promise<void>;
	/** The clock the pauses are counted by, in milliseconds: `Date.now` when left out. For a test, with `wait`. */
	now?(): number;
	onAttempt?(attempt: number): void;
	/** After every session, also the one the caller's `signal` ended. */
	onOutcome?(outcome: MovingOutcome<R>): void;
}

/**
 * Runs sessions one after the other for as long as `signal` lets it.
 * Resolves once `signal` was aborted and the session that ran then is
 * over; never rejects.
 *
 * - **A GOAWAY ends the session with NO_ERROR** (§3.6), also one that comes
 *   while `run` waits for an answer, one that came before `run` resolved,
 *   and one read after a refusal with GOING_AWAY (`goAwayBehind`); the next
 *   `connect` is handed it. WHY the session ends before the next is made,
 *   where §3.6 has the client make it "in the background" and close the
 *   old one when no subscription is left on it: two sessions would play
 *   the same tracks at once, or be two publishers of one track at the
 *   relay, and the gap is the time of one connection. WHY NOT wait for the
 *   relay to close: it may wait for as long as its Timeout says, and a
 *   full relay that sends a session away (`--when-full`) closes it only
 *   then.
 * - Every session is closed when it is over, also one on which `run`
 *   failed; after a GOAWAY, the next is made once the `run` on the closed
 *   connection has ended, `RUN_END_WAIT_MS` at most.
 */
export async function keepMoving<S extends MovingSession, R>(
	options: KeepMovingOptions<S, R>,
): Promise<void> {
	const signal = options.signal ?? new AbortController().signal;
	const wait = options.wait ?? waitFor;
	const now = options.now ?? Date.now;
	const againMs = options.againMs ?? DEFAULT_RECONNECT_DELAY_MS;
	// A pause that does not grow, as `VIEWER_PAUSE`: every client of this
	// loop is a page that a person waits on.
	const next = createNextSession({ firstMs: againMs, mostMs: againMs }, now());
	let movedBy: GoAwayNotice | undefined;

	const session = async (): Promise<MovingOutcome<R>> => {
		let made: S;
		try {
			made = await options.connect(movedBy);
		} catch (error) {
			return { type: "failed", error };
		}
		const open = made.connection;
		let goneAway: GoAwayNotice | undefined;
		const goAway = open.goAway.then((notice) => {
			goneAway = notice;
			return notice;
		});
		const stop = (): void => open.close();
		signal.addEventListener("abort", stop, { once: true });
		let ran: Promise<R> | undefined;
		try {
			if (signal.aborted) stop();
			ran = options.run(made, signal);
			const first = await Promise.race([
				ran.then((result) => ({ result })),
				goAway.then((notice) => ({ notice })),
			]);
			if ("notice" in first) return { type: "moved", goAway: first.notice };
			// WHY `goneAway` too: a `run` that resolves with the end of the
			// session can win the race against a GOAWAY that came just before
			// the relay closed.
			const notice = goneAway ?? options.movedBy?.(first.result);
			return notice === undefined
				? { type: "done", result: first.result }
				: { type: "moved", goAway: notice };
		} catch (error) {
			// After a GOAWAY the requests of the connection reject: that is the
			// move, not a failure.
			const notice = goneAway ?? (await goAwayBehind(open, error));
			return notice === undefined
				? { type: "failed", error }
				: { type: "moved", goAway: notice };
		} finally {
			signal.removeEventListener("abort", stop);
			open.close();
			if (ran !== undefined) {
				const given = new AbortController();
				await Promise.race([
					ran.then(
						() => {},
						() => {},
					),
					waitFor(RUN_END_WAIT_MS, given.signal),
				]);
				given.abort();
			}
		}
	};

	for (let attempt = 1; !signal.aborted; attempt++) {
		options.onAttempt?.(attempt);
		const outcome = await session();
		if (outcome.type === "moved") movedBy = outcome.goAway;
		options.onOutcome?.(outcome);
		if (signal.aborted) return;
		await wait(next.after(endingOf(outcome), now()), signal);
	}
}

/**
 * The outcome of a session whose `run` resolves with its end
 * (`connection.closed`), as the callers that keep one say it: `done` is
 * `session-ended`.
 */
export type SessionMovingOutcome =
	| { type: "session-ended"; close: SessionClose }
	| Exclude<MovingOutcome<SessionClose>, { type: "done" }>;

export function sessionMovingOutcome(
	outcome: MovingOutcome<SessionClose>,
): SessionMovingOutcome {
	return outcome.type === "done"
		? { type: "session-ended", close: outcome.result }
		: outcome;
}

function endingOf<R>(outcome: MovingOutcome<R>): SessionEnding {
	switch (outcome.type) {
		case "moved":
			return "moved";
		case "failed":
			return "failed";
		case "done":
			return "ended";
	}
}
