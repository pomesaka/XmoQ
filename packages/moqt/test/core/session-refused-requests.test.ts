import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import {
	BY_HAND_CONTROL_MESSAGES,
	byHand,
	fixture,
	hex,
	utf8,
} from "../helpers.ts";

// What still comes on the stream of a request of the peer that was refused
// (docs/moqt-implementation-notes.md, the row of that name). The request is
// kept until its stream is gone, whoever refused it: a REQUEST_UPDATE there
// uses up its Request ID (§10.1) and gets no answer.

const TRACK = {
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
};

/** The captured SUBSCRIBE with a Request ID of the server, for "object" or for a track with another last letter. */
function subscribeBytes(requestId: number, lastLetter = "74"): Uint8Array {
	const pairs = fixture("subscribe-moqtail-rs").split(" ");
	pairs[3] = requestId.toString(16).padStart(2, "0");
	pairs[23] = lastLetter;
	return hex(pairs.join(" "));
}

/** The captured Standalone FETCH with a Request ID of the server and a range. */
function fetchBytes(
	requestId: number,
	start: [number, number],
	end: [number, number],
): Uint8Array {
	const pairs = fixture("fetch-standalone-moqtail-rs").split(" ");
	const byte = (value: number): string => value.toString(16).padStart(2, "0");
	pairs[3] = byte(requestId);
	pairs.splice(25, 4, ...[...start, ...end].map(byte));
	return hex(pairs.join(" "));
}

/** REQUEST_UPDATE with a Request ID and no parameters, written by hand: no capture of one exists. */
const update = (requestId: number): Uint8Array =>
	hex(`02 00 02 ${requestId.toString(16).padStart(2, "0")} 00`);

/** The start of a TRACK_STATUS with a Request ID: a request that is not implemented, whole. */
const trackStatus = (requestId: number): Uint8Array =>
	hex(`0d 00 02 ${requestId.toString(16).padStart(2, "0")} 00`);

function started(maxPeerRequests?: number): Session {
	const session = createSession({
		limits: maxPeerRequests === undefined ? {} : { maxPeerRequests },
	});
	session.start();
	return session;
}

/** What the outputs say to the peer, in a line each; nothing else. */
function said(outputs: SessionOutput[]): string[] {
	return outputs.flatMap((output) => {
		if (output.type === "close-session") return [`close ${output.code}`];
		if (output.type !== "respond" && output.type !== "send") return [];
		const decoded = decodeControlMessage(output.data);
		if (decoded.status !== "ok") return ["?"];
		return decoded.value.type === "REQUEST_ERROR"
			? [`REQUEST_ERROR ${decoded.value.errorCode}`]
			: [decoded.value.type];
	});
}

/**
 * The ways a request with the Request ID 1 is refused, on the peer's stream
 * 0. Each leaves the session with that one request kept.
 */
const REFUSED: {
	name: string;
	/** What this endpoint does first, which the refusal rests on. */
	setup?: (session: Session) => void;
	refuse: (session: Session) => void;
}[] = [
	{
		name: "a SUBSCRIBE the application rejects",
		refuse: (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
			session.rejectPeerRequest(1n, 0x10n, "");
		},
	},
	{
		name: "a FETCH the application rejects",
		refuse: (session) => {
			session.receivePeerBidiStreamData(
				0,
				fetchBytes(1, [0, 0], [1, 0]),
				false,
			);
			session.rejectPeerRequest(1n, 0x10n, "");
		},
	},
	{
		name: "a SUBSCRIBE the session refuses (DUPLICATE_SUBSCRIPTION)",
		setup: (session) => {
			session.publish({ ...TRACK, trackAlias: 1n });
		},
		refuse: (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
		},
	},
	{
		name: "a FETCH the session refuses (INVALID_RANGE)",
		refuse: (session) => {
			session.receivePeerBidiStreamData(
				0,
				fetchBytes(1, [3, 4], [3, 3]),
				false,
			);
		},
	},
	{
		name: "a request the session does not implement (NOT_SUPPORTED)",
		refuse: (session) => {
			session.receivePeerBidiStreamData(0, trackStatus(1), false);
		},
	},
	{
		name: "a SUBSCRIBE that a REQUEST_UPDATE before its answer ended",
		refuse: (session) => {
			session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
			// Request ID 0x3f, which no row below uses.
			session.receivePeerBidiStreamData(0, update(0x3f), false);
		},
	},
];

