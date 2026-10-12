import { expect, test } from "bun:test";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fromALibrary, hex, toHex, utf8 } from "../helpers.ts";

// A subscriber's REQUEST_UPDATE for its own SUBSCRIBE (§10.9), and the
// answers to it. The Rust session has the same rules
// (`a_subscriber_updates_its_subscription_and_reads_the_answers_in_order`).

const TRACK = {
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
};

/** A session whose SUBSCRIBE (Request ID 0, stream 1) has its SUBSCRIBE_OK. */
function subscribed(): Session {
	const session = createSession({});
	session.start();
	session.subscribe(TRACK);
	session.receiveRequestStreamData(1, hex("04 00 02 00 00"), false);
	return session;
}

/** What the session did, in a line each. */
function told(outputs: SessionOutput[]): string[] {
	return outputs.map((output) => {
		if (output.type === "send")
			return `send ${output.stream}: ${toHex(output.data)}`;
		if (output.type !== "event") return output.type;
		const { event } = output;
		if (event.type === "update-ok")
			return `update-ok ${JSON.stringify(Object.keys(event.parameters))}`;
		if (event.type === "update-error")
			return `update-error ${event.errorCode} retry ${event.retryInterval}`;
		return event.type;
	});
}

test("the REQUEST_UPDATE that sets the subgroup timeout is the bytes moqtail's library writes", () => {
	const session = subscribed();
	// §10.1: the update has a Request ID of its own, 2 after the SUBSCRIBE's 0.
	expect(
		told(session.updateSubscription(0n, { subgroupDeliveryTimeout: 250n })),
	).toEqual([
		`send 1: ${fromALibrary("request-update-subgroup-delivery-timeout-moqtail-ts")}`,
	]);
});

test.each<{
	name: string;
	/** Updates sent before the answers, each with these parameters. */
	updates: number;
	/** What comes on the stream of the SUBSCRIBE, and what the session says to each. */
	answers: [string, string[] | "PROTOCOL_VIOLATION"][];
}>([
	{
		name: "a REQUEST_OK answers one update",
		updates: 1,
		answers: [["07 00 01 00", ["update-ok []"]]],
	},
	{
		// EXPIRES (0x08) = 5.
		name: "a REQUEST_OK may carry EXPIRES (§10.2.10)",
		updates: 1,
		answers: [["07 00 03 01 08 05", ['update-ok ["expires"]']]],
	},
	{
		name: "the answers come in the order the updates were sent, one each",
		updates: 2,
		answers: [
			["07 00 01 00", ["update-ok []"]],
			["07 00 01 00", ["update-ok []"]],
			["07 00 01 00", "PROTOCOL_VIOLATION"],
		],
	},
	{
		// REQUEST_ERROR NOT_SUPPORTED (0x3), then PUBLISH_DONE UPDATE_FAILED (0x8).
		name: "a REQUEST_ERROR answers every update that waits, and PUBLISH_DONE ends the subscription (§10.9.1)",
		updates: 2,
		answers: [
			["05 00 03 03 00 00", ["update-error 3 retry 0"]],
			["0b 00 03 08 00 00", ["publish-done", "request-closed", "finish"]],
		],
	},
	{
		// REQUEST_ERROR EXCESSIVE_LOAD (0x9) with the Retry Interval 0, and
		// with 7: the peer's value, and not what this endpoint would give
		// that code (1,001).
		name: "the Retry Interval of the REQUEST_ERROR that answers an update is the peer's",
		updates: 1,
		answers: [["05 00 03 09 00 00", ["update-error 9 retry 0"]]],
	},
	{
		name: "the Retry Interval of the REQUEST_ERROR that answers an update is the peer's, where it is not 0",
		updates: 1,
		answers: [["05 00 03 09 07 00", ["update-error 9 retry 7"]]],
	},
	{
		name: "a REQUEST_OK that answers nothing is no message of this stream",
		updates: 0,
		answers: [["07 00 01 00", "PROTOCOL_VIOLATION"]],
	},
	{
		// FORWARD (0x10) = 1, which only a subscriber sends.
		name: "a REQUEST_OK with a parameter that is not its to carry closes the session (§10.2.1)",
		updates: 1,
		answers: [["07 00 03 01 10 01", "PROTOCOL_VIOLATION"]],
	},
])("$name", ({ updates, answers }) => {
	const session = subscribed();
	for (let sent = 0; sent < updates; sent++) {
		session.updateSubscription(0n, {});
	}
	for (const [bytes, expected] of answers) {
		const outputs = session.receiveRequestStreamData(1, hex(bytes), false);
		if (expected === "PROTOCOL_VIOLATION") {
			expect(session.closed).toBe(true);
			expect(outputs.map((output) => output.type)).toContain("close-session");
		} else {
			expect(told(outputs)).toEqual(expected);
			expect(session.closed).toBe(false);
		}
	}
});

test.each<{ name: string; act: (session: Session) => unknown }>([
	{
		name: "a request that is not a SUBSCRIBE of this endpoint",
		act: (session) => session.updateSubscription(6n, {}),
	},
	{
		// §10.2.10: EXPIRES is the publisher's to send.
		name: "a parameter a REQUEST_UPDATE may not carry",
		act: (session) => session.updateSubscription(0n, { expires: 5n }),
	},
	{
		// PUBLISH_DONE TRACK_ENDED (0x2) with a Stream Count of 1: the entry
		// stays for the stream that is counted, and is Terminated (§5.1).
		name: "a subscription whose PUBLISH_DONE came",
		act: (session) => {
			session.receiveRequestStreamData(1, hex("0b 00 03 02 01 00"), false);
			return session.updateSubscription(0n, {});
		},
	},
	{
		name: "a seventeenth update while sixteen wait for their answer",
		act: (session) => {
			for (let sent = 0; sent < 16; sent++) session.updateSubscription(0n, {});
			return session.updateSubscription(0n, {});
		},
	},
])(
	"updateSubscription throws for $name, and uses up no Request ID",
	({ act }) => {
		const session = subscribed();
		expect(() => act(session)).toThrow(RangeError);
	},
);

test("a SUBSCRIBE that has no answer yet cannot be updated", () => {
	const session = createSession({});
	session.start();
	session.subscribe(TRACK);
	expect(() => session.updateSubscription(0n, {})).toThrow(RangeError);
});
