import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { InactiveError } from "../../src/core/error.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { byHand, fixture, hex, toHex, utf8 } from "../helpers.ts";

/** The track of `subscribe-moqtail-rs`. */
const TRACK = {
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
};

/** The captured SUBSCRIBE with a Request ID of the server, and with another Track Name where told. */
function subscribeBytes(requestId: number, lastLetter = "74"): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	pairs[23] = lastLetter;
	return hex(pairs.join(" "));
}

/**
 * REQUEST_UPDATE with Request ID 3 and no parameters. Written by hand: no
 * capture of one exists (moqtail's clients send none).
 */
const REQUEST_UPDATE = hex(byHand("request-update-request-id-3"));

/** The captured Relative Joining FETCH, as Request ID 3 of the server, naming its SUBSCRIBE with Request ID 1. */
function joiningFetchBytes(): Uint8Array {
	const pairs = fixture("fetch-relative-joining-moqtail-rs").split(" ");
	pairs[3] = "03";
	pairs[5] = "01";
	return hex(pairs.join(" "));
}

function started(maxPeerRequests?: number): Session {
	const session = createSession({
		limits: maxPeerRequests === undefined ? {} : { maxPeerRequests },
	});
	session.start();
	return session;
}

/** A session whose peer sent the SUBSCRIBE with Request ID 1 on its stream 0; the session named that stream 1. */
function asked(): Session {
	const session = started();
	session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
	return session;
}

/** That session, with the SUBSCRIBE accepted under Track Alias 0. */
function serving(): Session {
	const session = asked();
	session.acceptSubscribe(1n, {});
	return session;
}

