import { expect, test } from "bun:test";
import type {
	GoAwayNotice,
	ReceivedObject,
	SessionClose,
	SubscriptionEnd,
} from "@xmoq/moqt";
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

/**
 * A session whose subscription to `speech` says `largest` was published and
 * brings the piece `live` (not one that reads as a piece; the reader takes
 * its number all the same), then is closed by the relay. Its FETCH, if one
 * is asked for, is refused, and its Joining Start noted in `starts`.
 */
function sessionWith(
	largest: bigint | undefined,
	live: bigint | undefined,
	starts: bigint[],
): TranscriptConnection {
	const closed = Promise.withResolvers<SessionClose>();
	const close = (): void =>
		closed.resolve({ type: "local", code: 0, reason: "" });
	const objects = new ReadableStream<ReceivedObject>({
		start(controller) {
			if (live !== undefined) {
				controller.enqueue({
					groupId: live,
					subgroupId: 0n,
					publisherPriority: undefined,
					object: {
						objectId: 0n,
						properties: [],
						status: "Normal",
						payload: new Uint8Array(),
					},
				});
			}
			setTimeout(() => {
				controller.close();
				close();
			}, 10);
		},
	});
	return {
		subscribe: async () => ({
			requestId: 0n,
			trackAlias: 0n,
			parameters:
				largest === undefined
					? {}
					: { largestObject: { group: largest, object: 0n } },
			trackProperties: [],
			objects,
			ended: closed.promise.then(
				(close): SubscriptionEnd => ({ type: "session-closed", close }),
			),
			goAway: new Promise(() => {}),
			cancel: async () => {},
			update: () => Promise.reject(new Error("no update in this test")),
		}),
		async fetch(request) {
			if (request.range.fetchType === "AbsoluteJoining") {
				starts.push(request.range.joiningStart);
			}
			throw new Error("refused by the test");
		},
		close,
		closed: closed.promise,
		goAway: new Promise(() => {}),
	};
}

// Q4 of the plan (decided on 2026-10-26): a transcript is read on from where
// the session before left it, as the agent reads it, and not the latest
// pieces anew.
test("the next session's FETCH begins after the newest piece the session before had", async () => {
	const stop = new AbortController();
	const starts: bigint[] = [];
	const made: [bigint | undefined, bigint | undefined][] = [
		[undefined, 7n],
		[9n, undefined],
	];
	let reading = 0;
	await keepFollowingTranscript({
		connect: async () => {
			const session = made.shift();
			if (session === undefined) throw new Error("no more sessions");
			// Made when it is connected: its relay closes it 10 ms after.
			return sessionWith(...session, starts);
		},
		follow: { listened: ["xmoq", "demo"], onSpeech() {} },
		againMs: 1,
		signal: stop.signal,
		onState(state) {
			if (state.type === "reading") reading += 1;
			if (state.type === "session-ended" && reading === 2) stop.abort();
		},
	});
	expect(starts).toEqual([8n]);
});
