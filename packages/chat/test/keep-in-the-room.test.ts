import { expect, test } from "bun:test";
import {
	type GoAwayNotice,
	RequestErrorCode,
	RequestFailedError,
	type SessionClose,
} from "@xmoq/moqt";
import {
	type ChatConnection,
	keepInTheRoom,
	type RoomState,
} from "../src/moqt/keep-in-the-room.ts";

/** What a session of the test does. */
type Session =
	/** The relay answers the PUBLISH_NAMESPACE, then closes the session. */
	| "joins-then-closes"
	/** The relay sends GOAWAY to `uri` before it answers the PUBLISH_NAMESPACE, and refuses it after. */
	| { goAwayBeforeAnswer: string }
	/** The refusal with GOING_AWAY is read before the GOAWAY to `uri`, which comes after it on the control stream. */
	| { refusedBeforeGoAway: string }
	/** The PUBLISH_NAMESPACE is refused with GOING_AWAY, no GOAWAY comes, and the session closes. */
	| "refused-going-away-then-closes"
	/** The relay answers, then sends GOAWAY to `uri`. */
	| { goAwayAfterJoined: string };

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

/**
 * A session that does what `session` says. The SUBSCRIBE for `lines` is
 * refused, so that no FETCH follows; `close` ends the session, as the
 * library's does.
 */
function sessionOf(session: Session): ChatConnection["connection"] {
	const closed = Promise.withResolvers<SessionClose>();
	const goAway = Promise.withResolvers<GoAwayNotice>();
	const close = (): void =>
		closed.resolve({ type: "local", code: 0, reason: "" });
	return {
		serve() {},
		async publishNamespace() {
			if (typeof session === "object" && "goAwayBeforeAnswer" in session) {
				goAway.resolve(notice(session.goAwayBeforeAnswer));
				throw goingAway();
			}
			if (typeof session === "object" && "refusedBeforeGoAway" in session) {
				setTimeout(
					() => goAway.resolve(notice(session.refusedBeforeGoAway)),
					10,
				);
				throw goingAway();
			}
			if (session === "refused-going-away-then-closes") {
				setTimeout(close, 10);
				throw goingAway();
			}
			if (session === "joins-then-closes") setTimeout(close, 0);
			else
				setTimeout(() => goAway.resolve(notice(session.goAwayAfterJoined)), 0);
		},
		subscribe: () => Promise.reject(new Error("refused by the test")),
		fetch: () => Promise.reject(new Error("no FETCH in this test")),
		close,
		closed: closed.promise,
		goAway: goAway.promise,
	};
}

/** A state as the table writes it. */
const written = (state: RoomState): string => {
	switch (state.type) {
		case "connecting":
			return `connecting ${state.attempt}`;
		case "joined":
			return `joined ${state.session}`;
		case "moved":
			return `moved ${state.goAway.newSessionUri}`;
		default:
			return state.type;
	}
};

test.each<{
	name: string;
	sessions: Session[];
	/** Where each `connect` was told to go: the URI of the GOAWAY it was handed, `-` for none. */
	connects: string[];
	states: string[];
}>([
	{
		name: "a session that ends is followed by one where it was",
		sessions: ["joins-then-closes", "joins-then-closes"],
		connects: ["-", "-"],
		states: [
			"connecting 1",
			"joined 1",
			"session-ended",
			"connecting 2",
			"joined 2",
		],
	},
	{
		name: "a GOAWAY before the namespace is answered moves it, and the session it moves to joins",
		sessions: [{ goAwayBeforeAnswer: "https://b" }, "joins-then-closes"],
		connects: ["-", "https://b"],
		states: ["connecting 1", "moved https://b", "connecting 2", "joined 1"],
	},
	{
		name: "a refusal with GOING_AWAY read before the GOAWAY waits for it, and moves",
		sessions: [{ refusedBeforeGoAway: "https://b" }, "joins-then-closes"],
		connects: ["-", "https://b"],
		states: ["connecting 1", "moved https://b", "connecting 2", "joined 1"],
	},
	{
		name: "a refusal with GOING_AWAY and no GOAWAY before the session closes is a failure where it was",
		sessions: ["refused-going-away-then-closes", "joins-then-closes"],
		connects: ["-", "-"],
		states: ["connecting 1", "failed", "connecting 2", "joined 1"],
	},
	{
		name: "a GOAWAY after it joined moves it as well",
		sessions: [{ goAwayAfterJoined: "https://b" }, "joins-then-closes"],
		connects: ["-", "https://b"],
		states: [
			"connecting 1",
			"joined 1",
			"moved https://b",
			"connecting 2",
			"joined 2",
		],
	},
	{
		name: "the GOAWAY stays the one it goes by after a session ends there",
		sessions: [
			{ goAwayBeforeAnswer: "https://b" },
			"joins-then-closes",
			"joins-then-closes",
		],
		connects: ["-", "https://b", "https://b"],
		states: [
			"connecting 1",
			"moved https://b",
			"connecting 2",
			"joined 1",
			"session-ended",
			"connecting 3",
			"joined 2",
		],
	},
])("$name", async ({ sessions, connects, states }) => {
	const stop = new AbortController();
	const toldTo: string[] = [];
	const seen: string[] = [];
	const made = [...sessions];
	await keepInTheRoom({
		async connect(goAway) {
			toldTo.push(goAway === undefined ? "-" : goAway.newSessionUri);
			const session = made.shift();
			if (session === undefined) throw new Error("no more sessions");
			return { connection: sessionOf(session) };
		},
		join: () => ({ room: "blue", name: "ann", onLine() {} }),
		againMs: 1,
		signal: stop.signal,
		onState(state) {
			seen.push(written(state));
			if (seen.length === states.length) stop.abort();
		},
	});
	expect({ toldTo, seen }).toEqual({ toldTo: connects, seen: states });
});
