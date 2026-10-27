import { describe, expect, test } from "bun:test";
import { createSession } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixtureTable, hex, utf8 } from "../helpers.ts";

/** The SUBSCRIBE of the peer, a server, with this Request ID, for ("xmoq") and a one-letter track. */
function peerSubscribe(requestId: number, trackName: string): Uint8Array {
	const name = trackName.charCodeAt(0).toString(16);
	return hex(
		`03 00 0a ${requestId.toString(16).padStart(2, "0")} 01 04 78 6d 6f 71 01 ${name} 00`,
	);
}

// fixtures/early-objects/aliases.tsv, which the Rust session reads as well:
// steps in a client's session, and then whom a Track Alias names. §11.1:
// "Track Alias is chosen by the publisher and included in SUBSCRIBE_OK", and
// "Objects can arrive after a subscription has been cancelled", so an alias
// this end chose is not given again, and one past an alias this end uses is
// not given either. §10.11: after PUBLISH_DONE the alias stays the
// subscription's until the streams its Stream Count named have ended; then a
// late stream under it is stopped, and more streams than counted are not a
// reason to close the session. A SUBSCRIBE_OK that reuses the alias of a
// draining subscription takes it; one that is refused for a Mandatory Track
// Property (§2.5.1) leaves it to the draining one; one whose alias another
// subscription holds closes the session with DUPLICATE_TRACK_ALIAS before the
// property is looked at.
describe("fixtures/early-objects/aliases.tsv", () => {
	const rows = fixtureTable("early-objects", "aliases.tsv");
	test("the table has its 8 rows of 3 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(8).fill(3));
	});
	test.each(rows)("%s: %s names %s", (_name, steps, expected) => {
		const session = createSession();
		session.start();
		/** The stream and Request ID of each SUBSCRIBE of this end, by track. */
		const own = new Map<string, { stream: number; requestId: bigint }>();
		const given: bigint[] = [];
		let nextStream = 1;
		let nextPeerStream = 0;
		let peerRequestId = 1;
		let uniStream = 3;
		let closed: number | undefined;
		const watch = (outputs: SessionOutput[]): void => {
			for (const output of outputs) {
				if (output.type === "close-session") closed ??= output.code;
			}
		};
		for (const step of (steps ?? "").split(" ")) {
			if (closed !== undefined) break;
			const [kind, track = "", value = ""] = step.split(":");
			const request = own.get(track);
			switch (kind) {
				case "subscribe": {
					const requestId = session.subscribe({
						trackNamespace: [utf8("xmoq")],
						trackName: utf8(track),
					}).value;
					own.set(track, { stream: nextStream++, requestId });
					break;
				}
				case "ok":
				case "ok-mandatory": {
					if (request === undefined) throw new Error(`no ${track}`);
					const alias = Number(value).toString(16).padStart(2, "0");
					watch(
						session.receiveRequestStreamData(
							request.stream,
							hex(
								kind === "ok"
									? `04 00 02 ${alias} 00`
									: `04 00 06 ${alias} 00 c0 40 00 01`,
							),
							false,
						),
					);
					break;
				}
				case "done": {
					if (request === undefined) throw new Error(`no ${track}`);
					const streams = Number(value).toString(16).padStart(2, "0");
					watch(
						session.receiveRequestStreamData(
							request.stream,
							hex(`0b 00 03 02 ${streams} 00`),
							false,
						),
					);
					break;
				}
				case "end": {
					const alias = Number(track).toString(16).padStart(2, "0");
					watch(
						session.receivePeerUniStreamData(
							uniStream,
							hex(`14 ${alias} 00 00 00`),
							true,
						),
					);
					uniStream += 4;
					break;
				}
				case "publish":
					session.publish({
						trackNamespace: [utf8("xmoq")],
						trackName: utf8("own"),
						trackAlias: BigInt(track),
					});
					nextStream++;
					break;
				case "peer-subscribe": {
					const requestId = peerRequestId;
					peerRequestId += 2;
					session.receivePeerBidiStreamData(
						nextPeerStream++,
						peerSubscribe(requestId, track),
						false,
					);
					given.push(session.acceptSubscribe(BigInt(requestId), {}).value);
					break;
				}
				default:
					throw new Error(`a step the test does not know: ${step}`);
			}
		}
		let got: string;
		if (closed !== undefined) {
			got = `closed:0x${closed.toString(16)}`;
		} else if (given.length > 0) {
			got = `given:${given.join(",")}`;
		} else {
			// An Object under alias 9, on a stream of its own.
			const outputs = session.receivePeerUniStreamData(
				uniStream,
				hex("14 09 00 00 00 00 01 61"),
				false,
			);
			const object = outputs.find(
				(output) => output.type === "event" && output.event.type === "object",
			);
			const to =
				object?.type === "event" && object.event.type === "object"
					? [...own].find(([, { requestId }]) =>
							"requestId" in object.event
								? object.event.requestId === requestId
								: false,
						)?.[0]
					: undefined;
			got =
				to !== undefined
					? `object:${to}`
					: outputs.some((output) => output.type === "stop-sending")
						? "stopped"
						: "nothing";
		}
		expect(got).toBe(expected ?? "");
	});
});
