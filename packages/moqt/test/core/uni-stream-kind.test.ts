import { describe, expect, test } from "bun:test";
import { createByteReader } from "../../src/core/bytes.ts";
import { createSession, uniStreamKind } from "../../src/core/session.ts";
import { fixtureTable, hex } from "../helpers.ts";

// fixtures/moqt-18-verdicts/streams.tsv, which the Rust implementation reads
// as well: the stream types of Table 3 (§3.4); "An endpoint that receives an
// unknown stream type MUST close the session" (§3.4), and a padding stream is
// read and discarded (§11.5.1).
describe("fixtures/moqt-18-verdicts/streams.tsv", () => {
	const rows = fixtureTable("moqt-18-verdicts", "streams.tsv");
	test("the table has its 11 rows of 3 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(11).fill(3));
	});
	test.each(rows)("%s: %s is %s", (_name, bytes, verdict) => {
		const input = hex((bytes ?? "").match(/../g)?.join(" ") ?? "");
		const kind = uniStreamKind(createByteReader(input).varint());
		if (kind !== "unknown") {
			expect<string>(kind).toBe(verdict ?? "");
			return;
		}
		const session = createSession();
		session.start();
		const closed = session
			.receivePeerUniStreamData(7, input, false)
			.find((output) => output.type === "close-session");
		expect(
			closed?.type === "close-session"
				? `closed:0x${closed.code.toString(16)}`
				: "open",
		).toBe(verdict ?? "");
	});
});
