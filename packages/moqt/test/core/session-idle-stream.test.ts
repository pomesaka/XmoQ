import { describe, expect, test } from "bun:test";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex, MOQTAIL_RELAY_SETUP, utf8 } from "../helpers.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

/** A session with one subscription, Request ID 0 on stream 1, accepted with Track Alias 9 when `accepted`. */
function subscribed(accepted: boolean): Session {
	const session = createSession();
	session.start();
	session.subscribe(TRACK);
	if (accepted)
		session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
	return session;
}

/** The outputs that are about the idle time or the stream's stop, in a line each. */
function briefly(outputs: SessionOutput[]): string[] {
	return outputs.flatMap((output) => {
		switch (output.type) {
			case "watch-peer-stream-idle":
				return [`watch ${output.peerStream}`];
			case "stop-sending":
				return [`stop ${output.peerStream} code ${output.code}`];
			case "close-session":
				return [`close ${output.code}`];
			case "event":
				switch (output.event.type) {
					case "subgroup-abandoned":
						return [`abandoned: ${output.event.reason}`];
					case "object":
						return [`Object ${output.event.object.objectId}`];
					default:
						return [];
				}
			default:
				return [];
		}
	});
}

// Peer stream 7 throughout. The Subgroup Header is Track Alias 9, Group 0,
// Subgroup 0, priority 0 (type 0x14).
describe("when a subgroup stream gets the idle time", () => {
	test.each<{
		name: string;
		chunks: { bytes: string; fin: boolean }[];
		expected: string[][];
	}>([
		{
			name: "with its whole header, and not with its type",
			chunks: [
				{ bytes: "14 09", fin: false },
				{ bytes: "00 00 00", fin: false },
				{ bytes: "00 01 61", fin: false },
			],
			expected: [[], ["watch 7"], ["Object 0"]],
		},
		{
			name: "not when its header comes with its end",
			chunks: [{ bytes: "14 09 00 00 00 00 01 61", fin: true }],
			expected: [["Object 0"]],
		},
	])("$name", ({ chunks, expected }) => {
		const session = subscribed(true);
		session.peerStreamOpened("uni", 7);
		expect(
			chunks.map(({ bytes, fin }) =>
				briefly(session.receivePeerUniStreamData(7, hex(bytes), fin)),
			),
		).toEqual(expected);
	});

	test("not when it is stopped at its header: a Track Alias no SUBSCRIBE can establish", () => {
		const session = createSession();
		session.start();
		session.peerStreamOpened("uni", 7);
		expect(
			briefly(
				session.receivePeerUniStreamData(7, hex("14 09 00 00 00"), false),
			),
		).toEqual(["stop 7 code 1", "abandoned: unknown Track Alias"]);
	});
});

describe("a subgroup stream whose idle time ran out", () => {
	test.each<{
		name: string;
		accepted: boolean;
		sent: string;
		expected: string[];
	}>([
		{
			name: "of an Established subscription is stopped with INTERNAL_ERROR",
			accepted: true,
			sent: "14 09 00 00 00 00 01 61",
			expected: ["stop 7 code 0", "abandoned: no bytes within the idle time"],
		},
		{
			name: "held for a SUBSCRIBE that is Pending is stopped with INTERNAL_ERROR",
			accepted: false,
			sent: "14 09 00 00 00 00 01 61",
			expected: ["stop 7 code 0", "abandoned: no bytes within the idle time"],
		},
	])("$name", ({ accepted, sent, expected }) => {
		const session = subscribed(accepted);
		session.peerStreamOpened("uni", 7);
		session.receivePeerUniStreamData(7, hex(sent), false);
		expect(briefly(session.peerUniStreamIdle(7))).toEqual(expected);
		expect(session.closed).toBe(false);
	});

	test("held Objects are not delivered by the SUBSCRIBE_OK that comes after", () => {
		const session = subscribed(false);
		session.peerStreamOpened("uni", 7);
		session.receivePeerUniStreamData(7, hex("14 09 00 00 00 00 01 61"), false);
		session.peerUniStreamIdle(7);
		expect(
			briefly(
				session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false),
			),
		).toEqual([]);
	});

	test("what still arrives on it is dropped, and its end is no error", () => {
		const session = subscribed(true);
		session.peerStreamOpened("uni", 7);
		session.receivePeerUniStreamData(7, hex("14 09 00 00 00"), false);
		session.peerUniStreamIdle(7);
		expect(
			briefly(session.receivePeerUniStreamData(7, hex("00 01 61"), true)),
		).toEqual([]);
		expect(session.closed).toBe(false);
	});
});

// A time that is reported late, or twice, does no harm.
describe("an idle time that finds nothing to stop", () => {
	test.each<{ name: string; act: (session: Session) => void }>([
		{
			name: "the peer's control stream",
			act: (session) => {
				session.receivePeerUniStreamData(7, hex(MOQTAIL_RELAY_SETUP), false);
			},
		},
		{
			name: "a stream that has not said what it is",
			act: (session) => {
				session.receivePeerUniStreamData(7, hex("14 09"), false);
			},
		},
		{
			name: "a fetch stream",
			act: (session) => {
				session.receivePeerUniStreamData(7, hex("05 02"), false);
			},
		},
		{
			name: "a subgroup stream that has ended",
			act: (session) => {
				session.receivePeerUniStreamData(7, hex("14 09 00 00 00"), true);
			},
		},
		{
			name: "a subgroup stream that was stopped for it before",
			act: (session) => {
				session.receivePeerUniStreamData(7, hex("14 09 00 00 00"), false);
				session.peerUniStreamIdle(7);
			},
		},
	])("$name", ({ act }) => {
		const session = subscribed(true);
		session.fetch({
			range: {
				fetchType: "Standalone",
				...TRACK,
				startLocation: { group: 0n, object: 0n },
				endLocation: { group: 1n, object: 0n },
			},
		});
		session.peerStreamOpened("uni", 7);
		act(session);
		expect(briefly(session.peerUniStreamIdle(7))).toEqual([]);
		// Never told of: no stream 8.
		expect(briefly(session.peerUniStreamIdle(8))).toEqual([]);
		expect(session.closed).toBe(false);
	});
});
