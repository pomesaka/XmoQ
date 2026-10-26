import { expect, test } from "bun:test";
import { RequestErrorCode } from "../../src/core/error.ts";
import type { GoAwayNotice } from "../../src/webtransport/connection.ts";
import { RequestFailedError } from "../../src/webtransport/endings.ts";
import {
	keepMoving,
	keepSessions,
	type MovingConnection,
	type MovingOutcome,
} from "../../src/webtransport/keep-moving.ts";

/** What a session of the test does. */
type Session =
	/** `connect` rejects. */
	| "cannot-connect"
	/** `run` resolves with "over". */
	| "runs"
	/** `run` resolves with "over" after the clock moved by 5 s. */
	| "runs-long"
	/** `run` resolves with "track moved", which `movedBy` takes for a GOAWAY to `https://t`. */
	| "track-moves"
	/** `run` rejects with an Error. */
	| "breaks"
	/** The GOAWAY to `uri` comes while `run` waits. */
	| { goAway: string }
	/** `run` is refused with GOING_AWAY, and the GOAWAY to `uri` comes 10 ms after. */
	| { refusedThenGoAway: string }
	/** `run` is refused with GOING_AWAY, and the session closes 10 ms after. */
	| "refused-then-closes"
	/** The GOAWAY to `uri` comes, and `run` resolves before the race has seen it. */
	| { goAwayAsItEnds: string }
	/** The GOAWAY to `uri` comes, and `run` ends 20 ms after the connection closed. */
	| { goAwayEndsLate: string };

/** A session as `connect` gives it: the connection, and what the test is to do on it. */
interface FakeSession {
	connection: FakeConnection;
	does: Session;
}

interface FakeConnection extends MovingConnection {
	resolveGoAway(uri: string): void;
}

const notice = (uri: string): GoAwayNotice => ({
	newSessionUri: uri,
	timeoutMs: 2_000,
});

const goingAway = (): RequestFailedError =>
	new RequestFailedError({
		errorCode: RequestErrorCode.GOING_AWAY,
		retryInterval: 0n,
		errorReason: "the session goes away",
	});

function connectionOf(log: string[]): FakeConnection {
	const closed = Promise.withResolvers<{
		type: "local";
		code: number;
		reason: string;
	}>();
	const goAway = Promise.withResolvers<GoAwayNotice>();
	let open = true;
	return {
		resolveGoAway: (uri) => goAway.resolve(notice(uri)),
		close() {
			if (!open) return;
			open = false;
			log.push("close");
			closed.resolve({ type: "local", code: 0, reason: "" });
		},
		closed: closed.promise,
		goAway: goAway.promise,
	};
}

const outcomeOf = (outcome: MovingOutcome<string>): string => {
	switch (outcome.type) {
		case "done":
			return `done ${outcome.result}`;
		case "moved":
			return `moved ${outcome.goAway.newSessionUri}`;
		case "failed":
			return "failed";
	}
};