/** An output in a line: its type, where it acts, and the message it carries. */
function brief(output: SessionOutput): string {
	const message = (data: Uint8Array): string => {
		const decoded = decodeControlMessage(data);
		if (decoded.status !== "ok") return `bytes ${toHex(data)}`;
		const { value } = decoded;
		switch (value.type) {
			case "REQUEST_ERROR":
				return `REQUEST_ERROR ${value.errorCode}`;
			case "SUBSCRIBE_OK":
				return `SUBSCRIBE_OK alias ${value.trackAlias}`;
			case "PUBLISH_DONE":
				return `PUBLISH_DONE ${value.statusCode} streams ${value.streamCount}`;
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
			return `send ${output.stream}: ${message(output.data)}`;
		case "finish":
			return `finish ${output.stream}`;
		case "reset":
			return `reset ${output.stream} code ${output.code}`;
		case "stop-peer-direction":
			return `stop peer direction ${output.stream} code ${output.code}`;
		case "open-uni-stream":
			return `open uni ${output.stream}`;
		case "close-session":
			return `close ${output.code}`;
		case "event": {
			const { event } = output;
			switch (event.type) {
				case "peer-subscribe":
					return `event peer-subscribe ${event.requestId}`;
				case "request-closed":
					return `event request-closed ${event.requestId} ${event.reason}`;
				case "peer-request-rejected":
					return `event peer-request-rejected ${event.requestId} code ${event.errorCode}`;
				case "peer-direction-ended":
					return `event peer-direction-ended ${event.requestId} ${event.how}`;
				default:
					return `event ${event.type}`;
			}
		}
		default:
			return output.type;
	}
}

const briefly = (outputs: SessionOutput[]): string[] => outputs.map(brief);

const SUBGROUP = {
	groupId: 0n,
	subgroupId: { mode: "Zero" },
	publisherPriority: 0,
	properties: false,
	endOfGroup: true,
} as const;

describe("a SUBSCRIBE of the peer", () => {
	test("is handed on, with its stream taken over and its parameters", () => {
		const session = started();
		const outputs = session.receivePeerBidiStreamData(
			5,
			subscribeBytes(1),
			false,
		);
		expect(briefly(outputs)).toEqual([
			"peer-stream-identified",
			"adopt peer 5 as 1",
			"event peer-subscribe 1",
		]);
		expect(outputs.at(-1)).toEqual({
			type: "event",
			event: {
				type: "peer-subscribe",
				requestId: 1n,
				...TRACK,
				parameters: {
					forward: true,
					subscriberPriority: 128,
					subscriptionFilter: { filterType: "LargestObject" },
					groupOrder: "Ascending",
				},
			},
		});
	});

	test("is not looked at before it is whole", () => {
		const session = started();
		const bytes = subscribeBytes(1);
		expect(
			session.receivePeerBidiStreamData(0, bytes.subarray(0, 20), false),
		).toEqual([]);
		expect(
			briefly(session.receivePeerBidiStreamData(0, bytes.subarray(20), false)),
		).toEqual([
			"peer-stream-identified",
			"adopt peer 0 as 1",
			"event peer-subscribe 1",
		]);
	});

	test("accepted, gets SUBSCRIBE_OK with the first Track Alias and what the application gives", () => {
		const session = asked();
		const { value, outputs } = session.acceptSubscribe(1n, {
			parameters: { largestObject: { group: 3n, object: 2n } },
		});
		expect(value).toBe(0n);
		const [send] = outputs;
		if (send?.type !== "send" || outputs.length !== 1) {
			throw new Error("the answer is not one message");
		}
		expect(send.stream).toBe(1);
		// The bytes of the capture with LARGEST_OBJECT: the same alias and the
		// same Location.
		expect(toHex(send.data)).toBe(
			fixture("subscribe-ok-largest-object-moqtail-relay"),
		);
	});

	test.each([
		{ name: "nothing else", own: [], alias: 0n },
		{ name: "a PUBLISH of this endpoint under 0", own: [0n], alias: 1n },
		{ name: "its PUBLISHes under 0 and 1", own: [0n, 1n], alias: 2n },
		{ name: "a PUBLISH of this endpoint under 1", own: [1n], alias: 0n },
	])("accepted beside $name, gets Track Alias $alias", ({ own, alias }) => {
		const session = started();
		for (const trackAlias of own) {
			session.publish({
				trackNamespace: [utf8("other")],
				trackName: utf8(`track ${trackAlias}`),
				trackAlias,
			});
		}
		session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
		expect(session.acceptSubscribe(1n, {}).value).toBe(alias);
	});

	test("a second one is given the next Track Alias, and the alias of one that ended is not given again at once", () => {
		const session = serving();
		session.publishDone(1n, 2n, "");
		session.receivePeerBidiStreamData(1, subscribeBytes(3), false);
		expect(session.acceptSubscribe(3n, {}).value).toBe(1n);
	});

	test("accepted, is a subscription this endpoint publishes: Objects, PUBLISH_DONE, and the wait for the peer's end", () => {
		const session = serving();
		const opened = session.openSubgroup(1n, SUBGROUP);
		expect(briefly(opened.outputs)).toEqual(["open uni 2"]);
		session.closeSubgroup(opened.value);
		expect(briefly(session.publishDone(1n, 2n, ""))).toEqual([
			"send 1: PUBLISH_DONE 2 streams 1",
			"finish 1",
		]);
		// The subscriber finishes its direction: reported under the peer's
		// number for the stream.
		expect(
			briefly(session.receivePeerBidiStreamData(0, new Uint8Array(0), true)),
		).toEqual(["event peer-direction-ended 1 fin"]);
	});

	test("a subscriber that does not end its direction after PUBLISH_DONE is stopped when the wait is over", () => {
		const session = serving();
		session.publishDone(1n, 2n, "");
		expect(briefly(session.expirePeerEnd(1n))).toEqual([
			"stop peer direction 1 code 1",
		]);
	});

	test("rejected, gets REQUEST_ERROR and a FIN, and what it still sends is ignored", () => {
		const session = asked();
		expect(briefly(session.rejectPeerRequest(1n, 0x10n, "no"))).toEqual([
			"send 1: REQUEST_ERROR 16",
			"event peer-request-rejected 1 code 16",
			"finish 1",
		]);
		// A REQUEST_UPDATE that crossed the answer: it gets none, and its
		// Request ID is used up (§10.1).
		expect(session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false)).toEqual(
			[],
		);
		expect(
			briefly(
				session.receivePeerBidiStreamData(9, hex("0d 00 02 03 00"), false),
			).at(-1),
		).toBe("close 4");
	});

	test.each([
		{
			name: "accept",
			answer: (session: Session) => session.acceptSubscribe(1n, {}),
		},
		{
			name: "reject",
			answer: (session: Session) => session.rejectPeerRequest(1n, 3n, ""),
		},
	])("cannot be answered twice: $name after an answer", ({ answer }) => {
		const session = serving();
		expect(() => answer(session)).toThrow(InactiveError);
	});

	test("accepted with a parameter that is none of SUBSCRIBE_OK, still waits for its answer", () => {
		const session = asked();
		expect(() =>
			session.acceptSubscribe(1n, { parameters: { forward: true } }),
		).toThrow(RangeError);
		expect(session.acceptSubscribe(1n, {}).value).toBe(0n);
	});

	test("that asks for Forward State 0 gets no Objects (§5.1)", () => {
		const session = started();
		const pairs = fixture("subscribe-moqtail-rs").split(" ");
		pairs[3] = "01";
		// FORWARD (0x10) is the first parameter: its value is the byte after
		// the count and the type.
		expect(pairs.slice(24, 27)).toEqual(["04", "10", "01"]);
		pairs[26] = "00";
		session.receivePeerBidiStreamData(0, hex(pairs.join(" ")), false);
		session.acceptSubscribe(1n, {});
		expect(() => session.openSubgroup(1n, SUBGROUP)).toThrow("Forward State 0");
	});
});

