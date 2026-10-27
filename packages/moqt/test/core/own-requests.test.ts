import { describe, expect, test } from "bun:test";
import { createSession } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixtureTable, hex, utf8 } from "../helpers.ts";

const TRACK = { trackNamespace: [utf8("xmoq")], trackName: utf8("a") };

/** What the outputs say of what came, in the words of the table. */
function verdict(outputs: SessionOutput[]): string {
	const closed = outputs.find((output) => output.type === "close-session");
	if (closed?.type === "close-session") {
		return `closed:0x${closed.code.toString(16)}`;
	}
	if (
		outputs.some(
			(output) =>
				output.type === "event" && output.event.type === "request-closed",
		)
	) {
		return "over";
	}
	if (outputs.some((output) => output.type === "stop-sending")) {
		return "stopped";
	}
	return "taken";
}

// fixtures/own-requests/events.tsv, which the Rust session reads as well:
// what comes from the peer for a request of this end, a client, on the
// request's stream or as a fetch stream, and what the session makes of it.
// §10.13: "If End Location is smaller than the Start Location in the
// corresponding FETCH the receiver MUST close the session with a
// PROTOCOL_VIOLATION", an End Location naming the last Object plus one, and
// {Group, 0} the whole Group. §2.5.1 and §2.4.2: a Mandatory Track Property in
// FETCH_OK ends the FETCH, as one in SUBSCRIBE_OK ends the subscription. The
// draft has no case of a fetch stream for no open FETCH (§5.2, §10.12.3): one
// for a request that is over is stopped, as §5.2 has Objects come for a FETCH
// that ended; any other is a violation, as is a FETCH_OK on a stream that
// carries no FETCH.
describe("fixtures/own-requests/events.tsv", () => {
	const rows = fixtureTable("own-requests", "events.tsv");
	test("the table has its 11 rows of 5 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(11).fill(5));
	});
	test.each(rows)(
		"%s: for a %s, on the %s, %s is %s",
		(_name, request, where, bytes, expected) => {
			const session = createSession();
			session.start();
			// Request ID 0 on stream 1, the client's first.
			if (request === "subscribe") {
				session.subscribe(TRACK);
			} else {
				session.fetch({
					range: {
						fetchType: "Standalone",
						...TRACK,
						startLocation: { group: 4n, object: 2n },
						endLocation: { group: 9n, object: 0n },
					},
				});
			}
			if (request === "cancelled-fetch") session.cancelRequest(0n);
			const input = hex((bytes ?? "").match(/../g)?.join(" ") ?? "");
			if (where === "request-stream") {
				expect(verdict(session.receiveRequestStreamData(1, input, false))).toBe(
					expected ?? "",
				);
				return;
			}
			if (where === "two-fetch-streams") {
				session.receivePeerUniStreamData(4, input, false);
			}
			expect(verdict(session.receivePeerUniStreamData(8, input, false))).toBe(
				expected ?? "",
			);
		},
	);
});

// fixtures/own-requests/stream-ends.tsv, which the Rust callers of the
// session read as well, each for the requests of its role: what the peer's
// FIN, reset or STOP_SENDING on the stream of a request of this end does to
// it. §3.3.2 cancels a request by "resetting or sending STOP_SENDING"; §5.1
// has the subscriber end a subscription with STOP_SENDING and the publisher
// with PUBLISH_DONE, and a FIN is no cancellation. A FIN before the answer
// ends a request, since the answer can no longer come; a FIN after
// SUBSCRIBE_OK without PUBLISH_DONE ends the subscription, since PUBLISH_DONE
// travels on that direction; a FIN after FETCH_OK or after the REQUEST_OK of a
// PUBLISH_NAMESPACE ends nothing, the FETCH going on until its fetch stream
// ends (§5.2) and the namespace staying published. A SUBSCRIBE_NAMESPACE ends
// with either (§6.1). A STOP_SENDING ends nothing before the answer, nor on a
// SUBSCRIBE, whose publisher ends it, and ends a PUBLISH, a PUBLISH_NAMESPACE,
// whose subscriber withdraws it. After PUBLISH_DONE, the end of the peer's
// direction is reported, the PUBLISH being over already (§10.11).
describe("fixtures/own-requests/stream-ends.tsv", () => {
	const rows = fixtureTable("own-requests", "stream-ends.tsv");
	test("the table has its 28 rows of 5 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(28).fill(5));
	});
	const ANSWERS: Record<string, string> = {
		subscribe: "04 00 02 09 00",
		fetch: "18 00 04 00 04 02 00",
		publish: "07 00 01 00",
		"publish-namespace": "07 00 01 00",
		"subscribe-namespace": "07 00 01 00",
	};
	test.each(rows)(
		"%s: a %s, %s, gets a %s: %s",
		(_name, request, state, event, expected) => {
			const session = createSession();
			session.start();
			// Request ID 0 on stream 1, the client's first.
			switch (request) {
				case "subscribe":
					session.subscribe(TRACK);
					break;
				case "fetch":
					session.fetch({
						range: {
							fetchType: "Standalone",
							...TRACK,
							startLocation: { group: 4n, object: 2n },
							endLocation: { group: 9n, object: 0n },
						},
					});
					break;
				case "publish":
					session.publish({ ...TRACK, trackAlias: 1n });
					break;
				case "publish-namespace":
					session.publishNamespace({ trackNamespace: TRACK.trackNamespace });
					break;
				case "subscribe-namespace":
					session.subscribeNamespace({
						trackNamespacePrefix: TRACK.trackNamespace,
					});
					break;
				default:
					throw new Error(`no request ${request}`);
			}
			if (state !== "pending") {
				session.receiveRequestStreamData(
					1,
					hex(ANSWERS[request ?? ""] ?? ""),
					false,
				);
			}
			if (state === "done" && request === "subscribe") {
				// PUBLISH_DONE, Stream Count 1: the subscription drains.
				session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
			}
			if (state === "done" && request === "publish") {
				session.publishDone(0n, 2n, "");
			}
			const outputs =
				event === "fin"
					? session.receiveRequestStreamData(1, new Uint8Array(0), true)
					: event === "reset"
						? session.requestStreamReset(1, 1)
						: session.localStreamStopped(1, 1);
			const said = (type: string): boolean =>
				outputs.some(
					(output) => output.type === "event" && output.event.type === type,
				);
			expect(
				said("request-closed")
					? "ends"
					: said("peer-direction-ended")
						? "reported"
						: "nothing",
			).toBe(expected ?? "");
		},
	);
});
