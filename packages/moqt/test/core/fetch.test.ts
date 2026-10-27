import { describe, expect, test } from "bun:test";
import {
	createFetchStreamDecoder,
	createFetchStreamEncoder,
	type FetchObject,
	type FetchStreamItem,
} from "../../src/core/fetch.ts";
import type { GroupOrder } from "../../src/core/parameters.ts";
import {
	BY_HAND_CONTROL_MESSAGES,
	byHand,
	byHandNames,
	fixtureTable,
	hex,
	toHex,
} from "../helpers.ts";

const LIMITS = {
	maxObjectPayloadLength: 1024,
	maxObjectPropertiesLength: 1024,
};

function plain(
	groupId: bigint,
	subgroupId: bigint | undefined,
	objectId: bigint,
	publisherPriority: number,
	payload: string,
): FetchStreamItem {
	return {
		type: "object",
		object: {
			groupId,
			subgroupId,
			objectId,
			publisherPriority,
			properties: [],
			payload: hex(payload),
		},
	};
}

const header = (requestId: bigint): FetchStreamItem => ({
	type: "header",
	header: { requestId },
});

// Bytes written by hand from §11.4.4, for what no capture has, read from
// fixtures/moqt-18-by-hand/, which the Rust implementation reads too. The
// reader of the draft and the writer of the expectation are the same, so
// these check nothing against another implementation: they check that the
// two own implementations read the same bytes as the same values. The Group
// Order is not in the bytes; the README there has it for each file.
const BY_HAND: {
	file: string;
	groupOrder: GroupOrder;
	items: FetchStreamItem[];
}[] = [
	{
		// Flags 0x1C: Group 9, Subgroup 0 by the mode, Object 0, priority 7.
		// Then 0x0C with Group ID Delta 1: descending, so Group 9 - (1 + 1) =
		// 7, and the Object ID Delta 1 is the Object ID. Then 0x0C with deltas
		// 0 and 5: Group 6, Object 5.
		file: "fetch-stream-descending",
		groupOrder: "Descending",
		items: [
			header(4n),
			plain(9n, 0n, 0n, 7, "aa"),
			plain(7n, 0n, 1n, 7, "bb"),
			plain(6n, 0n, 5n, 7, "cc"),
		],
	},
	{
		// An Object, then End of Non-Existent Range (0x8C, which is the two
		// bytes 0x80 0x8C by §1.4.1) at {0, 5}, then flags 0x00: the Object ID
		// is the prior one plus one, and the prior one is the indicator's: 6.
		// Then End of Unknown Range (0x10C) at {2, 0}, then 0x04 with Object ID
		// Delta 3: Group 2 from the indicator, Object 0 + 3. The priority 7 is
		// that of the last Object.
		file: "fetch-stream-end-of-range",
		groupOrder: "Ascending",
		items: [
			header(0n),
			plain(0n, 0n, 0n, 7, "aa"),
			{
				type: "end-of-range",
				kind: "NonExistent",
				location: { group: 0n, object: 5n },
			},
			plain(0n, 0n, 6n, 7, "bb"),
			{
				type: "end-of-range",
				kind: "Unknown",
				location: { group: 2n, object: 0n },
			},
			plain(2n, 0n, 3n, 7, "cc"),
		],
	},
	{
		// An indicator first, then an Object that takes its Group and Object
		// ID from it and brings its own priority (0x10).
		file: "fetch-stream-indicator-first",
		groupOrder: "Ascending",
		items: [
			header(0n),
			{
				type: "end-of-range",
				kind: "Unknown",
				location: { group: 3n, object: 1n },
			},
			plain(3n, 0n, 2n, 7, ""),
		],
	},
	{
		// The same indicator, then flags 0x1C with Group ID Delta 5: counted
		// from the indicator's Group 3, that is Group 3 + 5 + 1 = 9, and the
		// Object ID Delta 0 is the Object ID. §11.4.4.1 would have the first
		// Object carry absolute IDs, which would make this Group 5; the notes
		// decide for §11.4.4.2 (row on the first Object of a fetch stream when
		// an indicator came before it; provisional).
		file: "fetch-stream-indicator-first-group-delta",
		groupOrder: "Ascending",
		items: [
			header(0n),
			{
				type: "end-of-range",
				kind: "Unknown",
				location: { group: 3n, object: 1n },
			},
			plain(9n, 0n, 0n, 7, ""),
		],
	},
	{
		// Flags 0x5C: the Datagram bit, so no Subgroup ID. Then 0x1F: Group ID
		// Delta 0, so Group 2; Subgroup 4 present; Object 0; priority 9. Then
		// 0x02: the prior Subgroup plus one, 5.
		file: "fetch-stream-subgroup-id-modes",
		groupOrder: "Ascending",
		items: [
			header(0n),
			plain(1n, undefined, 0n, 7, ""),
			plain(2n, 4n, 0n, 9, ""),
			plain(2n, 5n, 1n, 9, ""),
		],
	},
];

