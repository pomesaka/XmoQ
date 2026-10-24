import { describe, expect, test } from "bun:test";
import { createIncomingSubgroups } from "../../src/core/incoming-subgroups.ts";
import { resolveLimits } from "../../src/core/limits.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type {
	PeerStreamKind,
	SessionOutput,
} from "../../src/core/session-output.ts";
import {
	firstBytesFixtures,
	hex,
	MOQTAIL_RELAY_SETUP,
	utf8,
} from "../helpers.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

/** A session with a FETCH that has Request ID 0. */
function started(): Session {
	const session = createSession();
	session.start();
	session.fetch({
		range: {
			fetchType: "Standalone",
			...TRACK,
			startLocation: { group: 0n, object: 0n },
			endLocation: { group: 1n, object: 0n },
		},
	});
	return session;
}

/** What the I/O layer is told to do about the time a stream has, and to the stream. */
function brief(output: SessionOutput): string | undefined {
	switch (output.type) {
		case "peer-stream-identified":
			return `identified ${output.kind} ${output.peerStream}`;
		case "stop-sending":
			return `stop uni ${output.peerStream} code ${output.code}`;
		case "abandon-peer-request":
			return `abandon bidi ${output.peerStream} code ${output.code}`;
		case "close-session":
			return `close ${output.code}`;
		case "event":
			return output.event.type === "peer-stream-timed-out"
				? `timed out ${output.event.kind} ${output.event.peerStream}`
				: undefined;
		default:
			return undefined;
	}
}

const briefly = (outputs: SessionOutput[]): string[] =>
	outputs.flatMap((output) => brief(output) ?? []);

/** Feeds the chunks to one stream of the peer, number 7, and says what each led to. */
function feed(
	session: Session,
	kind: PeerStreamKind,
	chunks: string[],
): string[][] {
	return chunks.map((chunk) =>
		briefly(
			kind === "uni"
				? session.receivePeerUniStreamData(7, hex(chunk), false)
				: session.receivePeerBidiStreamData(7, hex(chunk), false),
		),
	);
}

// The table both implementations read (fixtures/first-bytes/README.md): what
// a stream has sent when its time runs out, and whether it is stopped then.
// Each row is run with its bytes in one read and a byte at a time.
describe("fixtures/first-bytes: whether a stream has said what it is when its time runs out", () => {
	const STOPPED: Record<PeerStreamKind, string[]> = {
		uni: ["stop uni 7 code 0", "timed out uni 7"],
		bidi: ["abandon bidi 7 code 1", "timed out bidi 7"],
	};
	test.each(firstBytesFixtures())(
		"$kind $name: $said",
		({ kind, sentHex, said }) => {
			const bytes = hex(sentHex);
			const readings = [
				bytes.length === 0 ? [] : [sentHex],
				Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")),
			];
			for (const chunks of readings) {
				const session = started();
				session.peerStreamOpened(kind, 7);
				feed(session, kind, chunks);
				expect(briefly(session.peerStreamTimedOut(kind, 7))).toEqual(
					said ? [] : STOPPED[kind],
				);
				expect(session.closed).toBe(false);
			}
		},
	);
});

