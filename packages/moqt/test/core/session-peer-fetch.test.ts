import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { InactiveError } from "../../src/core/error.ts";
import { createFetchStreamDecoder } from "../../src/core/fetch.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixture, hex, toHex, utf8 } from "../helpers.ts";

const FIXTURES = join(import.meta.dir, "..", "..", "..", "..", "fixtures");

/** The rows of a table of `fixtures/peer-fetch/`, each with as many fields as told. */
function table(name: string, fields: number): string[][] {
	return readFileSync(join(FIXTURES, "peer-fetch", name), "utf8")
		.split("\n")
		.filter((line) => line.length > 0)
		.map((line) => {
			const row = line.split("\t");
			if (row.length !== fields) {
				throw new Error(
					`a row of ${name} does not have ${fields} fields: ${line}`,
				);
			}
			return row;
		});
}

/** The track of the captures. */
const TRACK = {
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
};

/** The captured SUBSCRIBE with Request ID 1, one of the server. */
function subscribeBytes(forward = "01"): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = "01";
	// The value of its FORWARD, the parameter of Type 0x10 at index 25.
	pairs[26] = forward;
	return hex(pairs.join(" "));
}

/**
 * The captured Standalone FETCH with a Request ID of the server, a range,
 * and its one parameter (SUBSCRIBER_PRIORITY 200) or the ones given.
 */
function fetchBytes(
	requestId: number,
	start: [number, number],
	end: [number, number],
	parameters = "01 20 c8",
): Uint8Array {
	const pairs = fixture("fetch-standalone-moqtail-rs").split(" ");
	const byte = (value: number): string => value.toString(16).padStart(2, "0");
	pairs[3] = byte(requestId);
	pairs.splice(
		25,
		7,
		...[...start, ...end].map(byte),
		...parameters.split(" "),
	);
	pairs[2] = byte(pairs.length - 3);
	return hex(pairs.join(" "));
}

/** REQUEST_UPDATE with a Request ID and parameters, written by hand: no capture of one exists. */
function updateBytes(requestId: number, parameters: string): Uint8Array {
	// A varint of one byte up to 127, of two with the prefix 0x80 beyond
	// (§1.4.1).
	const id =
		requestId < 128
			? requestId.toString(16).padStart(2, "0")
			: `80 ${requestId.toString(16).padStart(2, "0")}`;
	const body = `${id} ${parameters}`;
	return hex(
		`02 00 ${body.split(" ").length.toString(16).padStart(2, "0")} ${body}`,
	);
}

function started(limits: { maxPeerFetches?: number } = {}): Session {
	const session = createSession({ limits });
	session.start();
	return session;
}

/** A session whose peer asked for {3, 4} to {5, 1} with Request ID 1, on its stream 0; the session named that stream 1. */
function asked(): Session {
	const session = started();
	session.receivePeerBidiStreamData(0, fetchBytes(1, [3, 4], [5, 1]), false);
	return session;
}

/** That, accepted with what was asked for: the fetch stream is stream 2. */
function serving(): Session {
	const session = asked();
	session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } });
	return session;
}

/** An output in a line. */
function brief(output: SessionOutput): string {
	const message = (data: Uint8Array): string => {
		const decoded = decodeControlMessage(data);
		if (decoded.status !== "ok") return `bytes ${toHex(data)}`;
		const { value } = decoded;
		switch (value.type) {
			case "REQUEST_ERROR":
				return `REQUEST_ERROR ${value.errorCode}`;
			case "FETCH_OK":
				return `FETCH_OK end {${value.endLocation.group}, ${value.endLocation.object}}`;
			default:
				return value.type;
		}
	};
	switch (output.type) {
		case "adopt-peer-stream":
			return `adopt peer ${output.peerStream} as ${output.stream}`;
		case "respond":
			return `respond peer ${output.peerStream}: ${message(output.data)}`;
		case "send":
			return output.stream === 2
				? `fetch stream: ${toHex(output.data)}`
				: `send ${output.stream}: ${message(output.data)}`;
		case "finish":
			return `finish ${output.stream}`;
		case "reset":
			return `reset ${output.stream} code ${output.code}`;
		case "stop-peer-direction":
			return `stop peer direction ${output.stream} code ${output.code}`;
		case "open-uni-stream":
			return `open uni ${output.stream}: ${toHex(output.data)}`;
		case "close-session":
			return `close ${output.code}`;
		case "event": {
			const { event } = output;
			switch (event.type) {
				case "peer-fetch":
					return `event peer-fetch ${event.requestId}`;
				case "request-closed":
					return `event request-closed ${event.requestId} ${event.reason}`;
				case "peer-request-rejected":
					return `event peer-request-rejected ${event.requestId} code ${event.errorCode}`;
				case "peer-direction-ended":
					return `event peer-ended ${event.requestId} ${event.how}`;
				default:
					return `event ${event.type}`;
			}
		}
		default:
			return output.type;
	}
}