test.each<{
	name: string;
	sessions: Session[];
	/** Where each `connect` was told to go: the URI of the GOAWAY it was handed, `-` for none. */
	connects: string[];
	/** What was done, and each pause before the next try, with `againMs` 1,000 and a clock that moves by the pauses alone, unless a session says otherwise. */
	log: string[];
}>([
	{
		name: "a session that ends is closed, and the next is made where it was, 1 s after the try before",
		sessions: ["runs", "runs"],
		connects: ["-", "-"],
		log: ["close", "done over", "wait 1000", "close", "done over"],
	},
	{
		name: "a connection that cannot be made is tried again, after the same pause",
		sessions: ["cannot-connect", "runs"],
		connects: ["-", "-"],
		log: ["failed", "wait 1000", "close", "done over"],
	},
	{
		name: "a session that lasted longer than the pause is followed at once",
		sessions: ["runs-long", "runs"],
		connects: ["-", "-"],
		log: ["close", "done over", "wait 0", "close", "done over"],
	},
	{
		name: "a GOAWAY while it runs moves it at once, and the next session is handed it",
		sessions: [{ goAway: "https://b" }, "runs"],
		connects: ["-", "https://b"],
		log: ["close", "moved https://b", "wait 0", "close", "done over"],
	},
	{
		name: "a second GOAWAY in a row is followed after the pause, and a third at once",
		sessions: [
			{ goAway: "https://b" },
			{ goAway: "https://c" },
			{ goAway: "https://d" },
			"runs",
		],
		connects: ["-", "https://b", "https://c", "https://d"],
		log: [
			"close",
			"moved https://b",
			"wait 0",
			"close",
			"moved https://c",
			"wait 1000",
			"close",
			"moved https://d",
			"wait 0",
			"close",
			"done over",
		],
	},
	{
		name: "a refusal with GOING_AWAY waits for the GOAWAY behind it, and moves",
		sessions: [{ refusedThenGoAway: "https://b" }, "runs"],
		connects: ["-", "https://b"],
		log: ["close", "moved https://b", "wait 0", "close", "done over"],
	},
	{
		name: "a refusal with GOING_AWAY and no GOAWAY before the session closes is a failure where it was",
		sessions: ["refused-then-closes", "runs"],
		connects: ["-", "-"],
		log: ["close", "failed", "wait 1000", "close", "done over"],
	},
	{
		name: "after a GOAWAY the next session waits for the run on the closed connection to end",
		sessions: [{ goAwayEndsLate: "https://b" }, "runs"],
		connects: ["-", "https://b"],
		log: [
			"close",
			"run ended",
			"moved https://b",
			"wait 0",
			"close",
			"done over",
		],
	},
	{
		name: "a GOAWAY that came as `run` resolved is a move",
		sessions: [{ goAwayAsItEnds: "https://b" }, "runs"],
		connects: ["-", "https://b"],
		log: ["close", "moved https://b", "wait 0", "close", "done over"],
	},
	{
		name: "a result `movedBy` names a GOAWAY for is a move",
		sessions: ["track-moves", "runs"],
		connects: ["-", "https://t"],
		log: ["close", "moved https://t", "wait 0", "close", "done over"],
	},
	{
		name: "the GOAWAY stays the one it goes by after a session fails or ends there",
		sessions: [{ goAway: "https://b" }, "breaks", "runs", "runs"],
		connects: ["-", "https://b", "https://b", "https://b"],
		log: [
			"close",
			"moved https://b",
			"wait 0",
			"close",
			"failed",
			"wait 1000",
			"close",
			"done over",
			"wait 1000",
			"close",
			"done over",
		],
	},
])("$name", async ({ sessions, connects, log }) => {
	const stop = new AbortController();
	const toldTo: string[] = [];
	const seen: string[] = [];
	const made = [...sessions];
	let clock = 0;
	let outcomes = 0;
	await keepMoving<FakeSession, string>({
		async connect(goAway) {
			toldTo.push(goAway === undefined ? "-" : goAway.newSessionUri);
			const does = made.shift();
			if (does === undefined || does === "cannot-connect") {
				throw new Error("no relay");
			}
			return { connection: connectionOf(seen), does };
		},
		async run({ connection, does }) {
			if (does === "runs") return "over";
			if (does === "runs-long") {
				clock += 5_000;
				return "over";
			}
			if (does === "track-moves") return "track moved";
			if (does === "breaks") throw new Error("broken");
			if (does === "refused-then-closes") {
				setTimeout(() => connection.close(), 10);
				throw goingAway();
			}
			if (typeof does === "object" && "refusedThenGoAway" in does) {
				setTimeout(() => connection.resolveGoAway(does.refusedThenGoAway), 10);
				throw goingAway();
			}
			if (typeof does === "object" && "goAwayAsItEnds" in does) {
				// `run` settles in the turn after the GOAWAY is taken, and before
				// the race sees the GOAWAY: it wins the race.
				connection.resolveGoAway(does.goAwayAsItEnds);
				return "over";
			}
			if (typeof does === "object" && "goAway" in does) {
				setTimeout(() => connection.resolveGoAway(does.goAway), 0);
			}
			if (typeof does === "object" && "goAwayEndsLate" in does) {
				setTimeout(() => connection.resolveGoAway(does.goAwayEndsLate), 0);
				// What it does on the session goes on for a while after the
				// connection closed.
				await connection.closed;
				await new Promise((resolve) => setTimeout(resolve, 20));
				seen.push("run ended");
				return "late";
			}
			// What a run does when its connection closes: its requests reject.
			return connection.closed.then(() => "closed");
		},
		movedBy: (result) =>
			result === "track moved" ? notice("https://t") : undefined,
		againMs: 1_000,
		signal: stop.signal,
		async wait(ms) {
			seen.push(`wait ${ms}`);
			clock += ms;
		},
		now: () => clock,
		onOutcome(outcome) {
			seen.push(outcomeOf(outcome));
			outcomes += 1;
			if (outcomes === sessions.length) stop.abort();
		},
	});
	expect({ toldTo, seen }).toEqual({ toldTo: connects, seen: log });
});