// When the output for it comes: a unidirectional stream has said what it is
// with its type, and with its whole header where it has one; a bidirectional
// one with its first message, whole.
describe("a stream of the peer has said what it is", () => {
	test.each<{
		name: string;
		kind: PeerStreamKind;
		chunks: string[];
		expected: string[][];
	}>([
		{
			name: "the control stream, with its type",
			kind: "uni",
			// 0x2F00 in two bytes, and the rest of SETUP later.
			chunks: ["af 00", "00 00"],
			expected: [["identified uni 7"], []],
		},
		{
			name: "a padding stream, with its type (§11.5.1)",
			kind: "uni",
			// 0x132B3E28, which takes five bytes (§1.4.1).
			chunks: ["f0 13 2b 3e", "28", "00 00"],
			expected: [[], ["identified uni 7"], []],
		},
		{
			name: "a subgroup stream, with its whole header and not with its type",
			kind: "uni",
			// Type 0x14: Track Alias 9, Group 0, Subgroup 0, priority 0.
			chunks: ["14", "09 00", "00 00", "00 01 61"],
			expected: [[], [], ["identified uni 7", "stop uni 7 code 1"], []],
		},
		{
			name: "a fetch stream, with its whole header",
			kind: "uni",
			// The Request ID 0 in two bytes (§1.4.1).
			chunks: ["05", "80", "00", "aa"],
			expected: [[], [], ["identified uni 7"], []],
		},
		{
			name: "a request that is refused on its Request ID, when the message is whole",
			kind: "bidi",
			// A SUBSCRIBE_TRACKS of 5 bytes of payload, Request ID 1.
			chunks: ["51 00 05 01", "00 00", "00 00"],
			expected: [[], [], ["identified bidi 7"]],
		},
		{
			name: "a request that comes whole in one read",
			kind: "bidi",
			chunks: ["51 00 02 01 00"],
			expected: [["identified bidi 7"]],
		},
		{
			name: "a FETCH, which is decoded whole",
			kind: "bidi",
			// Relative Joining, Request ID 1.
			chunks: ["16 00 05 01 02", "00 00", "00"],
			expected: [[], [], ["identified bidi 7"]],
		},
		{
			name: "what follows the first message says nothing more",
			kind: "bidi",
			chunks: ["51 00 02 01 00", "aa bb"],
			expected: [["identified bidi 7"], []],
		},
	])("$name", ({ kind, chunks, expected }) => {
		expect(feed(started(), kind, chunks)).toEqual(expected);
	});
});

