import { expect, test } from "bun:test";
import {
	type GoAwayNotice,
	type MovingConnection,
	type NamespaceSubscriptionEnd,
	RequestErrorCode,
	RequestFailedError,
} from "@xmoq/moqt";
import { type AttendingState, keepAttending } from "../src/index.ts";

/** What a session of the test does. */
type Session =
	/** The SUBSCRIBE_NAMESPACE is taken, and then finished by the relay. */
	| "ends"
	/** It is taken, and the relay sends GOAWAY to `uri` while the meeting is attended. */
	| { goAway: string }
	/** It is refused, with no GOAWAY. */
	| "refused"
	/** It is refused with GOING_AWAY, and the GOAWAY to `uri` comes 10 ms after. */
	| { goingAwayThenGoAway: string }
	/** It is taken, and the caller stops while the meeting is attended. */
	| "stopped";

interface FakeConnection extends MovingConnection {
	readonly does: Session;
	sendGoAway(uri: string): void;
}

function connectionOf(does: Session): FakeConnection {
	const closed = Promise.withResolvers<{
		type: "local";
		code: number;
		reason: string;
	}>();
	const goAway = Promise.withResolvers<GoAwayNotice>();
	return {
		does,
		sendGoAway: (uri) =>
			goAway.resolve({ newSessionUri: uri, timeoutMs: 2_000 }),
		close: () => closed.resolve({ type: "local", code: 0, reason: "" }),
		closed: closed.promise,
		goAway: goAway.promise,
	};
}

/** A state as the table writes it. */
const written = (state: AttendingState<FakeConnection>): string => {
	switch (state.type) {
		case "connecting":
			return `connecting ${state.attempt}`;
		case "attending":
			return `attending ${state.session}`;
		case "attending-ended":
			return `ended ${state.ended.type}`;
		case "moved":
			return `moved ${state.goAway.newSessionUri}`;
		case "failed":
			return "failed";
	}
};

test.each<{
	name: string;
	sessions: Session[];
	/** Where each `connect` was told to go: the URI of the GOAWAY it was handed, `-` for none. */
	connects: string[];
	/** The states, until the test stops it after the last one (or the last session stops it). */
	states: string[];
}>([
	{
		name: "an attending that ends is followed by another, on a new session where it was",
		sessions: ["ends", "ends"],
		connects: ["-", "-"],
		states: [
			"connecting 1",
			"attending 1",
			"ended peer-closed",
			"connecting 2",
			"attending 2",
			"ended peer-closed",
		],
	},
	{
		name: "a GOAWAY moves it, and the next session is made where it says",
		sessions: [{ goAway: "https://b" }, "ends"],
		connects: ["-", "https://b"],
		states: [
			"connecting 1",
			"attending 1",
			"moved https://b",
			"connecting 2",
			"attending 2",
			"ended peer-closed",
		],
	},
	{
		name: "a refusal is a failure, it is not counted as attended, and it is tried again where it was",
		sessions: ["refused", "ends"],
		connects: ["-", "-"],
		states: [
			"connecting 1",
			"failed",
			"connecting 2",
			"attending 1",
			"ended peer-closed",
		],
	},
	{
		name: "a refusal with GOING_AWAY waits for the GOAWAY behind it, and moves",
		sessions: [{ goingAwayThenGoAway: "https://b" }, "ends"],
		connects: ["-", "https://b"],
		states: [
			"connecting 1",
			"moved https://b",
			"connecting 2",
			"attending 1",
			"ended peer-closed",
		],
	},
	{
		name: "a caller that stops is told of no end and no failure",
		sessions: ["stopped"],
		connects: ["-"],
		states: ["connecting 1", "attending 1"],
	},
])("$name", async ({ sessions, connects, states }) => {
	const stop = new AbortController();
	const toldTo: string[] = [];
	const seen: string[] = [];
	const made = [...sessions];
	const ended: NamespaceSubscriptionEnd = { type: "peer-closed" };
	await keepAttending<FakeConnection>({
		meeting: {
			async attend(connection, _signal, onTaken) {
				const { does } = connection;
				if (does === "refused") throw new Error("refused by the test");
				if (typeof does === "object" && "goingAwayThenGoAway" in does) {
					setTimeout(() => connection.sendGoAway(does.goingAwayThenGoAway), 10);
					throw new RequestFailedError({
						errorCode: RequestErrorCode.GOING_AWAY,
						retryInterval: 0n,
						errorReason: "the session goes away",
					});
				}
				onTaken();
				if (does === "ends") return ended;
				if (does === "stopped") {
					setTimeout(() => stop.abort(), 0);
				} else {
					setTimeout(() => connection.sendGoAway(does.goAway), 0);
				}
				// What attending does when its connection closes.
				await connection.closed;
				return { type: "cancelled" };
			},
		},
		async connect(goAway) {
			toldTo.push(goAway === undefined ? "-" : goAway.newSessionUri);
			const does = made.shift();
			if (does === undefined) throw new Error("no more sessions");
			return connectionOf(does);
		},
		againMs: 1,
		signal: stop.signal,
		onState(state) {
			seen.push(written(state));
			if (seen.length === states.length && made.length === 0) {
				// The last session that stops says nothing more; the others end here.
				if (sessions.at(-1) !== "stopped") stop.abort();
			}
		},
	});
	expect({ toldTo, seen }).toEqual({ toldTo: connects, seen: states });
});