test("aborting the signal closes the session that runs, and ends the trying", async () => {
	const stop = new AbortController();
	const seen: string[] = [];
	await keepMoving<{ connection: FakeConnection }, string>({
		connect: async () => ({ connection: connectionOf(seen) }),
		async run({ connection }) {
			setTimeout(() => stop.abort(), 0);
			await connection.closed;
			return "closed";
		},
		signal: stop.signal,
		onAttempt: (attempt) => seen.push(`attempt ${attempt}`),
		onOutcome: (outcome) => seen.push(outcomeOf(outcome)),
	});
	expect(seen).toEqual(["attempt 1", "close", "done closed"]);
});

test("aborting the signal while it waits makes no session after it", async () => {
	const stop = new AbortController();
	const seen: string[] = [];
	await keepMoving<{ connection: FakeConnection }, string>({
		connect: async () => ({ connection: connectionOf(seen) }),
		run: async () => "over",
		signal: stop.signal,
		async wait(ms) {
			seen.push(`wait ${ms}`);
			stop.abort();
		},
		now: () => 0,
		onAttempt: (attempt) => seen.push(`attempt ${attempt}`),
		onOutcome: (outcome) => seen.push(outcomeOf(outcome)),
	});
	expect(seen).toEqual(["attempt 1", "close", "done over", "wait 1000"]);
});

// `keepSessions`: the sessions are numbered as they begin, once each
// however often `began` is called, and the end of a session the caller's
// `signal` closed is not told; an end before that is.
test.each<{
	name: string;
	sessions: ("cannot-connect" | "runs" | "runs-not" | "caller-stops")[];
	/** The numbers `began` gave, and the outcomes told, in order. */
	log: string[];
}>([
	{
		name: "two sessions that began, and the third the caller stops",
		sessions: ["runs", "runs", "caller-stops"],
		log: [
			"began 1",
			"began 1",
			"done over",
			"began 2",
			"began 2",
			"done over",
			"began 3",
		],
	},
	{
		name: "a session that could not be made, or did not begin, gets no number",
		sessions: ["cannot-connect", "runs-not", "runs", "caller-stops"],
		log: ["failed", "done over", "began 1", "began 1", "done over", "began 2"],
	},
])("keepSessions: $name", async ({ sessions, log }) => {
	const told: string[] = [];
	const stop = new AbortController();
	const queue = [...sessions];
	await keepSessions<{ connection: FakeConnection; does: string }, string>({
		async connect() {
			const does = queue.shift() ?? "caller-stops";
			if (does === "cannot-connect") throw new Error("no connection");
			return { connection: connectionOf([]), does };
		},
		async run({ connection, does }, _signal, began) {
			if (does === "runs-not") return "over";
			told.push(`began ${began()}`);
			if (does === "caller-stops") {
				stop.abort();
				await connection.closed;
				return "stopped";
			}
			told.push(`began ${began()}`);
			return "over";
		},
		againMs: 1,
		signal: stop.signal,
		onOutcome: (outcome) => told.push(outcomeOf(outcome)),
	});
	expect(told).toEqual(log);
});