describe("a SUBSCRIBE of the peer, and what the subscriber does next", () => {
	// Each row: the session, what the peer does on the stream of its
	// SUBSCRIBE, and what that leads to.
	const rows: {
		name: string;
		session: () => Session;
		act: (session: Session) => SessionOutput[];
		outputs: string[];
	}[] = [
		{
			name: "REQUEST_UPDATE before the answer: refused, and the request is over (§10.9)",
			session: asked,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false),
			outputs: [
				"send 1: REQUEST_ERROR 3",
				"event request-closed 1 update-failed",
				"finish 1",
			],
		},
		{
			name: "REQUEST_UPDATE after SUBSCRIBE_OK: refused, and PUBLISH_DONE UPDATE_FAILED (§10.9.1)",
			session: serving,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false),
			outputs: [
				"send 1: REQUEST_ERROR 3",
				"send 1: PUBLISH_DONE 8 streams 0",
				"event request-closed 1 update-failed",
				"finish 1",
			],
		},
		{
			name: "a reset of its direction before the answer: the request is gone",
			session: asked,
			act: (session: Session) => session.peerBidiStreamReset(0, 1),
			outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
		},
		{
			name: "a reset of its direction after SUBSCRIBE_OK: the subscription is cancelled",
			session: serving,
			act: (session: Session) => session.peerBidiStreamReset(0, 1),
			outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
		},
		{
			name: "STOP_SENDING before the answer: the request is gone",
			session: asked,
			act: (session: Session) => session.localStreamStopped(1, 1),
			outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
		},
		{
			name: "STOP_SENDING after SUBSCRIBE_OK: the subscription is cancelled (§5.1)",
			session: serving,
			act: (session: Session) => session.localStreamStopped(1, 1),
			outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
		},
		{
			name: "a FIN before the answer: nothing, the request still waits",
			session: asked,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			outputs: [],
		},
		{
			name: "a FIN after SUBSCRIBE_OK: nothing, the subscription goes on",
			session: serving,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
			outputs: [],
		},
		{
			name: "PUBLISH_DONE, which is the publisher's to send: the session is closed",
			session: serving,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(
					0,
					hex(fixture("publish-done-moqtail-relay")),
					false,
				),
			outputs: ["close 3"],
		},
		{
			name: "a FIN inside a message: the session is closed",
			session: asked,
			act: (session: Session) =>
				session.receivePeerBidiStreamData(0, hex("02 00"), true),
			outputs: ["close 3"],
		},
	];
	test.each(rows)("$name", ({ session, act, outputs }) => {
		expect(briefly(act(session()))).toEqual(outputs);
	});

	test("a subscriber that finished its direction before the answer is still answered, and can still stop the stream", () => {
		const session = asked();
		session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
		expect(briefly(session.acceptSubscribe(1n, {}).outputs)).toEqual([
			"send 1: SUBSCRIBE_OK alias 0",
		]);
		expect(briefly(session.localStreamStopped(1, 1))).toEqual([
			"event request-closed 1 peer-reset",
			"reset 1 code 1",
		]);
	});

	test("a REQUEST_UPDATE that follows the SUBSCRIBE in one read is refused with it", () => {
		const session = started();
		const bytes = new Uint8Array([...subscribeBytes(1), ...REQUEST_UPDATE]);
		expect(briefly(session.receivePeerBidiStreamData(0, bytes, false))).toEqual(
			[
				"peer-stream-identified",
				"adopt peer 0 as 1",
				"event peer-subscribe 1",
				"send 1: REQUEST_ERROR 3",
				"event request-closed 1 update-failed",
				"finish 1",
			],
		);
	});

	test("a subscription that was cancelled leaves its Track free to be asked for again", () => {
		const session = serving();
		session.peerBidiStreamReset(0, 1);
		expect(
			briefly(session.receivePeerBidiStreamData(1, subscribeBytes(3), false)),
		).toContain("event peer-subscribe 3");
	});
});

