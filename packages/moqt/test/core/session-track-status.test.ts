import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { InactiveError } from "../../src/core/error.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixture, hex, toHex, utf8 } from "../helpers.ts";

// TRACK_STATUS (§10.14): one answer, REQUEST_OK (TRACK_STATUS_OK) with what a
// SUBSCRIBE_OK would have had, or REQUEST_ERROR; "the subscriber cannot send
// REQUEST_UPDATE". The same cases as the Rust session's
// (`track_status_transitions`, `a_track_status_of_the_peer_is_answered_once`).

/** The captured TRACK_STATUS of the moqtail relay, Request ID 1. */
const PEER_TRACK_STATUS = hex(fixture("track-status-moqtail-relay"));

/** REQUEST_UPDATE with Request ID 3 and no parameters. */
const REQUEST_UPDATE = hex("02 00 02 03 00");

function started(maxPeerRequests?: number): Session {
	const session = createSession({
		limits: maxPeerRequests === undefined ? {} : { maxPeerRequests },
	});
	session.start();
	return session;
}

/** An output in a line: its type, where it acts, and the message it carries. */
function brief(output: SessionOutput): string {
	const message = (data: Uint8Array): string => {
		const decoded = decodeControlMessage(data);
		if (decoded.status !== "ok") return `bytes ${toHex(data)}`;
		const { value } = decoded;
		return value.type === "REQUEST_ERROR"
			? `REQUEST_ERROR ${value.errorCode}`
			: `${value.type} ${toHex(data)}`;
	};
	switch (output.type) {
		case "adopt-peer-stream":
			return `adopt peer ${output.peerStream} as ${output.stream}`;
		case "open-bidi-stream":
			return `open bidi ${output.stream}: ${message(output.data)}`;
		case "send":
			return `send ${output.stream}: ${message(output.data)}`;
		case "finish":
			return `finish ${output.stream}`;
		case "reset":
			return `reset ${output.stream} code ${output.code}`;
		case "close-session":
			return `close ${output.code}`;
		case "event": {
			const { event } = output;
			switch (event.type) {
				case "request-error":
					return `event request-error ${event.requestId} code ${event.errorCode}`;
				case "request-closed":
					return `event request-closed ${event.requestId} ${event.reason}`;
				case "peer-request-rejected":
					return `event peer-request-rejected ${event.requestId} type ${event.messageType} code ${event.errorCode}`;
				case "peer-direction-ended":
					return `event peer-direction-ended ${event.requestId} ${event.how}`;
				case "peer-track-status":
				case "track-status-ok":
					return `event ${event.type} ${event.requestId}`;
				default:
					return `event ${event.type}`;
			}
		}
		default:
			return output.type;
	}
}

const briefly = (outputs: SessionOutput[]): string[] => outputs.map(brief);

describe("a TRACK_STATUS of this endpoint", () => {
	test("is sent on a request stream of its own", () => {
		const { value, outputs } = started().trackStatus({
			trackNamespace: [utf8("a")],
			trackName: utf8("b"),
		});
		expect(value).toBe(0n);
		// Stream 0 is the control stream.
		expect(briefly(outputs)).toEqual([
			"open bidi 1: TRACK_STATUS 0d 00 07 00 01 01 61 01 62 00",
		]);
	});

	test("is refused at the call for a parameter other than AUTHORIZATION_TOKEN (§10.2.2)", () => {
		const session = started();
		expect(() =>
			session.trackStatus({
				trackNamespace: [utf8("a")],
				trackName: utf8("b"),
				parameters: { rendezvousTimeout: 1n },
			}),
		).toThrow(RangeError);
		// No Request ID was used up.
		expect(
			session.trackStatus({ trackNamespace: [utf8("a")], trackName: utf8("b") })
				.value,
		).toBe(0n);
	});

	// Each step is what the peer sends on the request stream, in a row, and
	// what the session does with it.
	test.each([
		{
			name: "answered with LARGEST_OBJECT {4, 1} and the Track Property 2 = 5",
			steps: [
				{
					// REQUEST_OK: LARGEST_OBJECT {4, 1}; one Track Property, type 2, value 5.
					bytes: "07 00 06 01 09 04 01 02 05",
					outputs: ["event track-status-ok 0", "finish 1"],
				},
				// The request is over: a second answer is a violation.
				{ bytes: "07 00 01 00", outputs: ["close 3"] },
			],
		},
		{
			name: "rejected",
			steps: [
				{
					// REQUEST_ERROR DOES_NOT_EXIST, no Retry Interval, no reason.
					bytes: "05 00 03 10 00 00",
					outputs: ["event request-error 0 code 16", "finish 1"],
				},
			],
		},
		{
			// §10.2.11: LARGEST_OBJECT alone may appear in it.
			name: "answered with a REQUEST_OK that carries FORWARD",
			steps: [{ bytes: "07 00 03 01 10 01", outputs: ["close 3"] }],
		},
		{
			name: "answered with a SUBSCRIBE_OK",
			steps: [{ bytes: "04 00 02 01 00", outputs: ["close 3"] }],
		},
	] satisfies {
		name: string;
		steps: { bytes: string; outputs: string[] }[];
	}[])("$name", ({ steps }) => {
		const session = started();
		session.trackStatus({ trackNamespace: [utf8("a")], trackName: utf8("b") });
		for (const { bytes, outputs } of steps) {
			expect(
				briefly(session.receiveRequestStreamData(1, hex(bytes), false)),
			).toEqual(outputs);
		}
	});

	test("hands on the parameters and Track Properties of its answer", () => {
		const session = started();
		session.trackStatus({ trackNamespace: [utf8("a")], trackName: utf8("b") });
		expect(
			session.receiveRequestStreamData(
				1,
				hex("07 00 06 01 09 04 01 02 05"),
				false,
			)[0],
		).toEqual({
			type: "event",
			event: {
				type: "track-status-ok",
				requestId: 0n,
				parameters: { largestObject: { group: 4n, object: 1n } },
				trackProperties: [{ type: 2n, value: 5n }],
			},
		});
	});
});

