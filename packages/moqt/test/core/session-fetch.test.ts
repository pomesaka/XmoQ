import { describe, expect, test } from "bun:test";
import {
	type ControlMessage,
	decodeControlMessage,
	encodeControlMessage,
	type FetchRange,
} from "../../src/core/control-message.ts";
import type { KeyValuePair } from "../../src/core/key-value-pair.ts";
import { endsBefore, type Location } from "../../src/core/parameters.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex, isStreamTiming, toHex, utf8 } from "../helpers.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

const at = (group: bigint, object: bigint): Location => ({ group, object });

const standalone = (start: Location, end: Location): FetchRange => ({
	fetchType: "Standalone",
	...TRACK,
	startLocation: start,
	endLocation: end,
});

function started(): Session {
	const session = createSession();
	session.start();
	return session;
}

/** A session with a FETCH for {1, 0} to {3, 0}: Request ID 0, on stream 1. */
function fetching(): Session {
	const session = started();
	session.fetch({ range: standalone(at(1n, 0n), at(3n, 0n)) });
	return session;
}

const fetchOk = (
	endLocation: Location,
	trackProperties: KeyValuePair[] = [],
): Uint8Array =>
	encodeControlMessage({
		type: "FETCH_OK",
		endOfTrack: false,
		endLocation,
		parameters: {},
		trackProperties,
	});

const REQUEST_ERROR = encodeControlMessage({
	type: "REQUEST_ERROR",
	errorCode: 0x11n,
	retryInterval: 0n,
	errorReason: "",
});

/**
 * The outputs in a few words each, so that a row of a table can say what it
 * expects. The outputs about the times of a stream (`isStreamTiming`) are
 * left out.
 */
function briefly(outputs: SessionOutput[]): string[] {
	return outputs.filter((output) => !isStreamTiming(output)).map(brief);
}

/** One output in a few words. */
function brief(output: SessionOutput): string {
	switch (output.type) {
		case "event": {
			const { event } = output;
			switch (event.type) {
				case "request-closed":
					return `closed ${event.reason}${event.resetCode === undefined ? "" : ` ${event.resetCode}`}`;
				case "fetch-data":
					return `data ${toHex(event.data)}`;
				case "fetch-stream-ended":
					return `stream ${event.how}${event.resetCode === undefined ? "" : ` ${event.resetCode}`}`;
				default:
					return event.type;
			}
		}
		case "stop-sending":
			return `stop peer-uni ${output.peerStream} code ${output.code}`;
		case "reset":
			return `reset ${output.stream} code ${output.code}`;
		case "finish":
			return `finish ${output.stream}`;
		case "close-session":
			return `close ${output.code}: ${output.reason}`;
		default:
			return output.type;
	}
}

/**
 * One thing that happens to the FETCH of `fetching()`. The fetch stream is
 * the peer's unidirectional stream 7, a second one 8.
 */
type Step =
	/** FETCH_OK with End Location {3, 0}. */
	| "answer"
	/** FETCH_OK with End Location {0, 5}, before the start {1, 0}. */
	| "answer that ends before the start"
	| "rejected"
	| "stream"
	| "second stream"
	| "data"
	| "stream fin"
	| "stream reset"
	| "cancel"
	| "request stream fin"
	| "request stream reset";

function run(session: Session, step: Step): SessionOutput[] {
	switch (step) {
		case "answer":
			return session.receiveRequestStreamData(1, fetchOk(at(3n, 0n)), false);
		case "answer that ends before the start":
			return session.receiveRequestStreamData(1, fetchOk(at(0n, 5n)), false);
		case "rejected":
			return session.receiveRequestStreamData(1, REQUEST_ERROR, false);
		case "stream":
			return session.receivePeerUniStreamData(7, hex("05 00"), false);
		case "second stream":
			return session.receivePeerUniStreamData(8, hex("05 00"), false);
		case "data":
			return session.receivePeerUniStreamData(7, hex("aa bb"), false);
		case "stream fin":
			return session.receivePeerUniStreamData(7, new Uint8Array(0), true);
		case "stream reset":
			return session.peerUniStreamReset(7, 9);
		case "cancel":
			return session.cancelRequest(0n);
		case "request stream fin":
			return session.receiveRequestStreamData(1, new Uint8Array(0), true);
		case "request stream reset":
			return session.requestStreamReset(1, 4);
	}
}