function decodeIn(bytes: Uint8Array, chunk: number, groupOrder: GroupOrder) {
	const decoder = createFetchStreamDecoder(LIMITS, groupOrder);
	const items: FetchStreamItem[] = [];
	for (let at = 0; at < bytes.length; at += chunk) {
		const result = decoder.push(bytes.subarray(at, at + chunk));
		items.push(...result.items);
		if (result.failure !== undefined) return { items, failure: result.failure };
	}
	const violation = decoder.finish();
	return violation === undefined
		? { items }
		: { items, failure: { scope: "session", error: violation } as const };
}

// WHY: a file added to fixtures/moqt-18-by-hand/ and not to the table would
// be read by the Rust implementation alone. The files that are one control
// message are read where that message is (`BY_HAND_CONTROL_MESSAGES`).
test("every file of fixtures/moqt-18-by-hand/ is in the table or a control message, and every table entry is a file", () => {
	expect(
		[...BY_HAND.map(({ file }) => file), ...BY_HAND_CONTROL_MESSAGES].sort(),
	).toEqual(byHandNames());
});

describe("fetch streams written by hand from the draft", () => {
	test.each(BY_HAND)("decodes $file", ({ file, groupOrder, items }) => {
		const bytes = hex(byHand(file));
		expect(decodeIn(bytes, bytes.length, groupOrder)).toEqual({ items });
	});

	test.each(BY_HAND)(
		"decodes $file byte by byte",
		({ file, groupOrder, items }) => {
			expect(decodeIn(hex(byHand(file)), 1, groupOrder)).toEqual({ items });
		},
	);

	test.each(BY_HAND)("encodes $file", ({ file, groupOrder, items }) => {
		const [first, ...rest] = items;
		if (first?.type !== "header") {
			throw new Error("the first item is the header");
		}
		const encoder = createFetchStreamEncoder(first.header, groupOrder);
		const stream = [
			encoder.header,
			...rest.map((item) => {
				if (item.type === "object") return encoder.object(item.object);
				if (item.type === "end-of-range") {
					return encoder.endOfRange(item.kind, item.location);
				}
				throw new Error("one header");
			}),
		];
		expect(stream.map(toHex).join(" ")).toBe(byHand(file));
	});
});

// The same bytes as `rejects_malformed_input` in crates/moqt/src/fetch.rs.
// `scope` is how far the damage reaches; `code` is the Session termination
// error code of §3.5 where the session ends.
describe("fixtures/moqt-18-verdicts/fetch-stream.tsv", () => {
	// Read by the Rust implementation as well, with Objects of 1,024 bytes of
	// payload and of properties at the most (LIMITS): a fetch stream, whole
	// and a byte at a time, and whether it is read, closes the session, makes
	// the track malformed, or ends that stream alone (a limit of this end).
	// §11.4.4.1 and §11.4.4.2: a flag that refers to a prior Object, where
	// there is none, is a PROTOCOL_VIOLATION, and so is an ID that leaves the
	// 64 bits; the draft has no case for the Subgroup of a prior datagram, nor
	// for a Subgroup ID past 2^64 - 1, and both are read as such. §2.5.1: a
	// Mandatory Track Property on an Object makes the track malformed, which
	// on a fetch stream is decided once the payload is there. The order of
	// the Objects is the sender's (§10.12.3), and is not checked.
	const rows = fixtureTable("moqt-18-verdicts", "fetch-stream.tsv");
	test("the table has its 26 rows of 4 columns", () => {
		expect(rows.map((row) => row.length)).toEqual(Array(26).fill(4));
	});
	test.each(rows)("%s, %s: %s is %s", (_name, order, bytes, verdict) => {
		const input = hex((bytes ?? "").match(/../g)?.join(" ") ?? "");
		const groupOrder: GroupOrder =
			order === "descending" ? "Descending" : "Ascending";
		for (const chunk of [input.length, 1]) {
			const { failure } = decodeIn(input, chunk, groupOrder);
			const got =
				failure === undefined
					? "ok"
					: failure.scope === "session"
						? `closed:0x${failure.error.code.toString(16)}`
						: failure.scope === "track"
							? "malformed"
							: "stopped";
			expect(got).toBe(verdict ?? "");
		}
	});
});