describe("a request of the peer that was refused is kept until its stream is gone", () => {
	test.each(REFUSED)(
		"$name: a REQUEST_UPDATE on its stream gets no answer and uses up its Request ID",
		({ setup, refuse }) => {
			const session = started();
			setup?.(session);
			refuse(session);
			expect(
				said(session.receivePeerBidiStreamData(0, update(3), false)),
			).toEqual([]);
			// The next request cannot have the Request ID 3.
			expect(
				said(session.receivePeerBidiStreamData(1, trackStatus(3), false)),
			).toEqual(["close 4"]);
		},
	);

	test.each(REFUSED)(
		"$name: any other message on its stream is ignored",
		({ setup, refuse }) => {
			const session = started();
			setup?.(session);
			refuse(session);
			expect(
				said(
					session.receivePeerBidiStreamData(
						0,
						hex(fixture("publish-done-moqtail-relay")),
						false,
					),
				),
			).toEqual([]);
			expect(session.closed).toBe(false);
		},
	);

	// `bytes` come on the stream of the refused request; `code` is the
	// Session Termination Error Code.
	test.each(
		REFUSED.flatMap(({ name, setup, refuse }) => [
			{
				name,
				setup,
				refuse,
				what: "a REQUEST_UPDATE with a Request ID of the wrong parity",
				bytes: "02 00 02 04 00",
				fin: false,
				code: 4,
			},
			{
				name,
				setup,
				refuse,
				what: "a REQUEST_UPDATE with the Request ID of the request itself",
				bytes: "02 00 02 01 00",
				fin: false,
				code: 4,
			},
			{
				name,
				setup,
				refuse,
				what: "a message type that is none",
				bytes: "3f 00 00",
				fin: false,
				code: 3,
			},
			{
				name,
				setup,
				refuse,
				what: "a FIN inside a message",
				bytes: "02 00 02",
				fin: true,
				code: 3,
			},
		]),
	)(
		"$name: $what closes the session",
		({ setup, refuse, bytes, fin, code }) => {
			const session = started();
			setup?.(session);
			refuse(session);
			expect(
				said(session.receivePeerBidiStreamData(0, hex(bytes), fin)),
			).toEqual([`close ${code}`]);
		},
	);

	test.each(REFUSED)(
		"$name: nothing of it is kept once the peer has ended its direction",
		({ setup, refuse }) => {
			const session = started();
			setup?.(session);
			const before = session.heldStreams();
			refuse(session);
			expect(session.heldStreams()).toBeGreaterThan(before);
			session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
			expect(session.heldStreams()).toBe(before);
		},
	);

	test("a REQUEST_UPDATE that follows a request the session refuses in one read uses up its Request ID as well", () => {
		const session = started();
		session.receivePeerBidiStreamData(
			0,
			new Uint8Array([...trackStatus(1), ...update(3)]),
			false,
		);
		expect(
			said(session.receivePeerBidiStreamData(1, trackStatus(3), false)),
		).toEqual(["close 4"]);
	});

	test("a request that was refused on its start has the rest of its first message skipped, and what follows read", () => {
		const session = started();
		// A TRACK_STATUS of 6 bytes of payload, of which the Request ID has
		// come: it is refused.
		expect(
			said(session.receivePeerBidiStreamData(0, hex("0d 00 06 01"), false)),
		).toEqual(["REQUEST_ERROR 3"]);
		// The other 5 bytes, which are not looked at, and a REQUEST_UPDATE.
		session.receivePeerBidiStreamData(
			0,
			new Uint8Array([0xff, 0xff, 0xff, 0xff, 0xff, ...update(3)]),
			false,
		);
		expect(session.closed).toBe(false);
		expect(
			said(session.receivePeerBidiStreamData(1, trackStatus(3), false)),
		).toEqual(["close 4"]);
	});
});