const briefly = (outputs: SessionOutput[]): string[] => outputs.map(brief);

const object = (groupId: bigint, objectId: bigint, text: string) => ({
	groupId,
	subgroupId: 0n,
	objectId,
	publisherPriority: 0,
	properties: [],
	payload: utf8(text),
});

describe("a Standalone FETCH of the peer", () => {
	test("is handed on with its range and its parameters, and its stream is taken over", () => {
		const session = started();
		const outputs = session.receivePeerBidiStreamData(
			4,
			fetchBytes(1, [3, 4], [5, 1], "02 0a 00 18 02"),
			false,
		);
		expect(briefly(outputs)).toEqual([
			"peer-stream-identified",
			"adopt peer 4 as 1",
			"event peer-fetch 1",
		]);
		expect(outputs.at(-1)).toEqual({
			type: "event",
			event: {
				type: "peer-fetch",
				requestId: 1n,
				...TRACK,
				startLocation: { group: 3n, object: 4n },
				endLocation: { group: 5n, object: 1n },
				parameters: { fillTimeout: 0n, groupOrder: "Descending" },
			},
		});
	});

	test("accepted, gets FETCH_OK and its fetch stream at once, and the Objects as they are written", () => {
		const session = asked();
		expect(
			briefly(
				session.acceptFetch(1n, {
					endLocation: { group: 5n, object: 1n },
					endOfTrack: true,
				}),
			),
		).toEqual(["send 1: FETCH_OK end {5, 1}", "open uni 2: 05 01"]);
		const written = [
			...session.writeFetchObject(1n, object(3n, 4n, "a")),
			...session.writeFetchObject(1n, object(3n, 5n, "b")),
			...session.writeFetchRangeEnd(1n, "NonExistent", {
				group: 4n,
				object: 9n,
			}),
			...session.writeFetchObject(1n, object(5n, 0n, "c")),
		];
		// What was written is what a reader of the stream gets.
		const decoder = createFetchStreamDecoder(
			{ maxObjectPayloadLength: 100, maxObjectPropertiesLength: 100 },
			"Ascending",
		);
		const items = [
			hex("05 01"),
			...written.map((output) =>
				output.type === "send" ? output.data : new Uint8Array(0),
			),
		].flatMap((bytes) => decoder.push(bytes).items);
		expect(
			items.map((item) =>
				item.type === "object"
					? `{${item.object.groupId}, ${item.object.objectId}} ${new TextDecoder().decode(item.object.payload)}`
					: item.type === "end-of-range"
						? `${item.kind} to {${item.location.group}, ${item.location.object}}`
						: `header ${item.header.requestId}`,
			),
		).toEqual([
			"header 1",
			"{3, 4} a",
			"{3, 5} b",
			"NonExistent to {4, 9}",
			"{5, 0} c",
		]);
		// The fetch stream alone: this endpoint's direction of the request
		// stream waits for the subscriber's.
		expect(briefly(session.finishFetch(1n))).toEqual(["finish 2"]);
	});

	test("an Object that goes back is refused, and the stream goes on as it was", () => {
		const session = serving();
		session.writeFetchObject(1n, object(3n, 5n, "a"));
		expect(() => session.writeFetchObject(1n, object(3n, 5n, "again"))).toThrow(
			RangeError,
		);
		expect(() => session.writeFetchObject(1n, object(2n, 0n, "back"))).toThrow(
			RangeError,
		);
		expect(() => session.writeFetchObject(1n, object(3n, 4n, "back"))).toThrow(
			RangeError,
		);
		expect(() =>
			session.writeFetchRangeEnd(1n, "NonExistent", { group: 3n, object: 1n }),
		).toThrow(RangeError);
		// The next one in order: Object ID one more, so no field but the
		// payload, as if nothing had been refused in between.
		expect(briefly(session.writeFetchObject(1n, object(3n, 6n, "b")))).toEqual([
			"fetch stream: 00 01 62",
		]);
	});

	test("an Object outside the range that was asked for is written as it is", () => {
		const session = serving();
		expect(
			briefly(session.writeFetchObject(1n, object(9n, 0n, "z"))).length,
		).toBe(1);
	});

	test("that asks for the Groups in descending order has them written so", () => {
		const session = started();
		session.receivePeerBidiStreamData(
			0,
			fetchBytes(1, [3, 0], [5, 0], "01 22 02"),
			false,
		);
		session.acceptFetch(1n, { endLocation: { group: 5n, object: 0n } });
		session.writeFetchObject(1n, object(5n, 0n, "a"));
		expect(() => session.writeFetchObject(1n, object(6n, 0n, "up"))).toThrow(
			RangeError,
		);
		// Group 4 after Group 5: flags 0x0c (a Group ID Delta and an Object
		// ID follow; Subgroup ID 0, the priority of the Object before), the
		// Group ID Delta 0, Object ID 0. A refusal that had moved the encoder
		// to Group 6 would give a Group ID Delta of 1.
		expect(briefly(session.writeFetchObject(1n, object(4n, 0n, "b")))).toEqual([
			"fetch stream: 0c 00 00 01 62",
		]);
	});

	// Each row: the session, what happens, and what that leads to.
	const rows: {
		name: string;
		session: () => Session;
		act: (session: Session) => SessionOutput[];
		outputs: string[];
	}[] = [
		{
			name: "rejected: REQUEST_ERROR and a FIN",
			session: asked,
			act: (session) => session.rejectPeerRequest(1n, 0x10n, ""),
			outputs: [
				"send 1: REQUEST_ERROR 16",
				"event peer-request-rejected 1 code 16",
				"finish 1",
			],
		},
		{
			name: "the subscriber resets the request stream before the answer",
			session: asked,
			act: (session) => session.peerBidiStreamReset(0, 1),
			outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
		},
		{
			name: "the subscriber resets the request stream while it is served: both streams are reset (§5.2)",
			session: serving,
			act: (session) => session.peerBidiStreamReset(0, 1),
			outputs: [
				"reset 2 code 1",
				"event request-closed 1 peer-reset",
				"reset 1 code 1",
			],
		},
		{
			name: "the subscriber stops the request stream while it is served: both are reset (§5.2)",
			session: serving,
			act: (session) => session.localStreamStopped(1, 1),
			outputs: [
				"reset 2 code 1",
				"event request-closed 1 peer-reset",
				"reset 1 code 1",
			],
		},
		{
			name: "the subscriber stops the fetch stream alone: the FETCH is given up, and both are reset",
			session: serving,
			act: (session) => session.localStreamStopped(2, 1),
			outputs: [
				"reset 2 code 1",
				"event request-closed 1 peer-reset",
				"reset 1 code 1",
			],
		},
		{
			name: "the subscriber finishes its direction while it is served: nothing",
			session: serving,
			act: (session) =>
				session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			outputs: [],
		},
		{
			name: "this endpoint gives it up: both streams are reset with its code",
			session: serving,
			act: (session) => session.resetFetch(1n, 0),
			outputs: ["reset 2 code 0", "reset 1 code 0"],
		},
		{
			name: "anything but a REQUEST_UPDATE on its stream closes the session",
			session: serving,
			act: (session) =>
				session.receivePeerBidiStreamData(
					0,
					hex(fixture("publish-done-moqtail-relay")),
					false,
				),
			outputs: ["close 3"],
		},
	];
	test.each(rows)("$name", ({ session, act, outputs }) => {
		expect(briefly(act(session()))).toEqual(outputs);
	});

	test.each([
		{ name: "before the answer", session: asked },
		{ name: "while it is served", session: serving },
	])("a REQUEST_UPDATE $name uses up its Request ID (§10.1)", ({ session }) => {
		const prepared = session();
		prepared.receivePeerBidiStreamData(0, updateBytes(3, "00"), false);
		// Another request with Request ID 3: the start of a SUBSCRIBE_TRACKS.
		expect(
			briefly(
				prepared.receivePeerBidiStreamData(9, hex("51 00 02 03 00"), false),
			).at(-1),
		).toBe("close 4");
	});

	// As many as the relay lets wait for a publisher's answer
	// (`MAX_HELD_FETCH_UPDATES`, crates/relay-core/src/fetch.rs).
	test("64 REQUEST_UPDATEs may wait for the answer, and each gets its REQUEST_OK after the FETCH_OK", () => {
		const session = asked();
		for (let index = 0; index < 64; index++) {
			expect(
				session.receivePeerBidiStreamData(
					0,
					updateBytes(3 + index * 2, "00"),
					false,
				),
			).toEqual([]);
		}
		const sent = briefly(
			session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
		);
		expect(sent.length).toBe(66);
		expect(sent.filter((each) => each === "send 1: REQUEST_OK").length).toBe(
			64,
		);
		expect([sent.at(0), sent.at(-1)]).toEqual([
			"send 1: FETCH_OK end {5, 1}",
			"open uni 2: 05 01",
		]);
	});

	test("one REQUEST_UPDATE more refuses the FETCH with EXCESSIVE_LOAD, and no update gets an answer", () => {
		const session = asked();
		for (let index = 0; index < 64; index++) {
			session.receivePeerBidiStreamData(
				0,
				updateBytes(3 + index * 2, "00"),
				false,
			);
		}
		expect(
			briefly(
				session.receivePeerBidiStreamData(0, updateBytes(131, "00"), false),
			),
		).toEqual([
			"send 1: REQUEST_ERROR 9",
			"event request-closed 1 update-failed",
			"finish 1",
		]);
		expect(() =>
			session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
		).toThrow(InactiveError);
		// A later one is ignored, and the session stays open.
		expect(
			session.receivePeerBidiStreamData(0, updateBytes(133, "00"), false),
		).toEqual([]);
		expect(session.closed).toBe(false);
	});

	test("served, waits for the subscriber to end its direction, and stops it when the wait is over", () => {
		const session = serving();
		session.finishFetch(1n);
		session.finishServedFetchSend(1n);
		expect(briefly(session.expirePeerEnd(1n))).toEqual([
			"stop peer direction 1 code 1",
		]);
	});

	test("served, hears how the subscriber ended its direction", () => {
		const session = serving();
		session.finishFetch(1n);
		expect(
			briefly(session.receivePeerBidiStreamData(0, new Uint8Array(0), true)),
		).toEqual(["finish 1", "event peer-ended 1 fin"]);
	});

	// After the fetch stream is finished, this endpoint's direction of the
	// request stream (stream 1) is finished when the subscriber has ended
	// its own (peer stream 0), or when the caller's wait for that is over.
	test.each<{
		name: string;
		steps: ((session: Session) => SessionOutput[])[];
		outputs: string[][];
	}>([
		{
			name: "the subscriber finishes its direction: this endpoint's is finished then",
			steps: [
				(session) => session.finishFetch(1n),
				(session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
				(session) => session.finishServedFetchSend(1n),
			],
			outputs: [["finish 2"], ["finish 1", "event peer-ended 1 fin"], []],
		},
		{
			name: "the subscriber had finished its direction before: both streams are finished at once",
			steps: [
				(session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
				(session) => session.finishFetch(1n),
				(session) => session.finishServedFetchSend(1n),
			],
			outputs: [[], ["finish 2", "finish 1", "event peer-ended 1 fin"], []],
		},
		{
			name: "the subscriber resets its direction: this endpoint's is finished then",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.peerBidiStreamReset(0, 1),
				(session) => session.finishServedFetchSend(1n),
			],
			outputs: [["finish 2"], ["finish 1", "event peer-ended 1 reset"], []],
		},
		{
			name: "the wait is over: this endpoint finishes its direction, once, and the subscriber's is stopped when the next wait is over",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.finishServedFetchSend(1n),
				(session) => session.finishServedFetchSend(1n),
				(session) => session.expirePeerEnd(1n),
			],
			outputs: [
				["finish 2"],
				["finish 1"],
				[],
				["stop peer direction 1 code 1"],
			],
		},
		{
			name: "the second wait is over without the first having been ended: this endpoint's direction is finished with the stop, once",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.expirePeerEnd(1n),
				(session) => session.finishServedFetchSend(1n),
			],
			outputs: [["finish 2"], ["finish 1", "stop peer direction 1 code 1"], []],
		},
		{
			name: "the subscriber finishes after this endpoint did: nothing is finished twice",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.finishServedFetchSend(1n),
				(session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			],
			outputs: [["finish 2"], ["finish 1"], ["event peer-ended 1 fin"]],
		},
		{
			name: "the subscriber stops this endpoint's direction during the wait: it is reset, the subscriber's is stopped, and the stream is forgotten",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.localStreamStopped(1, 7),
				(session) => session.finishServedFetchSend(1n),
				(session) => session.expirePeerEnd(1n),
				(session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			],
			outputs: [
				["finish 2"],
				[
					"reset 1 code 1",
					"stop peer direction 1 code 1",
					"event peer-ended 1 reset",
				],
				[],
				[],
				[],
			],
		},
		{
			name: "the subscriber stops this endpoint's direction after it was finished: nothing more is done for it, and its own direction is waited for as before",
			steps: [
				(session) => session.finishFetch(1n),
				(session) => session.finishServedFetchSend(1n),
				(session) => session.localStreamStopped(1, 7),
				(session) => session.expirePeerEnd(1n),
			],
			outputs: [
				["finish 2"],
				["finish 1"],
				[],
				["stop peer direction 1 code 1"],
			],
		},
		{
			name: "a FETCH that is still being served is not finished by the wait",
			steps: [(session) => session.finishServedFetchSend(1n)],
			outputs: [[]],
		},
	])("served, $name", ({ steps, outputs }) => {
		const session = serving();
		expect(steps.map((step) => briefly(step(session)))).toEqual(outputs);
	});

	test.each([
		{
			name: "writing before it is accepted",
			session: asked,
			act: (session: Session) =>
				session.writeFetchObject(1n, object(3n, 4n, "a")),
		},
		{
			name: "writing after it was finished",
			session: () => {
				const session = serving();
				session.finishFetch(1n);
				return session;
			},
			act: (session: Session) =>
				session.writeFetchObject(1n, object(3n, 4n, "a")),
		},
		{
			name: "accepting twice",
			session: serving,
			act: (session: Session) =>
				session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
		},
		{
			name: "rejecting what was accepted",
			session: serving,
			act: (session: Session) => session.rejectPeerRequest(1n, 3n, ""),
		},
	])("$name is a mistake of the caller", ({ session, act }) => {
		const prepared = session();
		expect(() => act(prepared)).toThrow(InactiveError);
	});

	// The session keeps something of one stream for a FETCH that is open:
	// the request stream under two numbers. The fetch stream is the I/O
	// layer's alone.
	test.each([
		{ name: "it waits", steps: [], held: 2 },
		{
			name: "served, and the subscriber has not ended its direction",
			steps: [
				(session: Session) =>
					session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
				(session: Session) => session.finishFetch(1n),
			],
			held: 2,
		},
		{
			name: "served, then the subscriber finishes",
			steps: [
				(session: Session) =>
					session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
				(session: Session) => session.finishFetch(1n),
				(session: Session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			],
			held: 0,
		},
		{
			name: "the subscriber finishes, then it is served",
			steps: [
				(session: Session) =>
					session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
				(session: Session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
				(session: Session) => session.finishFetch(1n),
			],
			held: 0,
		},
		{
			name: "the subscriber stops the fetch stream",
			steps: [
				(session: Session) =>
					session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
				(session: Session) => session.localStreamStopped(2, 1),
				// The reset ends the peer's direction for the I/O layer, which
				// reports it like an end.
				(session: Session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			],
			held: 0,
		},
		{
			name: "this endpoint gives it up",
			steps: [
				(session: Session) =>
					session.acceptFetch(1n, { endLocation: { group: 5n, object: 1n } }),
				(session: Session) => session.resetFetch(1n, 1),
				(session: Session) =>
					session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			],
			held: 0,
		},
	])("when $name, something is kept of $held streams", ({ steps, held }) => {
		const session = asked();
		for (const step of steps) step(session);
		expect(session.heldStreams()).toBe(held);
	});

	test("a FETCH counts among the requests of the peer that are open: EXCESSIVE_LOAD for a SUBSCRIBE beyond `maxPeerRequests`", () => {
		const session = createSession({ limits: { maxPeerRequests: 2 } });
		session.start();
		session.receivePeerBidiStreamData(0, fetchBytes(1, [0, 0], [1, 0]), false);
		session.receivePeerBidiStreamData(1, fetchBytes(3, [0, 0], [1, 0]), false);
		const pairs = fixture("subscribe-moqtail-rs").split(" ");
		pairs[3] = "05";
		expect(
			briefly(
				session.receivePeerBidiStreamData(2, hex(pairs.join(" ")), false),
			),
		).toEqual([
			"peer-stream-identified",
			"respond peer 2: REQUEST_ERROR 9",
			"event peer-request-rejected 5 code 9",
		]);
	});

	test("without a limit that was set, 32 FETCHes of the peer may be open, and one more gets EXCESSIVE_LOAD", () => {
		const session = started();
		const outcomes: string[] = [];
		for (let index = 0; index < 33; index++) {
			const outputs = briefly(
				session.receivePeerBidiStreamData(
					index,
					fetchBytes(1 + index * 2, [0, 0], [1, 0]),
					false,
				),
			);
			outcomes.push(
				outputs.find((each) => each.startsWith("respond")) ?? "handed",
			);
		}
		expect(outcomes.filter((each) => each === "handed").length).toBe(32);
		expect(outcomes.at(-1)).toBe("respond peer 32: REQUEST_ERROR 9");
	});

	test("EXCESSIVE_LOAD for one FETCH more than `maxPeerFetches`, waiting or served, and room again when one is over", () => {
		const session = started({ maxPeerFetches: 2 });
		session.receivePeerBidiStreamData(0, fetchBytes(1, [0, 0], [1, 0]), false);
		session.acceptFetch(1n, { endLocation: { group: 1n, object: 0n } });
		session.receivePeerBidiStreamData(1, fetchBytes(3, [0, 0], [1, 0]), false);
		expect(
			briefly(
				session.receivePeerBidiStreamData(
					2,
					fetchBytes(5, [0, 0], [1, 0]),
					false,
				),
			),
		).toEqual([
			"peer-stream-identified",
			"respond peer 2: REQUEST_ERROR 9",
			"event peer-request-rejected 5 code 9",
		]);
		session.finishFetch(1n);
		expect(
			briefly(
				session.receivePeerBidiStreamData(
					3,
					fetchBytes(7, [0, 0], [1, 0]),
					false,
				),
			),
		).toContain("event peer-fetch 7");
	});
});

// The tables both implementations read (fixtures/peer-fetch/README.md).
describe("fixtures/peer-fetch: what becomes of a FETCH of the peer", () => {
	const BEFORE: Record<string, (session: Session) => void> = {
		nothing: () => {},
		publish: (session) => {
			session.publish({ ...TRACK, trackAlias: 1n });
		},
		subscribe: (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(), false);
		},
		"subscribe-accepted": (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(), false);
			session.acceptSubscribe(1n, {});
		},
		"subscribe-done": (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(), false);
			session.acceptSubscribe(1n, {});
			session.publishDone(1n, 2n, "");
		},
		"subscribe-rejected": (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(), false);
			session.rejectPeerRequest(1n, 0x10n, "");
		},
		"subscribe-accepted-forward-0": (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes("00"), false);
			session.acceptSubscribe(1n, {});
		},
	};

	const outcomeOf = (outputs: SessionOutput[]): string => {
		for (const output of outputs) {
			if (output.type === "close-session") {
				return `closed 0x${output.code.toString(16)}`;
			}
			if (output.type === "respond") {
				const decoded = decodeControlMessage(output.data);
				if (decoded.status === "ok" && decoded.value.type === "REQUEST_ERROR") {
					return `refused 0x${decoded.value.errorCode.toString(16)}`;
				}
			}
			if (output.type === "event" && output.event.type === "peer-fetch") {
				return "handed";
			}
		}
		return "nothing";
	};

	const opened = table("opened.tsv", 5).map(
		([name, before, kind, sent, outcome]) => ({
			name,
			before: before ?? "",
			kind,
			sent: sent ?? "",
			// The one place the two implementations part, by design
			// (README.md): a Joining FETCH the Rust session hands to its
			// caller is refused here with NOT_SUPPORTED, as the Rust client
			// refuses it.
			outcome:
				kind === "joining" && outcome === "handed" ? "refused 0x3" : outcome,
		}),
	);
	test("opened.tsv has the 23 rows of its README", () => {
		expect(opened.length).toBe(23);
	});
	test.each(opened)(
		"$name ($kind), after $before: $outcome",
		({ before, sent, outcome }) => {
			const session = started();
			const prepare = BEFORE[before];
			if (prepare === undefined) throw new Error(`unknown state: ${before}`);
			prepare(session);
			const bytes = hex((sent.match(/../g) ?? []).join(" "));
			expect(
				outcomeOf(session.receivePeerBidiStreamData(1, bytes, false)),
			).toBe(outcome ?? "");
		},
	);

	/** What a step leads to, in the words of updates.tsv. */
	const sentBy = (act: () => SessionOutput[]): string => {
		let outputs: SessionOutput[];
		try {
			outputs = act();
		} catch (error) {
			if (error instanceof RangeError || error instanceof InactiveError) {
				return "usage-error";
			}
			throw error;
		}
		const words = outputs.flatMap((output) => {
			if (output.type === "close-session") {
				return [`closed 0x${output.code.toString(16)}`];
			}
			if (output.type !== "send" || output.stream !== 1) return [];
			const decoded = decodeControlMessage(output.data);
			return decoded.status === "ok" ? [decoded.value.type] : ["?"];
		});
		return words.length === 0 ? "-" : words.join(" ");
	};
	const updates = table("updates.tsv", 3).map(([name, steps, sent]) => ({
		name,
		steps: steps ?? "",
		sent: sent ?? "",
	}));
	test("updates.tsv has the 16 rows of its README", () => {
		expect(updates.length).toBe(16);
	});
	test.each(updates)("$name ($steps): $sent", ({ steps, sent }) => {
		const session = asked();
		let nextRequestId = 3;
		const update = (parameters: string): SessionOutput[] => {
			const bytes = updateBytes(nextRequestId, parameters);
			nextRequestId += 2;
			return session.receivePeerBidiStreamData(0, bytes, false);
		};
		const accept = (group: bigint, object: bigint) => (): SessionOutput[] =>
			session.acceptFetch(1n, { endLocation: { group, object } });
		const STEPS: Record<string, () => SessionOutput[]> = {
			update: () => update("00"),
			"update-priority": () => update("01 20 c8"),
			"update-forward": () => update("01 10 01"),
			"update-filter": () => update("01 21 01 01"),
			"update-new-group-request": () => update("01 32 07"),
			accept: accept(5n, 1n),
			"accept-before-start": accept(3n, 3n),
			"accept-at-start": accept(3n, 4n),
			"accept-beyond-end": accept(9n, 0n),
			reject: () => session.rejectPeerRequest(1n, 0x10n, ""),
		};
		const got = steps.split(" ").map((step) => {
			const act = STEPS[step];
			if (act === undefined) throw new Error(`unknown step: ${step}`);
			return sentBy(act);
		});
		expect(got.join(" | ")).toBe(sent);
	});
});