describe("a FETCH this endpoint sends", () => {
	test("is sent with what it was asked with, on a stream of its own", () => {
		const session = started();
		const range = standalone(at(1n, 2n), at(3n, 0n));
		const { value, outputs } = session.fetch({
			range,
			parameters: { subscriberPriority: 7 },
		});
		expect(value).toBe(0n);
		const [open] = outputs;
		if (open?.type !== "open-bidi-stream") {
			throw new Error("the FETCH did not open a stream");
		}
		expect(open.stream).toBe(1);
		expect(decodeControlMessage(open.data)).toMatchObject({
			status: "ok",
			value: {
				type: "FETCH",
				requestId: 0n,
				range,
				parameters: { subscriberPriority: 7 },
			},
		});
	});

	// §10.12.1: an End Location names the last Object plus 1, and with Object
	// 0 the whole Group. The rows are the ones of the Rust test.
	test.each([
		{ start: at(1n, 0n), end: at(1n, 0n), before: false },
		{ start: at(1n, 5n), end: at(1n, 0n), before: false },
		{ start: at(1n, 5n), end: at(1n, 5n), before: false },
		{ start: at(1n, 5n), end: at(1n, 4n), before: true },
		{ start: at(2n, 0n), end: at(1n, 0n), before: true },
		{ start: at(2n, 0n), end: at(1n, 9n), before: true },
		{ start: at(0n, 0n), end: at(0n, 1n), before: false },
		{ start: at(1n, 0n), end: at(2n, 0n), before: false },
	])(
		"a range from $start to $end ends before it starts: $before",
		({ start, end, before }) => {
			expect(endsBefore(start, end)).toBe(before);
		},
	);

	// The session has a SUBSCRIBE (Request ID 0) and a PUBLISH (Request ID 2).
	test.each([
		{
			name: "a range that ends before it starts (§10.12.3)",
			request: { range: standalone(at(2n, 0n), at(1n, 0n)) },
		},
		{
			name: "FORWARD, which is no parameter of a FETCH (§10.2.1)",
			request: {
				range: standalone(at(0n, 0n), at(1n, 0n)),
				parameters: { forward: true },
			},
		},
		{
			name: "a Joining FETCH that names a PUBLISH of this endpoint",
			request: {
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 2n,
					joiningStart: 0n,
				},
			},
		},
		{
			name: "a Joining FETCH that names an ID this endpoint has not used",
			request: {
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 40n,
					joiningStart: 0n,
				},
			},
		},
		{
			name: "an Absolute Joining FETCH that names an ID of the peer",
			request: {
				range: {
					fetchType: "AbsoluteJoining",
					joiningRequestId: 41n,
					joiningStart: 0n,
				},
			},
		},
	] satisfies {
		name: string;
		request: Parameters<Session["fetch"]>[0];
	}[])("is not sent with $name", ({ request }) => {
		const session = started();
		session.subscribe(TRACK);
		session.publish({ ...TRACK, trackAlias: 0n });
		expect(() => session.fetch(request)).toThrow(RangeError);
		// No Request ID was used up: the next request gets 4.
		expect(session.subscribe(TRACK).value).toBe(4n);
	});

	test("may join a SUBSCRIBE that has no answer yet, and not one that was cancelled (§10.12.2)", () => {
		const session = started();
		const subscribe = session.subscribe(TRACK).value;
		const joining: FetchRange = {
			fetchType: "RelativeJoining",
			joiningRequestId: subscribe,
			joiningStart: 0n,
		};
		expect(session.fetch({ range: joining }).value).toBe(2n);
		session.cancelRequest(subscribe);
		expect(() => session.fetch({ range: joining })).toThrow(RangeError);
	});

	// Each row: what happens, in order, and what every step has to lead to.
	// The rows up to "an answer after the FETCH was served" are the ones of
	// the Rust test `fetch_transitions`, with what is sent and reported here.
	test.each<{ name: string; steps: [Step, string[]][] }>([
		{
			name: "the answer, then the stream",
			steps: [
				["answer", ["fetch-ok"]],
				["stream", []],
				["data", ["data aa bb"]],
				["stream fin", ["stream fin", "closed fetch-served", "finish 1"]],
			],
		},
		{
			name: "the stream, then the answer",
			steps: [
				["stream", []],
				["data", ["data aa bb"]],
				["answer", ["fetch-ok"]],
				["stream fin", ["stream fin", "closed fetch-served", "finish 1"]],
			],
		},
		{
			name: "the stream ends before the answer comes",
			steps: [
				["stream", []],
				["stream fin", ["stream fin"]],
				["answer", ["fetch-ok", "closed fetch-served", "finish 1"]],
			],
		},
		{
			name: "no answer: the request stays until it is cancelled",
			steps: [
				["stream", []],
				["stream fin", ["stream fin"]],
				["cancel", ["reset 1 code 1"]],
				// The request is over: a stream for it is one to stop.
				["second stream", ["stop peer-uni 8 code 1"]],
			],
		},
		{
			name: "rejected, and the stream that was on its way",
			steps: [
				["rejected", ["request-error", "finish 1"]],
				["stream", ["stop peer-uni 7 code 1"]],
				// What the peer sent before it saw the stop is dropped.
				["data", []],
				["stream fin", []],
			],
		},
		{
			name: "rejected after the stream came",
			steps: [
				["stream", []],
				["rejected", ["stop peer-uni 7 code 1", "request-error", "finish 1"]],
				["data", []],
				["second stream", ["stop peer-uni 8 code 1"]],
			],
		},
		{
			name: "cancelled, and the stream that was on its way",
			steps: [
				["answer", ["fetch-ok"]],
				["cancel", ["reset 1 code 1"]],
				["stream", ["stop peer-uni 7 code 1"]],
			],
		},
		{
			name: "cancelled while the stream is open",
			steps: [
				["stream", []],
				["cancel", ["stop peer-uni 7 code 1", "reset 1 code 1"]],
				["data", []],
				["stream reset", []],
			],
		},
		{
			name: "two streams for one FETCH",
			steps: [
				["stream", []],
				["second stream", ["close 3: a second fetch stream for one FETCH"]],
			],
		},
		{
			name: "a second stream after the first ended, while the FETCH is open",
			steps: [
				["stream", []],
				["stream fin", ["stream fin"]],
				["second stream", ["close 3: a second fetch stream for one FETCH"]],
			],
		},
		{
			name: "a stream after the FETCH was served is not found out",
			steps: [
				["answer", ["fetch-ok"]],
				["stream", []],
				["stream fin", ["stream fin", "closed fetch-served", "finish 1"]],
				["second stream", ["stop peer-uni 8 code 1"]],
			],
		},
		{
			name: "an answer that ends before the start",
			steps: [
				[
					"answer that ends before the start",
					["close 3: FETCH_OK ends before the FETCH starts"],
				],
			],
		},
		{
			name: "two answers",
			steps: [
				["answer", ["fetch-ok"]],
				[
					"answer",
					["close 3: unexpected FETCH_OK on the FETCH request stream"],
				],
			],
		},
		{
			name: "an answer after the FETCH was served",
			steps: [
				["answer", ["fetch-ok"]],
				["stream", []],
				["stream fin", ["stream fin", "closed fetch-served", "finish 1"]],
				[
					"answer",
					["close 3: FETCH_OK on a FETCH request stream that had ended"],
				],
			],
		},
		{
			name: "the stream is reset by the peer",
			steps: [
				["answer", ["fetch-ok"]],
				["stream", []],
				["data", ["data aa bb"]],
				["stream reset", ["stream reset 9", "closed fetch-served", "finish 1"]],
			],
		},
		{
			name: "the publisher finishes the request stream after its answer",
			steps: [
				["answer", ["fetch-ok"]],
				["request stream fin", []],
				["stream", []],
				["stream fin", ["stream fin", "closed fetch-served", "finish 1"]],
			],
		},
		{
			name: "the publisher finishes the request stream without an answer",
			steps: [
				["stream", []],
				[
					"request stream fin",
					["stop peer-uni 7 code 1", "closed peer-fin", "finish 1"],
				],
			],
		},
		{
			name: "the publisher resets the request stream (§5.2)",
			steps: [
				["answer", ["fetch-ok"]],
				["stream", []],
				[
					"request stream reset",
					["stop peer-uni 7 code 1", "closed peer-reset 4", "reset 1 code 1"],
				],
			],
		},
	])("$name", ({ steps }) => {
		const session = fetching();
		for (const [index, [step, expected]] of steps.entries()) {
			expect([index, step, briefly(run(session, step))]).toEqual([
				index,
				step,
				expected,
			]);
		}
	});

	// (the range; the End Location of the FETCH_OK; whether it is taken).
	// The session has a SUBSCRIBE with Request ID 0; the FETCH gets 2, on
	// stream 2.
	test.each<{ name: string; range: FetchRange; end: Location; taken: boolean }>(
		[
			{
				name: "Standalone from {4, 2}, answered with {4, 2}",
				range: standalone(at(4n, 2n), at(9n, 0n)),
				end: at(4n, 2n),
				taken: true,
			},
			{
				name: "Standalone from {4, 2}, answered with {4, 0}, the whole Group",
				range: standalone(at(4n, 2n), at(9n, 0n)),
				end: at(4n, 0n),
				taken: true,
			},
			{
				name: "Standalone from {4, 2}, answered with {4, 1}",
				range: standalone(at(4n, 2n), at(9n, 0n)),
				end: at(4n, 1n),
				taken: false,
			},
			{
				// §10.12.2.1: {Joining Start, 0} is the start.
				name: "Absolute Joining from Group 4, answered with {3, 7}",
				range: {
					fetchType: "AbsoluteJoining",
					joiningRequestId: 0n,
					joiningStart: 4n,
				},
				end: at(3n, 7n),
				taken: false,
			},
			{
				// The start depends on what the publisher holds.
				name: "Relative Joining, answered with {0, 1}",
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 0n,
					joiningStart: 4n,
				},
				end: at(0n, 1n),
				taken: true,
			},
		],
	)(
		"the End Location of a FETCH_OK is compared with the start this endpoint knows: $name",
		({ range, end, taken }) => {
			const session = started();
			session.subscribe(TRACK);
			session.fetch({ range });
			expect(
				briefly(session.receiveRequestStreamData(2, fetchOk(end), false)),
			).toEqual(
				taken
					? ["fetch-ok"]
					: ["close 3: FETCH_OK ends before the FETCH starts"],
			);
		},
	);

	test("a FETCH_OK with a Mandatory Track Property ends the FETCH and leaves the session open (§2.5.1)", () => {
		const session = fetching();
		expect(briefly(run(session, "stream"))).toEqual([]);
		// 0x4000 is in the range of Mandatory Track Properties.
		expect(
			session
				.receiveRequestStreamData(
					1,
					fetchOk(at(3n, 0n), [{ type: 0x4000n, value: 1n }]),
					false,
				)
				.map(brief),
		).toEqual([
			"stop peer-uni 7 code 1",
			"closed unsupported-track-property",
			"reset 1 code 1",
		]);
		expect(session.closed).toBe(false);
	});

	// The session has an open SUBSCRIBE (Request ID 0), a PUBLISH that was
	// cancelled (2) and a FETCH that was cancelled (4). The next ID is 6.
	test.each<{ name: string; header: string; expected: string[] }>([
		{
			name: "an open request that is no FETCH",
			header: "05 00",
			expected: ["close 3: fetch stream for no FETCH of this endpoint"],
		},
		{
			name: "an ID this endpoint has not used yet",
			header: "05 06",
			expected: ["close 3: fetch stream for no FETCH of this endpoint"],
		},
		{
			name: "an ID of the peer's parity",
			header: "05 01",
			expected: ["close 3: fetch stream for no FETCH of this endpoint"],
		},
		{
			name: "a PUBLISH that is over: the stream is one to stop",
			header: "05 02",
			expected: ["stop peer-uni 7 code 1"],
		},
		{
			name: "a FETCH that is over",
			header: "05 04",
			expected: ["stop peer-uni 7 code 1"],
		},
	])("a fetch stream for $name", ({ header, expected }) => {
		const session = started();
		session.subscribe(TRACK);
		session.cancelRequest(session.publish({ ...TRACK, trackAlias: 0n }).value);
		session.cancelRequest(
			session.fetch({ range: standalone(at(0n, 0n), at(1n, 0n)) }).value,
		);
		expect(
			briefly(session.receivePeerUniStreamData(7, hex(header), false)),
		).toEqual(expected);
	});

	// A SUBSCRIBE (Request ID 0) that is Established with Track Alias 9 and
	// got PUBLISH_DONE with Stream Count 1: it waits for that stream (§10.11)
	// and is over for everything else, as in the Rust session.
	test.each<{
		name: string;
		expected: string[];
		act: (session: Session) => string[];
	}>([
		{
			name: "a fetch stream with its Request ID is stopped, and the session stays",
			expected: ["stop peer-uni 7 code 1"],
			act: (session) =>
				briefly(session.receivePeerUniStreamData(7, hex("05 00"), false)),
		},
		{
			name: "a Joining FETCH for it is not sent",
			expected: ["RangeError"],
			act: (session) => {
				try {
					session.fetch({
						range: {
							fetchType: "RelativeJoining",
							joiningRequestId: 0n,
							joiningStart: 0n,
						},
					});
					return ["sent"];
				} catch (error) {
					return [error instanceof RangeError ? "RangeError" : String(error)];
				}
			},
		},
	])(
		"a SUBSCRIBE that got PUBLISH_DONE and waits for its streams: $name",
		({ expected, act }) => {
			const session = started();
			session.subscribe(TRACK);
			session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
			// Before the PUBLISH_DONE the SUBSCRIBE is open: a FETCH may join it.
			expect(
				session.fetch({
					range: {
						fetchType: "AbsoluteJoining",
						joiningRequestId: 0n,
						joiningStart: 0n,
					},
				}).value,
			).toBe(2n);
			expect(
				session
					.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false)
					.map(brief),
			).toEqual(["publish-done"]);
			expect(act(session)).toEqual(expected);
			expect(session.closed).toBe(false);
		},
	);

	test("a stream for a FETCH that ended long ago is stopped like any other", () => {
		const session = started();
		for (let count = 0; count < 1000; count++) {
			session.cancelRequest(
				session.fetch({ range: standalone(at(0n, 0n), at(1n, 0n)) }).value,
			);
		}
		// Request ID 0, and 1998 in two bytes (§1.4.1).
		expect(
			[
				session.receivePeerUniStreamData(7, hex("05 00"), false),
				session.receivePeerUniStreamData(8, hex("05 87 ce"), false),
			].map(briefly),
		).toEqual([["stop peer-uni 7 code 1"], ["stop peer-uni 8 code 1"]]);
	});

	// §11.4.4: Stream Type 0x05 and the Request ID, here 0 in the two bytes
	// 80 00 (§1.4.1). Each entry of `chunks` arrives by itself; the last one with the
	// end of the stream where `fin` is set.
	test.each<{
		name: string;
		chunks: string[];
		fin: boolean;
		expected: string[][];
	}>([
		{
			name: "the header and bytes in one read",
			chunks: ["05 80 00 aa bb"],
			fin: false,
			expected: [["data aa bb"]],
		},
		{
			name: "the header cut after the type and inside the Request ID",
			chunks: ["05", "80", "00 aa", "bb"],
			fin: false,
			expected: [[], [], ["data aa"], ["data bb"]],
		},
		{
			name: "the type cut: 0x05 in two bytes",
			chunks: ["80", "05 00 aa"],
			fin: false,
			expected: [[], ["data aa"]],
		},
		{
			name: "a stream with its header and nothing else (§10.12.3)",
			chunks: ["05 00"],
			fin: true,
			expected: [["stream fin"]],
		},
		{
			name: "bytes and the end in one read",
			chunks: ["05 00 aa"],
			fin: true,
			expected: [["data aa", "stream fin"]],
		},
		{
			name: "a stream that ends after its type",
			chunks: ["05"],
			fin: true,
			expected: [["close 3: fetch stream ended in its FETCH_HEADER"]],
		},
		{
			name: "a stream that ends inside its Request ID",
			chunks: ["05", "80"],
			fin: true,
			expected: [[], ["close 3: fetch stream ended in its FETCH_HEADER"]],
		},
	])("reads the FETCH_HEADER: $name", ({ chunks, fin, expected }) => {
		const session = fetching();
		expect(
			chunks.map((chunk, index) =>
				briefly(
					session.receivePeerUniStreamData(
						7,
						hex(chunk),
						fin && index === chunks.length - 1,
					),
				),
			),
		).toEqual(expected);
	});

	test("a fetch stream that is reset inside its FETCH_HEADER carried nothing (§11.4.1)", () => {
		const session = fetching();
		expect(session.receivePeerUniStreamData(7, hex("05"), false)).toEqual([]);
		expect(session.peerUniStreamReset(7, 1)).toEqual([]);
		// The FETCH has seen no stream: one that comes is its stream.
		expect(briefly(run(session, "second stream"))).toEqual([]);
		expect(session.closed).toBe(false);
	});

	test.each<{ name: string; message: ControlMessage }>([
		{
			name: "REQUEST_OK",
			message: { type: "REQUEST_OK", parameters: {}, trackProperties: [] },
		},
		{
			name: "SUBSCRIBE_OK",
			message: {
				type: "SUBSCRIBE_OK",
				trackAlias: 0n,
				parameters: {},
				trackProperties: [],
			},
		},
		{
			name: "PUBLISH_DONE",
			message: {
				type: "PUBLISH_DONE",
				statusCode: 0n,
				streamCount: 0n,
				errorReason: "",
			},
		},
	])("$name in answer to a FETCH closes the session (§5.2)", ({ message }) => {
		expect(
			fetching()
				.receiveRequestStreamData(1, encodeControlMessage(message), false)
				.at(-1),
		).toMatchObject({ type: "close-session", code: 3 });
	});
});