describe("a request of the peer that was refused counts among the ones the peer may have open", () => {
	test("a rejected request holds its place until its stream is gone", () => {
		const session = started(2);
		session.receivePeerBidiStreamData(0, trackStatus(1), false);
		session.receivePeerBidiStreamData(1, trackStatus(3), false);
		// Two are kept: a SUBSCRIBE finds no room.
		expect(
			said(session.receivePeerBidiStreamData(2, subscribeBytes(5), false)),
		).toEqual(["REQUEST_ERROR 9"]);
		session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
		expect(
			said(session.receivePeerBidiStreamData(3, subscribeBytes(7), false)),
		).toEqual([]);
	});

	test("a subscription that ended with PUBLISH_DONE holds its place until the peer has ended its direction", () => {
		const session = started(1);
		session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
		session.acceptSubscribe(1n, {});
		session.publishDone(1n, 2n, "");
		expect(
			said(
				session.receivePeerBidiStreamData(1, subscribeBytes(3, "75"), false),
			),
		).toEqual(["REQUEST_ERROR 9"]);
		session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
		expect(
			said(
				session.receivePeerBidiStreamData(2, subscribeBytes(5, "75"), false),
			),
		).toEqual([]);
	});

	test("a FETCH refused because as many FETCHes are open as may be is kept like any other, while there is room among the requests", () => {
		const session = createSession({ limits: { maxPeerFetches: 1 } });
		session.start();
		session.receivePeerBidiStreamData(0, fetchBytes(1, [0, 0], [1, 0]), false);
		expect(
			said(
				session.receivePeerBidiStreamData(
					1,
					fetchBytes(3, [0, 0], [1, 0]),
					false,
				),
			),
		).toEqual(["REQUEST_ERROR 9"]);
		session.receivePeerBidiStreamData(1, update(5), false);
		expect(
			said(session.receivePeerBidiStreamData(2, trackStatus(5), false)),
		).toEqual(["close 4"]);
	});

	// The one request that is not kept: as many are kept as may be.
	test("a request refused while that many are kept is not kept: what follows is dropped, and no Request ID is used", () => {
		const session = started(1);
		session.receivePeerBidiStreamData(0, trackStatus(1), false);
		// EXCESSIVE_LOAD for a SUBSCRIBE, NOT_SUPPORTED for a TRACK_STATUS:
		// neither is kept.
		expect(
			said(session.receivePeerBidiStreamData(1, subscribeBytes(3), false)),
		).toEqual(["REQUEST_ERROR 9"]);
		expect(
			said(session.receivePeerBidiStreamData(2, trackStatus(5), false)),
		).toEqual(["REQUEST_ERROR 3"]);
		for (const peerStream of [1, 2]) {
			// A REQUEST_UPDATE, and bytes that do not decode: dropped.
			session.receivePeerBidiStreamData(peerStream, update(0x21), false);
			session.receivePeerBidiStreamData(peerStream, hex("3f 00 00"), false);
		}
		expect(session.closed).toBe(false);
		// The Request ID 0x21 was not used.
		session.receivePeerBidiStreamData(0, new Uint8Array(0), true);
		expect(
			said(session.receivePeerBidiStreamData(3, trackStatus(0x21), false)),
		).toEqual(["REQUEST_ERROR 3"]);
	});
});

describe("the Request ID of a request that is kept is a duplicate, also when it is no longer remembered among the IDs", () => {
	/** 1,100 requests above a gap, each whole and ended: more than the 1,024 IDs that are remembered. */
	const pushOut = (session: Session, from: number): void => {
		for (let index = 0; index < 1100; index++) {
			const requestId = (from + index * 2).toString(16).padStart(16, "0");
			session.receivePeerBidiStreamData(
				100 + index,
				hex(`0d 00 09 ff ${requestId.match(/../g)?.join(" ")}`),
				true,
			);
		}
	};
	test.each([
		{
			name: "a subscription that ended with PUBLISH_DONE, the peer's direction still open",
			keep: (session: Session) => {
				session.receivePeerBidiStreamData(0, subscribeBytes(3), false);
				session.acceptSubscribe(3n, {});
				session.publishDone(3n, 2n, "");
			},
		},
		{
			name: "a request the session refused, its stream still open",
			keep: (session: Session) => {
				session.receivePeerBidiStreamData(0, trackStatus(3), false);
			},
		},
	])("$name", ({ keep }) => {
		const session = started();
		// Request ID 1 never comes, so 3 stays remembered above the gap.
		keep(session);
		pushOut(session, 5);
		expect(session.closed).toBe(false);
		expect(
			said(session.receivePeerBidiStreamData(5000, trackStatus(3), false)),
		).toEqual(["close 4"]);
	});
});

describe("a request that is kept is not one that is yet to come, also when its ID is no longer remembered", () => {
	/** A Request ID as a varint of 9 bytes. */
	const nine = (id: number): string[] => [
		"ff",
		...(id.toString(16).padStart(16, "0").match(/../g) ?? []),
	];
	test("a Joining FETCH that names a Standalone FETCH that is open and was pushed out gets INVALID_JOINING_REQUEST_ID", () => {
		const session = started();
		// Request ID 1 never comes, so 3 stays above the gap. The FETCH is
		// handed on and not answered: it is open.
		session.receivePeerBidiStreamData(0, fetchBytes(3, [0, 0], [1, 0]), false);
		// 5 to 2203: more than the 1,024 IDs that are remembered.
		for (let id = 5; id <= 2203; id += 2) {
			session.receivePeerBidiStreamData(
				id,
				hex(["0d", "00", "09", ...nine(id)].join(" ")),
				true,
			);
		}
		expect(session.closed).toBe(false);
		// The captured Relative Joining FETCH, with the Request ID 2205 in 9
		// bytes (the Message Length grows by 8) and 3 as the one it names.
		const pairs = fixture("fetch-relative-joining-moqtail-rs").split(" ");
		const length = Number.parseInt(`${pairs[1]}${pairs[2]}`, 16) + 8;
		pairs[5] = "03";
		pairs.splice(3, 1, ...nine(2205));
		pairs.splice(
			1,
			2,
			...(length.toString(16).padStart(4, "0").match(/../g) ?? []),
		);
		expect(
			said(
				session.receivePeerBidiStreamData(5000, hex(pairs.join(" ")), false),
			),
		).toEqual(["REQUEST_ERROR 50"]);
	});
});