describe("what is kept for a SUBSCRIBE of the peer", () => {
	const peerFin = (session: Session): void => {
		session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
	};
	// Each row: what happens to the request, in order, and how many streams
	// the session keeps something of afterwards. A session that has only
	// started keeps none.
	const rows: {
		name: string;
		steps: ((session: Session) => void)[];
		held: number;
	}[] = [
		{ name: "it waits for its answer", steps: [], held: 2 },
		{
			name: "rejected, and the peer has not ended its direction",
			steps: [(session) => session.rejectPeerRequest(1n, 0x10n, "")],
			held: 2,
		},
		{
			name: "rejected, then the peer finishes its direction",
			steps: [(session) => session.rejectPeerRequest(1n, 0x10n, ""), peerFin],
			held: 0,
		},
		{
			name: "the peer finishes its direction, then it is rejected",
			steps: [peerFin, (session) => session.rejectPeerRequest(1n, 0x10n, "")],
			held: 0,
		},
		{
			name: "accepted, ended with PUBLISH_DONE, then the peer finishes its direction",
			steps: [
				(session) => session.acceptSubscribe(1n, {}),
				(session) => session.publishDone(1n, 2n, ""),
				peerFin,
			],
			held: 0,
		},
		{
			name: "accepted, the peer finishes its direction, then PUBLISH_DONE",
			steps: [
				(session) => session.acceptSubscribe(1n, {}),
				peerFin,
				(session) => session.publishDone(1n, 2n, ""),
			],
			held: 0,
		},
		{
			name: "accepted, and the peer resets its direction",
			steps: [
				(session) => session.acceptSubscribe(1n, {}),
				(session) => session.peerBidiStreamReset(0, 1),
			],
			held: 0,
		},
		{
			name: "the wait after PUBLISH_DONE runs out",
			steps: [
				(session) => session.acceptSubscribe(1n, {}),
				(session) => session.publishDone(1n, 2n, ""),
				(session) => session.expirePeerEnd(1n),
				// The stop ends the peer's direction for the I/O layer, which
				// reports it like an end.
				peerFin,
			],
			held: 0,
		},
	];
	test("a session that has started keeps nothing of any stream", () => {
		expect(started().heldStreams()).toBe(0);
	});
	test.each(rows)(
		"$name: something is kept of $held streams",
		({ steps, held }) => {
			const session = asked();
			for (const step of steps) step(session);
			expect(session.heldStreams()).toBe(held);
		},
	);

	test("the Track Alias of an accepted SUBSCRIBE is in use: this endpoint cannot PUBLISH under it (§11.1)", () => {
		const session = serving();
		expect(() =>
			session.publish({
				trackNamespace: [utf8("other")],
				trackName: utf8("track"),
				trackAlias: 0n,
			}),
		).toThrow("Track Alias 0 is already in use");
	});

	test("a REQUEST_UPDATE before the answer uses up its Request ID (§10.1)", () => {
		const session = asked();
		session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false);
		// Another request with Request ID 3, the one of the update.
		expect(
			briefly(
				session.receivePeerBidiStreamData(9, hex("0d 00 02 03 00"), false),
			).at(-1),
		).toBe("close 4");
	});
});

