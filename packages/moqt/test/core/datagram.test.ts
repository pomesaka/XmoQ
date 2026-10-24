import { describe, expect, test } from "bun:test";
import { classifyDatagram } from "../../src/core/datagram.ts";
import { createSession } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { hex } from "../helpers.ts";

describe("classifyDatagram (§11, §11.3.1, §11.5.2)", () => {
	// The varint is that of §1.4.1: one byte below 0x80, and `80 xx` the
	// two-byte form, which may also carry a small value (non-minimal).
	test.each([
		{ name: "the lowest Type", bytes: "00", type: 0n },
		{ name: "the last Type of 0x00..0x0F", bytes: "0f", type: 15n },
		{ name: "STATUS alone", bytes: "20", type: 32n },
		{ name: "STATUS and PROPERTIES", bytes: "21", type: 33n },
		{ name: "STATUS and ZERO_OBJECT_ID", bytes: "24", type: 36n },
		{ name: "STATUS and DEFAULT_PRIORITY", bytes: "28", type: 40n },
		{ name: "the last Type of the form with STATUS", bytes: "2d", type: 45n },
		{
			name: "END_OF_GROUP without STATUS, in the two-byte form",
			bytes: "80 02",
			type: 2n,
		},
		{
			name: "a Type followed by its fields and payload",
			bytes: "00 09 00 61",
			type: 0n,
		},
	])("an OBJECT_DATAGRAM: $name ($bytes)", ({ bytes, type }) => {
		expect(classifyDatagram(hex(bytes))).toEqual({ kind: "object", type });
	});

	test.each([
		{ name: "with no Padding Data", bytes: "f0 13 2b 3e 29" },
		{ name: "with zeros", bytes: "f0 13 2b 3e 29 00 00" },
		// The sender MUST send zeros; the receiver MUST discard all of it.
		{ name: "with bytes that are not zero", bytes: "f0 13 2b 3e 29 01" },
	])("a PADDING DATAGRAM $name", ({ bytes }) => {
		expect(classifyDatagram(hex(bytes))).toEqual({ kind: "padding" });
	});

	test.each([
		{ bytes: "10", reason: "unknown datagram type 0x10" },
		{ bytes: "1f", reason: "unknown datagram type 0x1f" },
		{ bytes: "30", reason: "unknown datagram type 0x30" },
		{ bytes: "40", reason: "unknown datagram type 0x40" },
		{ bytes: "7f", reason: "unknown datagram type 0x7f" },
		{ bytes: "80 80", reason: "unknown datagram type 0x80" },
		{ bytes: "80 a0", reason: "unknown datagram type 0xa0" },
		// The type of a padding stream (§11.5.1) is none of a datagram.
		{ bytes: "f0 13 2b 3e 28", reason: "unknown datagram type 0x132b3e28" },
		{
			bytes: "22",
			reason: "datagram type 0x22 has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "23",
			reason: "datagram type 0x23 has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "26",
			reason: "datagram type 0x26 has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "27",
			reason: "datagram type 0x27 has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "2a",
			reason: "datagram type 0x2a has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "2b",
			reason: "datagram type 0x2b has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "2e",
			reason: "datagram type 0x2e has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "2f",
			reason: "datagram type 0x2f has both STATUS and END_OF_GROUP",
		},
		{
			bytes: "80 22",
			reason: "datagram type 0x22 has both STATUS and END_OF_GROUP",
		},
		{ bytes: "", reason: "empty datagram" },
		{ bytes: "80", reason: "datagram ended inside its type" },
		{ bytes: "f0 13 2b 3e", reason: "datagram ended inside its type" },
	])("a fault: '$bytes'", ({ bytes, reason }) => {
		expect(classifyDatagram(hex(bytes))).toEqual({ kind: "fault", reason });
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
