import { describe, expect, test } from "bun:test";
import { classifyDatagram } from "../../src/core/datagram.ts";
import { createSession } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fixtureTable, hex } from "../helpers.ts";

describe("classifyDatagram (§11, §11.3.1, §11.5.2)", () => {
	// fixtures/moqt-18-verdicts/datagrams.tsv, which the Rust implementation
	// reads as well. §11: "An endpoint that receives an unknown datagram type
	// MUST close the session"; §11.3.1 for the types of the form 0b00X0XXXX
	// and STATUS with END_OF_GROUP; §11.5.2: "The receiver MUST discard all
	// data received in a padding datagram". An OBJECT_DATAGRAM is dropped
	// unread as well (known gap on datagrams).
	const rows = fixtureTable("moqt-18-verdicts", "datagrams.tsv");
	test("the table has its 33 rows of 3 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(33).fill(3));
	});
	test.each(rows)("%s: '%s' is %s", (_name, bytes, verdict) => {
		const kind = classifyDatagram(
			hex((bytes ?? "").match(/../g)?.join(" ") ?? ""),
		).kind;
		expect(kind === "fault" ? "closed:0x3" : "dropped").toBe(verdict ?? "");
	});

	// What each is taken for, beyond the table.
	test.each([
		{ bytes: "00 09 00 61", kind: { kind: "object", type: 0n } },
		{ bytes: "80 02", kind: { kind: "object", type: 2n } },
		{ bytes: "f0 13 2b 3e 29 01", kind: { kind: "padding" } },
		{
			bytes: "80 22",
			kind: {
				kind: "fault",
				reason: "datagram type 0x22 has both STATUS and END_OF_GROUP",
			},
		},
		{
			bytes: "80",
			kind: { kind: "fault", reason: "datagram ended inside its type" },
		},
	])("'$bytes'", ({ bytes, kind }) => {
		expect(classifyDatagram(hex(bytes))).toEqual(kind);
	});
});

describe("Session.receiveDatagram", () => {
	// `code` 3 is PROTOCOL_VIOLATION (§3.5).
	test.each<{ bytes: string; outputs: SessionOutput[]; closed: boolean }>([
		{
			bytes: "10",
			outputs: [
				{
					type: "close-session",
					code: 3,
					reason: "unknown datagram type 0x10",
				},
			],
			closed: true,
		},
		{
			bytes: "",
			outputs: [{ type: "close-session", code: 3, reason: "empty datagram" }],
			closed: true,
		},
		{ bytes: "00 09 00 61", outputs: [], closed: false },
		{ bytes: "f0 13 2b 3e 29 00", outputs: [], closed: false },
	])("'$bytes'", ({ bytes, outputs, closed }) => {
		const session = createSession();
		session.start();
		expect(session.receiveDatagram(hex(bytes))).toEqual(outputs);
		expect(session.closed).toBe(closed);
	});

	test("does nothing once the session is closed", () => {
		const session = createSession();
		session.start();
		session.receiveDatagram(hex("10"));
		expect(session.receiveDatagram(hex("10"))).toEqual([]);
	});
});