test("an Object ID that is left out is the prior one plus one, also in a new Group", () => {
	// Flags 0x1C: Group 2, Object 5, priority 7. Then 0x08: Group ID Delta 0,
	// so Group 3, and no Object ID Delta: "the prior Object's ID plus one,
	// regardless of which group it belongs to" (§11.4.4.1), so 6. The encoder
	// does not write a new Group that way.
	const { items } = decodeIn(
		hex("05 00 1c 02 05 07 00 08 00 00"),
		10,
		"Ascending",
	);
	expect(
		items.flatMap((item) =>
			item.type === "object"
				? [[item.object.groupId, item.object.objectId]]
				: [],
		),
	).toEqual([
		[2n, 5n],
		[3n, 6n],
	]);
});

// The two limits are on the Object Payload Length and on the Object
// Properties Length, each by itself: a length of exactly the limit is taken,
// one more is not (the table above has 1025 for both).
test("an Object whose payload and whose properties are each as long as the limit is taken", () => {
	// Properties of 1024 bytes: one pair of type 1 (odd, so with a length)
	// and 1021 bytes of value takes 1 + 2 + 1021. Then 1024 bytes of payload.
	const properties = `01 83 fd ${"00 ".repeat(1021).trim()}`;
	const payload = "aa ".repeat(1024).trim();
	const bytes = `05 00 3c 00 00 07 84 00 ${properties} 84 00 ${payload}`;
	const { items, failure } = decodeIn(hex(bytes), bytes.length, "Ascending");
	expect(failure).toBeUndefined();
	expect(
		items.map((item) =>
			item.type === "object"
				? [item.object.properties.length, item.object.payload.length]
				: item.type,
		),
	).toEqual(["header", [1, 1024]]);
});

test("more Object Properties than the local limit end the stream and not the session", () => {
	// 65 properties, one over the limit of 64 Key-Value-Pairs: each is Delta
	// Type 2 and the value 0, two bytes. Then an empty payload.
	const properties = "02 00 ".repeat(65).trim();
	const bytes = `05 00 3c 00 00 07 80 82 ${properties} 00`;
	expect(decodeIn(hex(bytes), bytes.length, "Ascending").failure).toMatchObject(
		{
			scope: "stream",
		},
	);
});

describe("createFetchStreamEncoder", () => {
	const at = (groupId: bigint, objectId: bigint): FetchObject => ({
		groupId,
		subgroupId: 0n,
		objectId,
		publisherPriority: 0,
		properties: [],
		payload: new Uint8Array(0),
	});

	// (the Group Order; the Object before; the Object that is refused).
	test.each([
		{ groupOrder: "Ascending", before: at(2n, 3n), refused: at(2n, 3n) },
		{ groupOrder: "Ascending", before: at(2n, 3n), refused: at(2n, 1n) },
		{ groupOrder: "Ascending", before: at(2n, 3n), refused: at(1n, 0n) },
		{ groupOrder: "Descending", before: at(2n, 3n), refused: at(3n, 0n) },
	] satisfies {
		groupOrder: GroupOrder;
		before: FetchObject;
		refused: FetchObject;
	}[])(
		"refuses an Object that does not come after the one before ($groupOrder, Group $refused.groupId Object $refused.objectId)",
		({ groupOrder, before, refused }) => {
			const encoder = createFetchStreamEncoder({ requestId: 0n }, groupOrder);
			encoder.object(before);
			expect(() => encoder.object(refused)).toThrow(RangeError);
			// Nothing of the refused Object was taken: the next one is encoded
			// against the Object before it. Flags 0x00, payload length 0.
			expect(toHex(encoder.object(at(2n, 4n)))).toBe("00 00");
		},
	);
});

// The Rust implementation decodes the Properties of an Object when its
// payload has arrived, and refuses a length over the limit when it reads it.
// Both own implementations have to say the same of the same bytes at the
// same point of the stream.
describe("when what is wrong with an Object is reported", () => {
	// Flags 0x3C: Group 0, Object 0, priority 7, Properties of 4 bytes with
	// the Mandatory Track Property 0x4000, then the Object Payload Length.
	const BROKEN_PROPERTIES = "05 00 3c 00 00 07 04 c0 40 00 01";

	test("an Object Payload Length over the limit is refused before Properties that break the track", () => {
		// 1025, over the limit of 1024.
		const bytes = `${BROKEN_PROPERTIES} 84 01`;
		expect(
			decodeIn(hex(bytes), bytes.length, "Ascending").failure,
		).toMatchObject({ scope: "stream" });
	});

	test("Properties that break the track are reported when the payload is there, not before", () => {
		const decoder = createFetchStreamDecoder(LIMITS, "Ascending");
		// Two bytes of payload announced, one sent.
		expect(decoder.push(hex(`${BROKEN_PROPERTIES} 02 aa`))).toEqual({
			items: [header(0n)],
		});
		expect(decoder.push(hex("bb")).failure).toMatchObject({ scope: "track" });
	});
});