describe("the time a stream of the peer has to say what it is runs out", () => {
	// `before` is what the stream had sent by then.
	test.each<{
		name: string;
		kind: PeerStreamKind;
		before: string[];
		expected: string[];
	}>([
		{
			name: "a unidirectional stream that sent nothing",
			kind: "uni",
			before: [],
			expected: ["stop uni 7 code 0", "timed out uni 7"],
		},
		{
			name: "one that sent half of its type",
			kind: "uni",
			before: ["80"],
			expected: ["stop uni 7 code 0", "timed out uni 7"],
		},
		{
			name: "a subgroup stream that sent its type and half of its header",
			kind: "uni",
			before: ["14 09"],
			expected: ["stop uni 7 code 0", "timed out uni 7"],
		},
		{
			name: "a fetch stream that sent its type",
			kind: "uni",
			before: ["05"],
			expected: ["stop uni 7 code 0", "timed out uni 7"],
		},
		{
			name: "a bidirectional stream that sent nothing",
			kind: "bidi",
			before: [],
			expected: ["abandon bidi 7 code 1", "timed out bidi 7"],
		},
		{
			name: "one that sent half of its Request ID",
			kind: "bidi",
			before: ["51 00 05 80"],
			expected: ["abandon bidi 7 code 1", "timed out bidi 7"],
		},
		{
			name: "a request that was refused on its Request ID and is not whole",
			kind: "bidi",
			before: ["51 00 05 01 00"],
			expected: ["abandon bidi 7 code 1", "timed out bidi 7"],
		},
		{
			name: "a FETCH that is not whole",
			kind: "bidi",
			before: ["16 00 05 01 02"],
			expected: ["abandon bidi 7 code 1", "timed out bidi 7"],
		},
	])(
		"$name is stopped, and the session stays",
		({ kind, before, expected }) => {
			const session = started();
			// The I/O layer says that the stream is there before any byte of it.
			session.peerStreamOpened(kind, 7);
			feed(session, kind, before);
			expect(briefly(session.peerStreamTimedOut(kind, 7))).toEqual(expected);
			expect(session.closed).toBe(false);
			// The end that follows the stop is no end inside a type or a
			// message: the stream was given up, not cut off by the peer.
			expect(
				briefly(
					kind === "uni"
						? session.receivePeerUniStreamData(7, hex(""), true)
						: session.receivePeerBidiStreamData(7, hex(""), true),
				),
			).toEqual([]);
			expect(session.closed).toBe(false);
		},
	);

	// What the peer sent before it saw the stop can still arrive, and is no
	// fault of the peer: it is dropped. `rest` is what the stream would have
	// gone on with; read as a stream or a request from its start, each of
	// them would close the session.
	test.each<{
		name: string;
		kind: PeerStreamKind;
		before: string;
		rest: string[];
	}>([
		{
			name: "a subgroup stream, the rest of its header and an Object",
			kind: "uni",
			before: "5d 00",
			rest: ["00 00 00", "00 01 61"],
		},
		{
			name: "a stream with half of the control type, the rest of it",
			kind: "uni",
			before: "af",
			rest: ["00", "00 00"],
		},
		{
			name: "a fetch stream, its Request ID",
			kind: "uni",
			before: "05",
			rest: ["00", "aa bb"],
		},
		{
			name: "a request with its type and length, the rest of the message",
			kind: "bidi",
			before: "03 00 1f",
			rest: ["01 02 04 78 6d 6f 71", "06 70 68"],
		},
		{
			name: "a request with its type alone, the rest of the message",
			kind: "bidi",
			before: "03",
			rest: ["00 1f 01 02"],
		},
		{
			name: "a request that was refused on its Request ID, the rest of the message",
			kind: "bidi",
			before: "03 00 1f 01 02",
			rest: ["04 78 6d 6f 71", "06"],
		},
	])(
		"what still arrives on a stream that was stopped is dropped: $name",
		({ kind, before, rest }) => {
			const session = started();
			feed(session, kind, [before]);
			session.peerStreamTimedOut(kind, 7);
			expect(feed(session, kind, rest)).toEqual(rest.map(() => []));
			// Its end, in the middle of what it was sending, ends nothing else.
			expect(
				briefly(
					kind === "uni"
						? session.receivePeerUniStreamData(7, hex(""), true)
						: session.receivePeerBidiStreamData(7, hex(""), true),
				),
			).toEqual([]);
			expect(session.closed).toBe(false);
		},
	);

	// The stream is remembered as stopped only until it ends: then nothing
	// of it is left. Seen by what a stream under the same number is taken
	// for, which a transport does not send; `80` and `03` are the start of a
	// type and of a request, and their end is a violation.
	test.each<{ name: string; kind: PeerStreamKind; start: string; how: string }>(
		[
			{ name: "a FIN", kind: "uni", start: "80", how: "fin" },
			{ name: "a reset", kind: "uni", start: "80", how: "reset" },
			{ name: "a FIN", kind: "bidi", start: "03", how: "fin" },
			{ name: "a reset", kind: "bidi", start: "03", how: "reset" },
		],
	)(
		"a $kind stream that was stopped is forgotten with $name",
		({ kind, start, how }) => {
			const session = started();
			feed(session, kind, [start]);
			session.peerStreamTimedOut(kind, 7);
			if (how === "reset") {
				if (kind === "uni") session.peerUniStreamReset(7);
				else session.peerBidiStreamReset(7);
			} else if (kind === "uni") {
				session.receivePeerUniStreamData(7, hex(""), true);
			} else {
				session.receivePeerBidiStreamData(7, hex(""), true);
			}
			expect(
				briefly(
					kind === "uni"
						? session.receivePeerUniStreamData(7, hex(start), true)
						: session.receivePeerBidiStreamData(7, hex(start), true),
				),
			).toEqual(["close 3"]);
		},
	);

	// The session does not count on when, or how often, its caller reports
	// the time: for a stream it does not know as one that is there and has
	// not said what it is, nothing is done and nothing is reported.
	test.each<{
		name: string;
		kind: PeerStreamKind;
		arrange: (session: Session) => void;
	}>([
		{
			name: "a unidirectional stream the session was never told of",
			kind: "uni",
			arrange: () => {},
		},
		{
			name: "a bidirectional stream the session was never told of",
			kind: "bidi",
			arrange: () => {},
		},
		{
			name: "a unidirectional stream that has ended without a byte",
			kind: "uni",
			arrange: (session) => {
				session.peerStreamOpened("uni", 7);
				session.receivePeerUniStreamData(7, hex(""), true);
			},
		},
		{
			name: "a padding stream that has ended",
			kind: "uni",
			arrange: (session) => {
				session.receivePeerUniStreamData(7, hex("f0 13 2b 3e 28"), true);
			},
		},
		{
			name: "a unidirectional stream that was reset inside its type",
			kind: "uni",
			arrange: (session) => {
				session.receivePeerUniStreamData(7, hex("80"), false);
				session.peerUniStreamReset(7);
			},
		},
		{
			name: "a request that was answered and has ended",
			kind: "bidi",
			arrange: (session) => {
				session.receivePeerBidiStreamData(7, hex("51 00 02 01 00"), true);
			},
		},
		{
			name: "a request that has ended without a byte",
			kind: "bidi",
			arrange: (session) => {
				session.peerStreamOpened("bidi", 7);
				session.receivePeerBidiStreamData(7, hex(""), true);
			},
		},
		{
			name: "a request that was reset inside its first message",
			kind: "bidi",
			arrange: (session) => {
				session.receivePeerBidiStreamData(7, hex("51 00 05 80"), false);
				session.peerBidiStreamReset(7);
			},
		},
		{
			name: "a unidirectional stream that was stopped for its time already",
			kind: "uni",
			arrange: (session) => {
				session.receivePeerUniStreamData(7, hex("14 09"), false);
				session.peerStreamTimedOut("uni", 7);
			},
		},
		{
			name: "a request that was given up for its time already",
			kind: "bidi",
			arrange: (session) => {
				session.receivePeerBidiStreamData(7, hex("51 00 05 80"), false);
				session.peerStreamTimedOut("bidi", 7);
			},
		},
	])("nothing is done for $name", ({ kind, arrange }) => {
		const session = started();
		arrange(session);
		expect(session.peerStreamTimedOut(kind, 7)).toEqual([]);
		expect(session.closed).toBe(false);
	});

	// What the session holds of a subgroup stream is with the part that
	// reads its header. Asking that part drops the stream in the same call,
	// so that a stream that was stopped cannot stay there: the second
	// answer is that there is no such stream.
	test("a subgroup stream that is asked about inside its header is dropped by the question", () => {
		const incoming = createIncomingSubgroups({
			limits: resolveLimits(),
			hasPendingSubscribe: () => true,
			onMalformedTrack: () => {},
			onDrained: () => {},
		});
		incoming.open(7);
		incoming.receive(7, hex("14 09"), false, []);
		expect([
			incoming.dropBeforeHeader(7),
			incoming.dropBeforeHeader(7),
		]).toEqual([true, false]);
		// One with its header is not dropped, and neither is one that is unknown.
		incoming.open(8);
		incoming.receive(8, hex("14 09 00 00 00"), false, []);
		expect([
			incoming.dropBeforeHeader(8),
			incoming.dropBeforeHeader(9),
		]).toEqual([false, false]);
	});

	// The I/O layer can report the time as over for a stream whose last read
	// said what it is, while the output for that is still on its way.
	test.each<{ name: string; kind: PeerStreamKind; before: string[] }>([
		{ name: "the control stream", kind: "uni", before: [MOQTAIL_RELAY_SETUP] },
		{ name: "a padding stream", kind: "uni", before: ["f0 13 2b 3e 28"] },
		{
			name: "a subgroup stream with its header",
			kind: "uni",
			before: ["14 09 00 00 00"],
		},
		{ name: "a fetch stream with its header", kind: "uni", before: ["05 00"] },
		{
			name: "a request that was refused and is whole",
			kind: "bidi",
			before: ["51 00 02 01 00"],
		},
	])("nothing is done to $name", ({ kind, before }) => {
		const session = started();
		feed(session, kind, before);
		expect(session.peerStreamTimedOut(kind, 7)).toEqual([]);
	});

	test("a subgroup stream that is stopped before its header does not count for its subscription (§10.11)", () => {
		const session = createSession();
		session.start();
		session.subscribe(TRACK);
		// SUBSCRIBE_OK with Track Alias 9; then a stream that sends its type
		// and the alias, and stops.
		session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
		session.receivePeerUniStreamData(7, hex("14 09"), false);
		expect(briefly(session.peerStreamTimedOut("uni", 7))).toEqual([
			"stop uni 7 code 0",
			"timed out uni 7",
		]);
		// PUBLISH_DONE with Stream Count 1: the subscription still waits for
		// that one stream.
		expect(
			session
				.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false)
				.map((output) =>
					output.type === "event" ? output.event.type : output.type,
				),
		).toEqual(["publish-done"]);
	});
});
