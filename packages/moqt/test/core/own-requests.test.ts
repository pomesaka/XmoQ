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