// The table both implementations read (fixtures/joining-request-id/README.md):
// whether the request a Joining FETCH names is yet to come. The Rust session
// hands a FETCH that is "held" to its caller; `@xmoq/moqt` serves no Joining
// FETCH and refuses it at once with NOT_SUPPORTED, as the Rust client does.
describe("fixtures/joining-request-id: whether the SUBSCRIBE of a Joining FETCH is yet to come", () => {
	const text = readFileSync(
		join(FIXTURES, "joining-request-id", "held.tsv"),
		"utf8",
	);
	// As the Rust test reads it: every line ends with a newline, none is
	// empty (an empty one fails below), and no name comes twice.
	if (!text.endsWith("\n")) {
		throw new Error("held.tsv does not end with a newline");
	}
	const all = text
		.slice(0, -1)
		.split("\n")
		.map((line) => {
			const [name, sender, used, fetchId, joiningId, outcome, ...more] =
				line.split("\t");
			if (
				name === undefined ||
				(sender !== "client" && sender !== "server") ||
				used === undefined ||
				fetchId === undefined ||
				joiningId === undefined ||
				(outcome !== "held" && outcome !== "refused") ||
				more.length > 0
			) {
				throw new Error(
					`a row of held.tsv is not what its README has: ${line}`,
				);
			}
			return { name, sender, used, fetchId, joiningId, outcome };
		});
	const names = new Set(all.map((row) => row.name));
	if (names.size !== all.length) {
		throw new Error("a name comes twice in held.tsv");
	}
	// This endpoint is a client: the requests it receives are a server's.
	const rows = all.filter((row) => row.sender === "server");

	/** One byte of a varint: every Request ID of the server's rows fits. */
	const byte = (id: string): string => {
		const value = Number(id);
		if (!/^\d+$/.test(id) || value > 63) {
			throw new Error(`Request ID ${id} does not fit a varint of one byte`);
		}
		return value.toString(16).padStart(2, "0");
	};
	/** The captured PUBLISH_NAMESPACE with a Request ID: a request that is no subscription. */
	const publishNamespace = (id: string): Uint8Array => {
		const pairs = fixture("publish-namespace-moqtail-ts").split(" ");
		pairs[3] = byte(id);
		return hex(pairs.join(" "));
	};
	/** The captured Relative Joining FETCH with its Request ID and the one it names; Joining Start 0. */
	const joiningFetch = (id: string, joins: string): Uint8Array => {
		const pairs = fixture("fetch-relative-joining-moqtail-rs").split(" ");
		pairs[3] = byte(id);
		pairs[5] = byte(joins);
		return hex(pairs.join(" "));
	};

	// 10 of the 30 rows of the table have `server` as the sender.
	test("the table has 10 rows for a server's requests", () => {
		expect(rows.length).toBe(10);
	});

	test.each(rows)(
		"$name: used $used, FETCH $fetchId names $joiningId: $outcome",
		({ used, fetchId, joiningId, outcome }) => {
			const session = started();
			let peerStream = 0;
			for (const each of used === "-" ? [] : used.split(",")) {
				// `N:gone`: the request with that ID was over, its stream reset,
				// before the FETCH came. This endpoint refuses a
				// PUBLISH_NAMESPACE by itself, so nothing of it is open either
				// way; the reset is reported as it would be. A row with `:gone`
				// therefore goes the way of the same row without it here.
				const [id, gone, ...more] = each.split(":");
				if (
					id === undefined ||
					more.length > 0 ||
					(gone ?? "gone") !== "gone"
				) {
					throw new Error(`not a Request ID of held.tsv: ${each}`);
				}
				session.receivePeerBidiStreamData(
					peerStream,
					publishNamespace(id),
					false,
				);
				if (gone !== undefined) session.peerBidiStreamReset(peerStream, 1);
				peerStream += 1;
			}
			const outputs = session.receivePeerBidiStreamData(
				peerStream,
				joiningFetch(fetchId, joiningId),
				false,
			);
			const answers = outputs.flatMap((output) => {
				if (output.type === "close-session") return [`closed ${output.code}`];
				if (output.type !== "respond") return [];
				const decoded = decodeControlMessage(output.data);
				return decoded.status === "ok" && decoded.value.type === "REQUEST_ERROR"
					? [`REQUEST_ERROR ${decoded.value.errorCode}`]
					: ["?"];
			});
			// "held": NOT_SUPPORTED (3). "refused": INVALID_JOINING_REQUEST_ID (50).
			expect(answers).toEqual([
				outcome === "held" ? "REQUEST_ERROR 3" : "REQUEST_ERROR 50",
			]);
			expect(session.closed).toBe(false);
		},
	);

	// At the edge of what is remembered (1,024 Request IDs above a gap; the
	// table stops at 63). The FETCH is the ID that makes them 1,025, so the
	// lowest is pushed out before the one the FETCH names is looked at, and
	// then reads as yet to come. The Rust session, which takes the ID of the
	// FETCH first, hands both to its caller.
	test.each([
		{
			name: "3 to 2049 were used, 1 was not; FETCH 2051 names 3, which its own ID pushes out",
			lastUsed: 2049,
			fetchId: 2051,
			joiningId: 3,
		},
		{
			name: "3 to 2051 were used, which pushed 3 out; FETCH 2053 names 5, which its own ID pushes out",
			lastUsed: 2051,
			fetchId: 2053,
			joiningId: 5,
		},
	])("$name: held", ({ lastUsed, fetchId, joiningId }) => {
		/** A Request ID as a varint of 9 bytes. */
		const nine = (id: number): string[] => [
			"ff",
			...(id.toString(16).padStart(16, "0").match(/../g) ?? []),
		];
		const session = started();
		let peerStream = 0;
		for (let id = 3; id <= lastUsed; id += 2) {
			// The start of a SUBSCRIBE_TRACKS, whole: refused, and its stream over.
			session.receivePeerBidiStreamData(
				peerStream,
				hex(["51", "00", "09", ...nine(id)].join(" ")),
				true,
			);
			peerStream += 1;
		}
		expect(session.closed).toBe(false);
		const pairs = fixture("fetch-relative-joining-moqtail-rs").split(" ");
		// The Request ID of 1 byte becomes one of 9: the Message Length
		// grows by 8.
		const length = Number.parseInt(`${pairs[1]}${pairs[2]}`, 16) + 8;
		pairs[5] = joiningId.toString(16).padStart(2, "0");
		pairs.splice(3, 1, ...nine(fetchId));
		pairs.splice(
			1,
			2,
			...(length.toString(16).padStart(4, "0").match(/../g) ?? []),
		);
		const outputs = session.receivePeerBidiStreamData(
			peerStream,
			hex(pairs.join(" ")),
			false,
		);
		const answers = outputs.flatMap((output) => {
			if (output.type === "close-session") return [`closed ${output.code}`];
			if (output.type !== "respond") return [];
			const decoded = decodeControlMessage(output.data);
			return decoded.status === "ok" && decoded.value.type === "REQUEST_ERROR"
				? [`REQUEST_ERROR ${decoded.value.errorCode}`]
				: ["?"];
		});
		// NOT_SUPPORTED, the answer to a Joining FETCH that would be held.
		expect(answers).toEqual(["REQUEST_ERROR 3"]);
	});
});
