import { expect, test } from "bun:test";
import { resolveLimits, type SessionLimits } from "../../src/core/limits.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex, isStreamTiming, utf8 } from "../helpers.ts";

/** Objects of 16 bytes at most; the streams of the session hold 20 together. */
const LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16,
	maxObjectPropertiesLength: 16,
	maxPeerRequests: 64,
	maxPeerFetches: 16,
	maxHeldSubgroupBytes: 1024,
	maxSessionSubgroupBytes: 20,
};

/** SUBGROUP_HEADER with Subgroup ID 0 and no properties, Track Alias 9, Group 0. */
const HEADER = "14 09 00 00 00";

/** `length` bytes of payload. */
const payload = (length: number): string =>
	Array.from({ length }, () => "61").join(" ");

/** What an output is, in a line, for the outputs a test looks at. */
function summary(output: SessionOutput): string {
	switch (output.type) {
		case "stop-sending":
			return `stop ${output.peerStream} with ${output.code}`;
		case "event":
			return output.event.type === "subgroup-abandoned"
				? `abandoned: ${output.event.reason}`
				: output.event.type;
		default:
			return output.type;
	}
}

/** A session with the subscription of Track Alias 9 Established. */
function subscribed(): Session {
	const session = createSession({ limits: LIMITS });
	session.start();
	session.subscribe({
		trackNamespace: [utf8("xmoq"), utf8("bytes")],
		trackName: utf8("t"),
	});
	session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
	return session;
}

type Step =
	/** A stream opens with the header and the first `held` bytes of an Object of 16. */
	| { type: "begin"; stream: number; held: number }
	/** `more` bytes of the Object of the stream come. */
	| { type: "more"; stream: number; more: number }
	| { type: "reset"; stream: number };

const run = (session: Session, step: Step): SessionOutput[] => {
	switch (step.type) {
		case "begin":
			return session.receivePeerUniStreamData(
				step.stream,
				hex(`${HEADER} 00 10 ${payload(step.held)}`),
				false,
			);
		case "more":
			return session.receivePeerUniStreamData(
				step.stream,
				hex(payload(step.more)),
				false,
			);
		case "reset":
			return session.peerUniStreamReset(step.stream);
	}
};

// The bytes counted are those of Objects that have not arrived whole: a
// whole Object is handed on, and what a stream that ends held is given back.
test.each<{ name: string; steps: Step[]; last: string[] }>([
	{
		name: "streams that hold the limit together go on",
		steps: [
			{ type: "begin", stream: 0, held: 8 },
			{ type: "begin", stream: 1, held: 8 },
			{ type: "begin", stream: 2, held: 4 },
		],
		last: [],
	},
	{
		name: "the stream whose bytes take the session over is stopped with INTERNAL_ERROR",
		steps: [
			{ type: "begin", stream: 0, held: 8 },
			{ type: "begin", stream: 1, held: 8 },
			{ type: "begin", stream: 2, held: 5 },
		],
		last: [
			"stop 2 with 0",
			"abandoned: the subgroup streams of the session hold too much",
		],
	},
	{
		name: "an Object that came whole is room for another stream",
		steps: [
			{ type: "begin", stream: 0, held: 8 },
			{ type: "begin", stream: 1, held: 8 },
			{ type: "more", stream: 0, more: 8 },
			{ type: "begin", stream: 2, held: 12 },
		],
		last: [],
	},
	{
		name: "a stream the peer reset gives its bytes back",
		steps: [
			{ type: "begin", stream: 0, held: 8 },
			{ type: "begin", stream: 1, held: 8 },
			{ type: "reset", stream: 0 },
			{ type: "begin", stream: 2, held: 12 },
		],
		last: [],
	},
	{
		name: "a stream that was stopped gives its bytes back",
		steps: [
			{ type: "begin", stream: 0, held: 8 },
			{ type: "begin", stream: 1, held: 8 },
			{ type: "begin", stream: 2, held: 5 },
			{ type: "begin", stream: 3, held: 4 },
		],
		last: [],
	},
])("$name, and the session goes on", ({ steps, last }) => {
	const session = subscribed();
	let outputs: SessionOutput[] = [];
	for (const step of steps) outputs = run(session, step);
	expect(
		outputs
			.filter((output) => !isStreamTiming(output))
			.filter(
				(output) => output.type !== "event" || output.event.type !== "object",
			)
			.map(summary),
	).toEqual(last);
	expect(session.closed).toBe(false);
});

test.each([0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY])(
	"a session limit of %p is refused with a RangeError: it cannot be left unbounded",
	(maxSessionSubgroupBytes) => {
		expect(() => resolveLimits({ maxSessionSubgroupBytes })).toThrow(
			RangeError,
		);
	},
);