describe("what the session answers by itself", () => {
	test("EXCESSIVE_LOAD for one request more than `maxPeerRequests`, waiting or accepted", () => {
		const session = started(2);
		session.receivePeerBidiStreamData(0, subscribeBytes(1, "61"), false);
		session.acceptSubscribe(1n, {});
		session.receivePeerBidiStreamData(1, subscribeBytes(3, "62"), false);
		expect(
			briefly(
				session.receivePeerBidiStreamData(2, subscribeBytes(5, "63"), false),
			),
		).toEqual([
			"peer-stream-identified",
			"respond peer 2: REQUEST_ERROR 9",
			"event peer-request-rejected 5 code 9",
		]);
		// One of the two is rejected, and is kept until its stream is gone:
		// there is no room yet.
		session.rejectPeerRequest(3n, 0x10n, "");
		expect(
			briefly(
				session.receivePeerBidiStreamData(3, subscribeBytes(7, "64"), false),
			),
		).toContain("respond peer 3: REQUEST_ERROR 9");
		// The peer ends its direction of that stream: there is room again.
		session.receivePeerBidiStreamData(1, new Uint8Array(0), true);
		expect(
			briefly(
				session.receivePeerBidiStreamData(4, subscribeBytes(9, "65"), false),
			),
		).toContain("event peer-subscribe 9");
	});

	test("the Request ID of an open SUBSCRIBE is a duplicate also when it is no longer remembered among the IDs (§10.1)", () => {
		const session = started();
		// Request ID 1 never comes, so 3 stays remembered above the gap.
		session.receivePeerBidiStreamData(0, subscribeBytes(3), false);
		// 1,100 further requests, each the start of a TRACK_STATUS whose
		// Request ID is written as a varint of 9 bytes: more than the 1,024
		// IDs that are remembered, so the 3 is pushed out.
		for (let index = 0; index < 1100; index++) {
			const requestId = (5 + index * 2).toString(16).padStart(16, "0");
			const bytes = `0d 00 09 ff ${requestId.match(/../g)?.join(" ")}`;
			session.receivePeerBidiStreamData(1 + index, hex(bytes), true);
		}
		expect(session.closed).toBe(false);
		// The 3 again, for another Track.
		expect(
			briefly(
				session.receivePeerBidiStreamData(5000, subscribeBytes(3, "75"), false),
			).at(-1),
		).toBe("close 4");
	});

	test("this endpoint cannot PUBLISH a Track the peer has asked it for (§5.1)", () => {
		const session = asked();
		expect(() => session.publish({ ...TRACK, trackAlias: 5n })).toThrow(
			RangeError,
		);
	});

	// A Joining FETCH names a subscription of the session by its Request ID
	// (§10.12.2). Relative, for the Request ID in the last column.
	test.each([
		{
			name: "a SUBSCRIBE of the peer that waits",
			session: asked,
			code: 3n,
		},
		{ name: "one that was accepted", session: serving, code: 3n },
		{
			name: "one that ended with PUBLISH_DONE",
			session: () => {
				const session = serving();
				session.publishDone(1n, 2n, "");
				return session;
			},
			code: 0x32n,
		},
		// The Request ID 1 was not used, and the FETCH has 3: the SUBSCRIBE
		// can be on its way. The Rust session hands such a FETCH on, and the
		// Rust client refuses it as it does any FETCH.
		{ name: "a request that is yet to come", session: started, code: 3n },
	])(
		"a Joining FETCH that names $name is refused with $code",
		({ session, code }) => {
			const outputs = session().receivePeerBidiStreamData(
				8,
				joiningFetchBytes(),
				false,
			);
			expect(briefly(outputs)).toContain(
				`respond peer 8: REQUEST_ERROR ${code}`,
			);
		},
	);
});

