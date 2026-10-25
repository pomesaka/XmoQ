import { expect, test } from "bun:test";
import type { GoAwayNotice, SessionClose } from "@xmoq/moqt";
import {
	type KeptTranscriptState,
	keepFollowingTranscript,
	type TranscriptConnection,
} from "../src/moqt/keep-following.ts";

/** What a session of the test does once it is made: the relay closes it, or sends GOAWAY to a URI. */
type Session = "closes" | { goAway: string };

/**
 * A session that does what `session` says. The SUBSCRIBE for `speech` is
 * refused, so that no FETCH follows; `close` ends the session, as the
 * library's does.
 */
function sessionOf(session: Session): TranscriptConnection {
	const closed = Promise.withResolvers<SessionClose>();
	const goAway = Promise.withResolvers<GoAwayNotice>();
	const close = (): void =>
		closed.resolve({ type: "local", code: 0, reason: "" });
	setTimeout(() => {
		if (session === "closes") close();
		else goAway.resolve({ newSessionUri: session.goAway, timeoutMs: 2_000 });
	}, 0);
	return {
		subscribe: () => Promise.reject(new Error("refused by the test")),
		fetch: () => Promise.reject(new Error("no FETCH in this test")),
		close,
		closed: closed.promise,
		goAway: goAway.promise,
	};
}

/** A state as the table writes it. */
const written = (state: KeptTranscriptState): string => {
	switch (state.type) {
		case "connecting":
			return `connecting ${state.attempt}`;
		case "reading":
			return `reading ${state.session}`;
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
		sessions: ["closes", "closes"],
		connects: ["-", "-"],
		states: [
			"connecting 1",
			"reading 1",
			"session-ended",
			"connecting 2",
			"reading 2",
		],
	},
	{
		name: "a GOAWAY moves it to its URI",
		sessions: [{ goAway: "https://b" }, "closes"],
		connects: ["-", "https://b"],
		states: [
			"connecting 1",
			"reading 1",
			"moved https://b",
			"connecting 2",
			"reading 2",
		],
	},
	{
		name: "the GOAWAY stays the one it goes by after a session ends there",
		sessions: [{ goAway: "https://b" }, "closes", "closes"],
		connects: ["-", "https://b", "https://b"],
		states: [
			"connecting 1",
			"reading 1",
			"moved https://b",
			"connecting 2",
			"reading 2",
			"session-ended",
			"connecting 3",
			"reading 3",
		],
	},
	{
		name: "a second GOAWAY moves it again",
		sessions: [{ goAway: "https://b" }, { goAway: "https://c" }, "closes"],
		connects: ["-", "https://b", "https://c"],
		states: [
			"connecting 1",
			"reading 1",
			"moved https://b",
			"connecting 2",
			"reading 2",
			"moved https://c",
			"connecting 3",
			"reading 3",
		],
	},
])("$name", async ({ sessions, connects, states }) => {
	const stop = new AbortController();
	const toldTo: string[] = [];
	const seen: string[] = [];
	const made = [...sessions];
	await keepFollowingTranscript({
		async connect(goAway) {
			toldTo.push(goAway === undefined ? "-" : goAway.newSessionUri);
			const session = made.shift();
			if (session === undefined) throw new Error("no more sessions");
			return sessionOf(session);
		},
		follow: { listened: ["xmoq", "demo"], onSpeech() {} },
		againMs: 1,
		signal: stop.signal,
		onState(state) {
			seen.push(written(state));
			if (seen.length === states.length) stop.abort();
		},
	});
	expect({ toldTo, seen }).toEqual({ toldTo: connects, seen: states });
});
