import { expect, test } from "bun:test";
import { closeCode, closeReason } from "../../src/webtransport/close-info.ts";
import {
	connectOver,
	type MoqtConnection,
} from "../../src/webtransport/connection.ts";
import {
	closeReasonFixtures,
	hex,
	MOQTAIL_RELAY_SETUP,
	parseCloseReasonFixture,
	toHex,
	utf8,
} from "../helpers.ts";
import { createFakePeer, type FakePeer } from "./fake-transport.ts";

async function connected(): Promise<{
	peer: FakePeer;
	connection: MoqtConnection;
}> {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport);
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	return { peer, connection: await connecting };
}

test.each([
	{ code: 0, sent: 0 },
	{ code: 1, sent: 1 },
	{ code: 255, sent: 255 },
	{ code: 256, sent: 256 },
	{ code: 4294967295, sent: 4294967295 },
])("closeCode($code) is a code of 32 bits", ({ code, sent }) => {
	expect(closeCode(code)).toBe(sent);
});

// -0 is a whole number in range; the browser sends 0, and so `closed` has
// to say 0 and not -0.
test("closeCode(-0) is 0", () => {
	expect(Object.is(closeCode(-0), 0)).toBe(true);
});

// Chromium 156 sent each of these as another code (2^32 as 0, -1 as 2^32 - 1,
// 1.5 as 1, NaN as 0), so they are refused.
test.each([
	{
		code: 4294967296,
		message:
			"a session is closed with a whole number from 0 to 4294967295, not 4294967296",
	},
	{
		code: -1,
		message:
			"a session is closed with a whole number from 0 to 4294967295, not -1",
	},
	{
		code: 1.5,
		message:
			"a session is closed with a whole number from 0 to 4294967295, not 1.5",
	},
	{
		code: Number.NaN,
		message:
			"a session is closed with a whole number from 0 to 4294967295, not NaN",
	},
	{
		code: Number.POSITIVE_INFINITY,
		message:
			"a session is closed with a whole number from 0 to 4294967295, not Infinity",
	},
])(
	"close($code) throws a RangeError and leaves the session open",
	async ({ code, message }) => {
		const { peer, connection } = await connected();
		expect(() => connection.close(code, "bye")).toThrow(
			new RangeError(message),
		);
		expect(peer.closeInfo()).toBeUndefined();

		connection.close(5, "bye");
		expect(peer.closeInfo()).toEqual({ closeCode: 5, reason: "bye" });
		expect(await connection.closed).toEqual({
			type: "local",
			code: 5,
			reason: "bye",
		});
	},
);

/** `count` times "a", then `tail`. */
function padded(count: number, tail: string): string {
	return "a".repeat(count) + tail;
}

// The bytes are the ones Chromium 156 sent for each reason and the own relay
// received (fixtures/webtransport-close/README.md): at most 1024 bytes of
// UTF-8, cut between two characters, a leading U+FEFF kept, U+FFFD for half
// of a surrogate pair. The Rust implementation reads the same file.
test.each(closeReasonFixtures())(
	"closeReason gives the bytes Chromium sent: $name",
	({ reason, sentHex }) => {
		expect(toHex(utf8(closeReason(reason)))).toBe(sentHex);
	},
);

// The transport the other tests play has to treat a reason as the browser
// does, or a test of `close()` shows nothing about the browser.
test.each(closeReasonFixtures())(
	"the fake transport sends the bytes Chromium sent: $name",
	({ reason, sentHex }) => {
		const peer = createFakePeer();
		peer.transport.close({ closeCode: 0, reason });
		expect(toHex(utf8(peer.closeInfo()?.reason ?? "not closed"))).toBe(sentHex);
	},
);

// `closed` has to say what the peer was told: the browser sends the cut
// reason and reports the whole one.
test("close() sends a reason cut to 1024 bytes and reports the same in closed", async () => {
	const { peer, connection } = await connected();
	connection.close(0, padded(1023, "é"));
	expect(peer.closeInfo()).toEqual({ closeCode: 0, reason: padded(1023, "") });
	expect(await connection.closed).toEqual({
		type: "local",
		code: 0,
		reason: padded(1023, ""),
	});
});

// What the session core closes with is reported as it was sent as well.
test("a session closed for a violation reports the reason as it was sent", async () => {
	const { peer, connection } = await connected();
	// A second SETUP on the control stream is a PROTOCOL_VIOLATION.
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const close = await connection.closed;
	const sent = peer.closeInfo();
	expect(sent?.closeCode).toBe(3);
	expect(close).toMatchObject({
		type: "error",
		error: { message: sent?.reason },
	});
});

// Where another error ends the session, the peer gets its message cut in
// the same way, and `closed` hands on the error as it was.
test("a session that fails sends the error's message cut to 1024 bytes", async () => {
	const peer = createFakePeer();
	const connecting = connectOver(peer.transport, {
		diagnostics: {
			onEvent() {
				throw new Error(padded(1023, "é"));
			},
		},
	});
	await peer.openUni().write(hex(MOQTAIL_RELAY_SETUP));
	const failure = await connecting.then(
		() => undefined,
		(error: unknown) => error,
	);
	expect(failure).toMatchObject({ close: { type: "error" } });
	expect(peer.closeInfo()).toEqual({ closeCode: 1, reason: padded(1023, "") });
});

test("a row of the close reason fixture is read as the README says", () => {
	expect(parseCloseReasonFixture("name\tutf8\t6279\t62")).toEqual({
		name: "name",
		reason: "by",
		sentHex: "62",
	});
	expect(parseCloseReasonFixture("half\tutf16\t0061d83d\t61efbfbd")).toEqual({
		name: "half",
		reason: "a\ud83d",
		sentHex: "61 ef bf bd",
	});
});

// A row that is read in another way than it was written pins nothing.
test.each([
	{ row: "name\tutf8\t62", message: "does not have four columns" },
	{ row: "name\tutf8\t62\t62\t62", message: "does not have four columns" },
	{ row: "\tutf8\t62\t62", message: "has no name" },
	{
		row: "name\tlatin1\t62\t62",
		message: "has an encoding that is neither utf8 nor utf16",
	},
	{
		row: "name\tutf8\t627\t62",
		message: "has a column that is not whole bytes in hexadecimal",
	},
	{
		row: "name\tutf8\t62\t6g",
		message: "has a column that is not whole bytes in hexadecimal",
	},
	{
		row: "name\tutf8\t62\t6A",
		message: "has a column that is not whole bytes in hexadecimal",
	},
	{ row: "name\tutf16\t006100\t61", message: "has half of a UTF-16 code unit" },
])(
	"a broken row of the close reason fixture is refused: $message",
	({ row, message }) => {
		expect(() => parseCloseReasonFixture(row)).toThrow(
			`a row of the close reason fixture ${message}: ${row}`,
		);
	},
);
