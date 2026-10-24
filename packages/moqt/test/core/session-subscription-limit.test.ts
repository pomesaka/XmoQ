import { describe, expect, test } from "bun:test";
import type { SessionLimits } from "../../src/core/limits.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex, isStreamTiming, utf8 } from "../helpers.ts";

const NAMESPACE = [utf8("xmoq"), utf8("phase0")];

/** The connection's limit is 16; held Objects have room for a few. */
const LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16,
	maxObjectPropertiesLength: 16,
	maxPeerRequests: 64,
	maxPeerFetches: 16,
	maxHeldSubgroupBytes: 1024,
};

/** SUBGROUP_HEADER with Subgroup ID 0 and no properties, Group 0, for this Track Alias. */
function header(trackAlias: number): string {
	return `14 ${trackAlias.toString(16).padStart(2, "0")} 00 00 00`;
}

/** The next Object of a stream: Object ID delta 0 and a payload of `length` bytes. */
function object(length: number): string {
	const payload = Array.from({ length }, () => "61").join(" ");
	return `00 ${length.toString(16).padStart(2, "0")} ${payload}`;
}

/** What an output is, in a line, for the outputs a test looks at. */
function summary(output: SessionOutput): string {
	switch (output.type) {
		case "stop-sending":
			return `stop-sending ${output.peerStream} with ${output.code}`;
		case "event":
			switch (output.event.type) {
				case "object":
					return `Object of request ${output.event.requestId}, ${output.event.object.payload.length} bytes`;
				case "subgroup-abandoned":
					return `abandoned on alias ${output.event.trackAlias}: ${output.event.reason}`;
				case "subscribe-ok":
					return `SUBSCRIBE_OK of request ${output.event.requestId}`;
				default:
					return output.event.type;
			}
		default:
			return output.type;
	}
}

function summaries(outputs: SessionOutput[]): string[] {
	return outputs.filter((output) => !isStreamTiming(output)).map(summary);
}

/**
 * A session with two Established subscriptions: Request ID 0 with Track
 * Alias 9 and the limit asked for, and Request ID 2 with Track Alias 10 and
 * none.
 */
function subscribedTwice(maxObjectPayloadLength: number | undefined): Session {
	const session = createSession({ limits: LIMITS });
	session.start();
	session.subscribe({
		trackNamespace: NAMESPACE,
		trackName: utf8("small"),
		...(maxObjectPayloadLength === undefined ? {} : { maxObjectPayloadLength }),
	});
	session.subscribe({ trackNamespace: NAMESPACE, trackName: utf8("other") });
	session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false);
	session.receiveRequestStreamData(2, hex("04 00 02 0a 00"), false);
	return session;
}

describe("a limit on the payload of the Objects of one subscription", () => {
	// The refusal is the one of the connection's limit: the stream is
	// stopped with INTERNAL_ERROR (0x0), and the subscription is not ended.
	test.each<{
		name: string;
		option: number | undefined;
		length: number;
		expected: string[];
	}>([
		{
			name: "an Object at the subscription's limit is delivered",
			option: 4,
			length: 4,
			expected: ["Object of request 0, 4 bytes"],
		},
		{
			name: "an Object one byte over the subscription's limit ends its stream",
			option: 4,
			length: 5,
			expected: [
				"stop-sending 0 with 0",
				"abandoned on alias 9: Object Payload Length 5 exceeds the limit 4",
			],
		},
		{
			name: "with no limit asked for, an Object at the connection's limit is delivered",
			option: undefined,
			length: 16,
			expected: ["Object of request 0, 16 bytes"],
		},
		{
			name: "with no limit asked for, an Object one byte over the connection's limit ends its stream",
			option: undefined,
			length: 17,
			expected: [
				"stop-sending 0 with 0",
				"abandoned on alias 9: Object Payload Length 17 exceeds the limit 16",
			],
		},
		{
			name: "a limit above the connection's is the connection's",
			option: 100,
			length: 17,
			expected: [
				"stop-sending 0 with 0",
				"abandoned on alias 9: Object Payload Length 17 exceeds the limit 16",
			],
		},
	])(
		"$name, and the session and both subscriptions go on",
		({ option, length, expected }) => {
			const session = subscribedTwice(option);
			expect(
				summaries(
					session.receivePeerUniStreamData(
						0,
						hex(`${header(9)} ${object(length)}`),
						false,
					),
				),
			).toEqual(expected);
			expect(session.closed).toBe(false);
			// The other subscription takes the connection's limit.
			expect(
				summaries(
					session.receivePeerUniStreamData(
						1,
						hex(`${header(10)} ${object(16)}`),
						true,
					),
				),
			).toEqual(["Object of request 2, 16 bytes"]);
			// A later stream of the same subscription is read.
			expect(
				summaries(
					session.receivePeerUniStreamData(
						2,
						hex(`${header(9)} ${object(1)}`),
						true,
					),
				),
			).toEqual(["Object of request 0, 1 bytes"]);
		},
	);

	test("an Object read before the SUBSCRIBE_OK is under the connection's limit, and the next one of its stream under the subscription's", () => {
		const session = createSession({ limits: LIMITS });
		session.start();
		session.subscribe({
			trackNamespace: NAMESPACE,
			trackName: utf8("small"),
			maxObjectPayloadLength: 4,
		});
		// Held: the Track Alias is not known yet.
		expect(
			summaries(
				session.receivePeerUniStreamData(
					0,
					hex(`${header(9)} ${object(5)}`),
					false,
				),
			),
		).toEqual([]);
		expect(
			summaries(
				session.receiveRequestStreamData(1, hex("04 00 02 09 00"), false),
			),
		).toEqual(["SUBSCRIBE_OK of request 0", "Object of request 0, 5 bytes"]);
		expect(
			summaries(session.receivePeerUniStreamData(0, hex(object(5)), false)),
		).toEqual([
			"stop-sending 0 with 0",
			"abandoned on alias 9: Object Payload Length 5 exceeds the limit 4",
		]);
	});

	test.each([0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY])(
		"a limit of %p is refused with a RangeError, and no request is made",
		(maxObjectPayloadLength) => {
			const session = createSession({ limits: LIMITS });
			session.start();
			expect(() =>
				session.subscribe({
					trackNamespace: NAMESPACE,
					trackName: utf8("small"),
					maxObjectPayloadLength,
				}),
			).toThrow(RangeError);
			// Request ID 0 was not used.
			expect(
				session.subscribe({
					trackNamespace: NAMESPACE,
					trackName: utf8("small"),
				}).value,
			).toBe(0n);
		},
	);
});