describe("a TRACK_STATUS of the peer", () => {
	test("is handed on, with its stream taken over", () => {
		const outputs = started().receivePeerBidiStreamData(
			5,
			PEER_TRACK_STATUS,
			false,
		);
		expect(briefly(outputs)).toEqual([
			"peer-stream-identified",
			"adopt peer 5 as 1",
			"event peer-track-status 1",
		]);
		expect(outputs.at(-1)).toEqual({
			type: "event",
			event: {
				type: "peer-track-status",
				requestId: 1n,
				trackNamespace: [utf8("xmoq"), utf8("phase0")],
				trackName: utf8("nobody-published-this"),
				parameters: {},
			},
		});
	});

	// Each step acts on the TRACK_STATUS with Request ID 1, on the peer's
	// stream 0, which the session named 1.
	test.each([
		{
			name: "answered with LARGEST_OBJECT and a Track Property, once",
			steps: [
				{
					act: (session: Session) =>
						session.acceptTrackStatus(1n, {
							parameters: { largestObject: { group: 4n, object: 1n } },
							trackProperties: [{ type: 2n, value: 5n }],
						}),
					outputs: [
						"send 1: REQUEST_OK 07 00 06 01 09 04 01 02 05",
						"finish 1",
					],
				},
				{
					act: (session: Session) => session.acceptTrackStatus(1n, {}),
					throws: InactiveError,
				},
				{
					act: (session: Session) => session.rejectPeerRequest(1n, 0x10n, ""),
					throws: InactiveError,
				},
			],
		},
		{
			name: "answered with a parameter other than LARGEST_OBJECT, which is refused, then rejected",
			steps: [
				{
					act: (session: Session) =>
						session.acceptTrackStatus(1n, { parameters: { expires: 1n } }),
					throws: RangeError,
				},
				{
					act: (session: Session) => session.rejectPeerRequest(1n, 0x10n, ""),
					outputs: [
						"send 1: REQUEST_ERROR 16",
						"event peer-request-rejected 1 type 13 code 16",
						"finish 1",
					],
				},
			],
		},
		{
			// §10.14: "the subscriber cannot send REQUEST_UPDATE".
			name: "sent a REQUEST_UPDATE",
			steps: [
				{
					act: (session: Session) =>
						session.receivePeerBidiStreamData(0, REQUEST_UPDATE, false),
					outputs: ["close 3"],
				},
			],
		},
		{
			// The subscriber has said everything with the TRACK_STATUS.
			name: "whose peer ended its direction before the answer",
			steps: [
				{
					act: (session: Session) =>
						session.receivePeerBidiStreamData(0, new Uint8Array(0), true),
					outputs: [],
				},
				{
					act: (session: Session) => session.acceptTrackStatus(1n, {}),
					outputs: [
						"send 1: REQUEST_OK 07 00 01 00",
						"finish 1",
						"event peer-direction-ended 1 fin",
					],
				},
			],
		},
		{
			name: "whose peer stopped this endpoint's direction before the answer",
			steps: [
				{
					act: (session: Session) => session.localStreamStopped(1, 0),
					outputs: ["event request-closed 1 peer-reset", "reset 1 code 1"],
				},
				{
					act: (session: Session) => session.acceptTrackStatus(1n, {}),
					throws: InactiveError,
				},
			],
		},
	] satisfies {
		name: string;
		steps: (
			| { act: (session: Session) => SessionOutput[]; outputs: string[] }
			| {
					act: (session: Session) => SessionOutput[];
					throws: typeof RangeError | typeof InactiveError;
			  }
		)[];
	}[])("$name", ({ steps }) => {
		const session = started();
		session.receivePeerBidiStreamData(0, PEER_TRACK_STATUS, false);
		for (const step of steps) {
			if ("throws" in step) {
				expect(() => step.act(session)).toThrow(step.throws);
			} else {
				expect(briefly(step.act(session))).toEqual(step.outputs);
			}
		}
	});

	test("is refused with EXCESSIVE_LOAD while as many requests of the peer are kept as may be", () => {
		const session = started(1);
		session.receivePeerBidiStreamData(0, PEER_TRACK_STATUS, false);
		const second = hex(fixture("track-status-moqtail-relay"));
		second[3] = 3;
		expect(
			briefly(session.receivePeerBidiStreamData(1, second, false)).slice(1),
		).toEqual(["respond", "event peer-request-rejected 3 type 13 code 9"]);
	});
});
