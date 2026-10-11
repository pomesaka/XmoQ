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
function subscribeBytes(): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = "01";
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
				case "publish-done-peer-ended":
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
		expect(briefly(session.finishFetch(1n))).toEqual(["finish 2", "finish 1"]);
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
		// Another request with Request ID 3: the start of a TRACK_STATUS.
		expect(
			briefly(
				prepared.receivePeerBidiStreamData(9, hex("0d 00 02 03 00"), false),
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
		expect(briefly(session.expirePublishDonePeerEnd(1n))).toEqual([
			"stop peer direction 1 code 1",
		]);
	});

	test("served, hears how the subscriber ended its direction", () => {
		const session = serving();
		session.finishFetch(1n);
		expect(
			briefly(session.receivePeerBidiStreamData(0, new Uint8Array(0), true)),
		).toEqual(["event peer-ended 1 fin"]);
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
	test("opened.tsv has the 22 rows of its README", () => {
		expect(opened.length).toBe(22);
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
	test("updates.tsv has the 13 rows of its README", () => {
		expect(updates.length).toBe(13);
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
			accept: accept(5n, 1n),
			"accept-before-start": accept(3n, 3n),
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