describe("fixtures/moqt-18-by-hand: the REQUEST_UPDATE messages written by hand", () => {
	test("the two files are the ones the tests read", () => {
		expect([...BY_HAND_CONTROL_MESSAGES]).toEqual([
			"request-update-request-id-3",
			"request-update-request-id-5",
		]);
	});

	test.each([
		{ file: "request-update-request-id-3", requestId: 3n },
		{ file: "request-update-request-id-5", requestId: 5n },
	])(
		"$file is a REQUEST_UPDATE with that Request ID and no parameters",
		({ file, requestId }) => {
			const bytes = hex(byHand(file));
			expect(decodeControlMessage(bytes)).toEqual({
				status: "ok",
				value: { type: "REQUEST_UPDATE", requestId, parameters: {} },
				bytesRead: bytes.length,
			});
		},
	);
});

// How the bytes of a stream are cut into reads is not the peer's doing: a
// request, what follows it and the FIN give the same whether they come in
// one read or in three.
describe("a refused request, what follows it and the FIN: in one read as in several", () => {
	const AFTER = [
		{ what: "a REQUEST_UPDATE", bytes: "02 00 02 03 00", closes: [] },
		{
			what: "a REQUEST_UPDATE with a Request ID of the wrong parity",
			bytes: "02 00 02 04 00",
			closes: ["close 4"],
		},
		{
			what: "a REQUEST_UPDATE with the Request ID of the request",
			bytes: "02 00 02 01 00",
			closes: ["close 4"],
		},
		{
			what: "a message type that is none",
			bytes: "3f 00 00",
			closes: ["close 3"],
		},
		{ what: "the start of a message", bytes: "02 00 02", closes: ["close 3"] },
		{
			what: "another message",
			bytes: fixture("publish-done-moqtail-relay"),
			closes: [],
		},
	];
	// The requests the session refuses by itself on the bytes of the
	// request alone. `first` is what the request is answered with.
	const REQUESTS = [
		{
			name: "a request that is not implemented",
			setup: (_session: Session) => {},
			request: trackStatus(1),
			first: "REQUEST_ERROR 3",
		},
		{
			name: "a SUBSCRIBE to a Track this endpoint publishes",
			setup: (session: Session) => {
				session.publish({ ...TRACK, trackAlias: 1n });
			},
			request: subscribeBytes(1),
			first: "REQUEST_ERROR 25",
		},
		{
			name: "a FETCH whose range ends before it starts",
			setup: (_session: Session) => {},
			request: fetchBytes(1, [3, 4], [3, 3]),
			first: "REQUEST_ERROR 17",
		},
	];
	const rows = REQUESTS.flatMap((request) =>
		AFTER.map((after) => ({ ...request, ...after })),
	);

	test.each(rows)(
		"$name, then $what",
		({ setup, request, first, bytes, closes }) => {
			const after = hex(bytes);
			const inOne = started();
			setup(inOne);
			const one = said(
				inOne.receivePeerBidiStreamData(
					0,
					new Uint8Array([...request, ...after]),
					true,
				),
			);
			const inThree = started();
			setup(inThree);
			const three = [
				...said(inThree.receivePeerBidiStreamData(0, request, false)),
				...said(inThree.receivePeerBidiStreamData(0, after, false)),
				...said(inThree.receivePeerBidiStreamData(0, new Uint8Array(0), true)),
			];
			expect(one).toEqual([first, ...closes]);
			expect(three).toEqual([first, ...closes]);
		},
	);

	test("the Request ID of a REQUEST_UPDATE that came with the request and the FIN is used up", () => {
		const session = started();
		session.receivePeerBidiStreamData(
			0,
			new Uint8Array([...trackStatus(1), ...update(3)]),
			true,
		);
		expect(
			said(session.receivePeerBidiStreamData(1, trackStatus(3), false)),
		).toEqual(["close 4"]);
	});
});

describe("a FETCH of the peer counts among the requests the peer may have", () => {
	test("a FETCH that comes while as many requests are kept as may be gets EXCESSIVE_LOAD, also with room among the FETCHes", () => {
		const session = createSession({
			limits: { maxPeerRequests: 1, maxPeerFetches: 8 },
		});
		session.start();
		session.receivePeerBidiStreamData(0, subscribeBytes(1), false);
		expect(
			said(
				session.receivePeerBidiStreamData(
					1,
					fetchBytes(3, [0, 0], [1, 0]),
					false,
				),
			),
		).toEqual(["REQUEST_ERROR 9"]);
	});
});
