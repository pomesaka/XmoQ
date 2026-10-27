import { describe, expect, test } from "bun:test";
import { decodeControlMessage } from "../../src/core/control-message.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixtureTable, hex, utf8 } from "../helpers.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

/** The SUBSCRIBE of the peer, a server, with Request ID 1, for TRACK. */
const PEER_SUBSCRIBE = hex("03 00 0a 01 01 04 78 6d 6f 71 01 61 00");

/** Opens the request a row names on a client, in the state it names. */
function opened(
	on: string,
	state: string,
): {
	session: Session;
	feed: (bytes: Uint8Array) => SessionOutput[];
} {
	const session = createSession();
	session.start();
	const established = state !== "pending";
	if (on === "peer-subscribe") {
		session.receivePeerBidiStreamData(0, PEER_SUBSCRIBE, false);
		if (state === "established") session.acceptSubscribe(1n, {});
		if (state === "refused") session.rejectPeerRequest(1n, 0x10n, "");
		return {
			session,
			feed: (bytes) => session.receivePeerBidiStreamData(0, bytes, false),
		};
	}
	// This session's one request, Request ID 0 on its stream 1, and the
	// answer of the peer that establishes it.
	const answers: Record<string, () => string> = {
		"own-publish": () => {
			session.publish({ ...TRACK, trackAlias: 1n });
			return "07 00 01 00";
		},
		"own-subscribe": () => {
			session.subscribe(TRACK);
			return "04 00 02 09 00";
		},
		"own-publish-namespace": () => {
			session.publishNamespace({ trackNamespace: TRACK.trackNamespace });
			return "07 00 01 00";
		},
		"own-track-status": () => {
			session.trackStatus(TRACK);
			return "07 00 01 00";
		},
	};
	const answer = answers[on]?.();
	if (answer === undefined) throw new Error(`no request ${on}`);
	if (established) session.receiveRequestStreamData(1, hex(answer), false);
	if (state === "done") session.publishDone(0n, 2n, "");
	return {
		session,
		feed: (bytes) => session.receiveRequestStreamData(1, bytes, false),
	};
}

/** The control messages a row's bytes hold, one after another. */
function messages(bytes: Uint8Array): Uint8Array[] {
	const each: Uint8Array[] = [];
	for (let at = 0; at < bytes.length; ) {
		const decoded = decodeControlMessage(bytes.subarray(at));
		const read =
			decoded.status === "ok" ? decoded.bytesRead : bytes.length - at;
		each.push(bytes.subarray(at, at + read));
		at += read;
	}
	return each;
}

/** What the outputs of the last update say of it, in the words of the table. */
function verdict(on: string, outputs: SessionOutput[]): string {
	const closed = outputs.find((output) => output.type === "close-session");
	if (closed?.type === "close-session") {
		return `closed:0x${closed.code.toString(16)}`;
	}
	const written = outputs.flatMap((output) => {
		if (output.type !== "send" && output.type !== "respond") return [];
		const decoded = decodeControlMessage(output.data);
		return decoded.status === "ok" ? [decoded.value.type] : [];
	});
	if (written.includes("REQUEST_OK")) return "ok";
	if (!written.includes("REQUEST_ERROR")) return "ignored";
	if (on !== "peer-subscribe" || written.includes("PUBLISH_DONE")) {
		return "refused";
	}
	return "refused-and-over";
}

// fixtures/request-update/received.tsv, which the Rust session reads as
// well: a REQUEST_UPDATE that a client receives, by the request it comes on
// and the state of that request, and what the session makes of it. §10.9:
// "The sender of a request ... can later send a REQUEST_UPDATE", and "A
// subscriber can also send REQUEST_UPDATE to modify parameters of a
// subscription established with PUBLISH"; on any other stream it is a
// violation, as before the subscription is established (§5.1) and on a
// TRACK_STATUS (§10.14: "the subscriber cannot send REQUEST_UPDATE"). The
// draft does not say which parameters a publisher applies: the delivery
// timeouts and NEW_GROUP_REQUEST are, and any other has the update refused
// with NOT_SUPPORTED, the subscription then ending with UPDATE_FAILED
// (§10.9.1); later ones get no answer. An update before the answer to a
// SUBSCRIBE of the peer is refused and the request is over. What crosses a
// PUBLISH_DONE is ignored (§5.1, §10.9). A parameter §10.2.1 does not allow
// in an update is a violation first.
describe("fixtures/request-update/received.tsv", () => {
	const rows = fixtureTable("request-update", "received.tsv");
	test("the table has its 22 rows of 5 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(22).fill(5));
	});
	test.each(rows)(
		"%s: on %s, %s, %s is %s",
		(_name, on, state, bytes, expected) => {
			const { feed } = opened(on ?? "", state ?? "");
			const updates = messages(
				hex((bytes ?? "").match(/../g)?.join(" ") ?? ""),
			);
			let last: SessionOutput[] = [];
			for (const update of updates) last = feed(update);
			expect(verdict(on ?? "", last)).toBe(expected ?? "");
		},
	);
});