// The table both implementations read (fixtures/peer-subscribe/README.md):
// what becomes of a SUBSCRIBE of the peer, by what went before it.
describe("fixtures/peer-subscribe: what becomes of a SUBSCRIBE of the peer", () => {
	const rows = readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"..",
			"..",
			"fixtures",
			"peer-subscribe",
			"opened.tsv",
		),
		"utf8",
	)
		.split("\n")
		.filter((line) => line.length > 0)
		.map((line) => {
			const [name, before, sent, outcome] = line.split("\t");
			if (
				name === undefined ||
				before === undefined ||
				sent === undefined ||
				outcome === undefined ||
				line.split("\t").length !== 4
			) {
				throw new Error(
					`a row of opened.tsv does not have four fields: ${line}`,
				);
			}
			return { name, before, sent, outcome };
		});

	/** The earlier SUBSCRIBE of the README: Request ID 1, for the track "object", on the peer's stream 0. */
	const earlier = (session: Session): void => {
		session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
	};
	const BEFORE: Record<string, (session: Session) => void> = {
		nothing: () => {},
		publish: (session) => {
			session.publish({ ...TRACK, trackAlias: 1n });
		},
		subscribe: earlier,
		"subscribe-accepted": (session) => {
			earlier(session);
			session.acceptSubscribe(1n, {});
		},
		"subscribe-done": (session) => {
			earlier(session);
			session.acceptSubscribe(1n, {});
			session.publishDone(1n, 2n, "");
		},
		"subscribe-rejected": (session) => {
			earlier(session);
			session.rejectPeerRequest(1n, 0x10n, "");
		},
		"subscribe-rejected-then-update": (session) => {
			earlier(session);
			session.rejectPeerRequest(1n, 0x10n, "");
			session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false);
		},
		"subscribe-refused-then-update": (session) => {
			earlier(session);
			// The row's own stream is 1; this one is the second SUBSCRIBE's.
			session.receivePeerBidiStreamData(2, subscribeBytes(3), false);
			session.receivePeerBidiStreamData(
				2,
				hex(byHand("request-update-request-id-5")),
				false,
			);
		},
	};

	/** The outcome in the words of the table. */
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
			if (output.type === "event" && output.event.type === "peer-subscribe") {
				return "handed";
			}
		}
		return "nothing";
	};

	test("the table has the twelve rows of its README", () => {
		expect(rows.length).toBe(12);
	});

	test.each(rows)(
		"$name, after $before: $outcome",
		({ before, sent, outcome }) => {
			const session = started();
			const prepare = BEFORE[before];
			if (prepare === undefined) throw new Error(`unknown state: ${before}`);
			prepare(session);
			const bytes = hex((sent.match(/../g) ?? []).join(" "));
			expect(
				outcomeOf(session.receivePeerBidiStreamData(1, bytes, false)),
			).toBe(outcome);
		},
	);
});