describe("what createFetchStreamEncoder refuses leaves it as it was", () => {
	const at = (groupId: bigint, objectId: bigint): FetchObject => ({
		groupId,
		subgroupId: 0n,
		objectId,
		publisherPriority: 0,
		properties: [],
		payload: new Uint8Array(0),
	});

	// What the decoder takes for a malformed Track (§2.5.1, §12.8, §12.9).
	// The Rust encoder refuses the same.
	test.each([
		{
			name: "a Mandatory Track Property",
			refused: { ...at(2n, 4n), properties: [{ type: 0x4000n, value: 0n }] },
		},
		{
			name: "a Prior Object ID Gap larger than the Object ID",
			refused: { ...at(3n, 0n), properties: [{ type: 0x3en, value: 1n }] },
		},
		{
			name: "two Prior Object ID Gaps",
			refused: {
				...at(2n, 9n),
				properties: [
					{ type: 0x3en, value: 1n },
					{ type: 0x3en, value: 1n },
				],
			},
		},
		{
			name: "a Prior Group ID Gap larger than the Group ID",
			refused: { ...at(0n, 0n), properties: [{ type: 0x3cn, value: 1n }] },
		},
	] satisfies { name: string; refused: FetchObject }[])(
		"an Object with $name",
		({ refused }) => {
			const encoder = createFetchStreamEncoder({ requestId: 0n }, "Descending");
			encoder.object(at(5n, 3n));
			expect(() => encoder.object(refused)).toThrow(RangeError);
			// Flags 0x00, payload length 0: encoded against {5, 3}.
			expect(toHex(encoder.object(at(5n, 4n)))).toBe("00 00");
		},
	);

	// After Object {5, 3}. An indicator that goes back would take the
	// encoder back with it, and an Object that goes back would then be
	// written.
	test.each([
		{ order: "Ascending", name: "the Object before it", group: 5n, object: 3n },
		{
			order: "Ascending",
			name: "an earlier Object of its Group",
			group: 5n,
			object: 1n,
		},
		{ order: "Ascending", name: "an earlier Group", group: 4n, object: 9n },
		{
			order: "Descending",
			name: "an earlier Object of its Group",
			group: 5n,
			object: 1n,
		},
		{
			order: "Descending",
			name: "a Group against the order",
			group: 6n,
			object: 0n,
		},
	] as const)(
		"an End of Range indicator at $name is refused ($order), and the encoder goes on as it was",
		({ order, group, object }) => {
			const encoder = createFetchStreamEncoder({ requestId: 0n }, order);
			encoder.object(at(5n, 3n));
			expect(() =>
				encoder.endOfRange("NonExistent", { group, object }),
			).toThrow(RangeError);
			// What a refused indicator must not have let through.
			expect(() => encoder.object(at(5n, 2n))).toThrow(RangeError);
			// The next Object in order: no field but the payload length.
			expect(toHex(encoder.object(at(5n, 4n)))).toBe("00 00");
		},
	);

	test.each([
		{
			order: "Ascending",
			name: "a later Object of its Group",
			group: 5n,
			object: 9n,
		},
		{
			order: "Ascending",
			name: "a later Group, at any Object",
			group: 7n,
			object: 0n,
		},
		{
			order: "Descending",
			name: "a Group that follows by the order, at any Object",
			group: 3n,
			object: 0n,
		},
	] as const)(
		"an End of Range indicator at $name is taken ($order)",
		({ order, group, object }) => {
			const encoder = createFetchStreamEncoder({ requestId: 0n }, order);
			encoder.object(at(5n, 3n));
			expect(
				encoder.endOfRange("NonExistent", { group, object }).length,
			).toBeGreaterThan(0);
		},
	);

	test("an End of Range indicator at a Location past 2^64 - 1", () => {
		const encoder = createFetchStreamEncoder({ requestId: 0n }, "Ascending");
		encoder.object(at(2n, 3n));
		expect(() =>
			encoder.endOfRange("Unknown", { group: 2n ** 64n, object: 0n }),
		).toThrow(RangeError);
		expect(toHex(encoder.object(at(2n, 4n)))).toBe("00 00");
	});
});
