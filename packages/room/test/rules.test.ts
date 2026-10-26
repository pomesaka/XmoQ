import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	holdsSeat,
	relayOf,
	type SeatAnswer,
	seatFor,
	seatTimesOf,
	tokenForNamespace,
	transcriptPrefix,
} from "../src/core/rules.ts";

/** The room of the broadcast "blue": its broadcast, and two transcripts. */
const BLUE = {
	chat: { kind: "broadcast" as const, name: "blue" },
	broadcast: ["xmoq", "broadcast", "blue"],
	transcripts: [
		["xmoq", "broadcast", "blue"],
		["xmoq", "meet", "blue"],
	],
};

test.each<{
	name: string;
	role: "pub" | "sub";
	asked: unknown[];
	namespace: string[] | undefined;
}>([
	{
		name: "sub in the room's chat",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "blue"],
		namespace: ["xmoq", "chat", "broadcast", "blue"],
	},
	{
		name: "pub as one in the room's chat",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "blue", "ann"],
		namespace: ["xmoq", "chat", "broadcast", "blue", "ann"],
	},
	{
		name: "sub in a transcript of the broadcast",
		role: "sub",
		asked: ["xmoq", "transcript", "xmoq", "broadcast", "blue"],
		namespace: ["xmoq", "transcript", "xmoq", "broadcast", "blue"],
	},
	{
		name: "sub in the transcript of the meeting",
		role: "sub",
		asked: ["xmoq", "transcript", "xmoq", "meet", "blue"],
		namespace: ["xmoq", "transcript", "xmoq", "meet", "blue"],
	},
	{
		name: "sub in a broadcast",
		role: "sub",
		asked: ["xmoq", "broadcast", "blue"],
		namespace: ["xmoq", "broadcast", "blue"],
	},
	{
		name: "pub in a broadcast",
		role: "pub",
		asked: ["xmoq", "broadcast", "blue"],
		namespace: ["xmoq", "broadcast", "blue"],
	},
	{
		name: "fields that are not text are made text",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "blue", 7],
		namespace: ["xmoq", "chat", "broadcast", "blue", "7"],
	},
	{
		name: "not sub in xmoq, which tells of every room",
		role: "sub",
		asked: ["xmoq"],
		namespace: undefined,
	},
	{
		name: "not sub in the chat of every room",
		role: "sub",
		asked: ["xmoq", "chat"],
		namespace: undefined,
	},
	{
		name: "not in another room's chat",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "red"],
		namespace: undefined,
	},
	{
		name: "not pub as one in another room's chat",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "red", "ann"],
		namespace: undefined,
	},
	{
		name: "not pub in the room's chat itself",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "blue"],
		namespace: undefined,
	},
	{
		name: "not pub under one in the chat",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "blue", "ann", "x"],
		namespace: undefined,
	},
	{
		name: "not in the broadcast of another room",
		role: "sub",
		asked: ["xmoq", "broadcast", "red"],
		namespace: undefined,
	},
	{
		name: "not in the chat of a meeting of the same name",
		role: "sub",
		asked: ["xmoq", "chat", "meeting", "blue"],
		namespace: undefined,
	},
	{
		name: "not in the chat of the old form, without the kind",
		role: "sub",
		asked: ["xmoq", "chat", "blue"],
		namespace: undefined,
	},
	{
		name: "not sub as one in the chat",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "blue", "ann"],
		namespace: undefined,
	},
	{
		name: "not pub in a transcript",
		role: "pub",
		asked: ["xmoq", "transcript", "xmoq", "broadcast", "blue"],
		namespace: undefined,
	},
	{
		name: "not in another room's transcript",
		role: "sub",
		asked: ["xmoq", "transcript", "xmoq", "meet", "red"],
		namespace: undefined,
	},
	{
		name: "not under a broadcast",
		role: "sub",
		asked: ["xmoq", "broadcast", "blue", "x"],
		namespace: undefined,
	},
	{
		name: "sub under the chat, to watch who is in it",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "blue", "*"],
		namespace: ["xmoq", "chat", "broadcast", "blue", "*"],
	},
	{
		name: "not pub under the chat with a *",
		role: "pub",
		asked: ["xmoq", "chat", "broadcast", "blue", "*"],
		namespace: undefined,
	},
	{
		name: "not with a field *",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "*"],
		namespace: undefined,
	},
	{
		name: "not with a * that is not the last field",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "blue", "*", "x"],
		namespace: undefined,
	},
	{
		name: "not under another room's chat",
		role: "sub",
		asked: ["xmoq", "chat", "broadcast", "red", "*"],
		namespace: undefined,
	},
	{
		name: "not with a / in a field",
		role: "sub",
		asked: ["xmoq/chat", "blue"],
		namespace: undefined,
	},
	{
		name: "not in a meeting",
		role: "pub",
		asked: ["xmoq", "meet", "blue", "x"],
		namespace: undefined,
	},
])("token-for: $name", ({ role, asked, namespace }) => {
	expect(tokenForNamespace(role, asked, BLUE)).toEqual(namespace);
});

test("token-for: nothing in a room whose name has a /", () => {
	expect(
		tokenForNamespace("sub", ["xmoq", "broadcast", "blue"], {
			...BLUE,
			chat: { kind: "broadcast", name: "a/b" },
		}),
	).toBeUndefined();
});

test.each<{
	name: string;
	prefix: unknown;
	room: string;
	fields: string[] | null;
}>([
	{
		name: "the room put in",
		prefix: "xmoq/meet/{room}",
		room: "blue",
		fields: ["xmoq", "meet", "blue"],
	},
	{
		name: "one without the room",
		prefix: "xmoq/demo",
		room: "blue",
		fields: ["xmoq", "demo"],
	},
	{ name: "none without a setting", prefix: "", room: "blue", fields: null },
	{
		name: "none for a setting that is not text",
		prefix: undefined,
		room: "blue",
		fields: null,
	},
	{
		name: "none for a room with a /",
		prefix: "xmoq/meet/{room}",
		room: "a/b",
		fields: null,
	},
	{
		name: "none for a room with a *",
		prefix: "xmoq/meet/{room}",
		room: "*",
		fields: null,
	},
	{
		name: "none for an empty field",
		prefix: "xmoq//{room}",
		room: "blue",
		fields: null,
	},
	{ name: "none for a field *", prefix: "xmoq/*", room: "blue", fields: null },
])("the prefix of a transcript: $name", ({ prefix, room, fields }) => {
	expect(transcriptPrefix(prefix, room)).toEqual(fields);
});

/** Seats that hold until `expiresAt`, by id. */
const held = (ids: readonly string[], expiresAt: number) =>
	new Map(ids.map((id) => [id, { lease: `lease-${id}`, expiresAt }]));

/** 49 seats, p0 to p48, that hold until 1,000. */
const seated49 = Array.from({ length: 49 }, (_, index) => `p${index}`);

// The seats are looked at at 500; one that holds until 500 has run out.
test.each<{
	name: string;
	id: unknown;
	room: string;
	seats: Map<string, { lease: string; expiresAt: number }>;
	answer: SeatAnswer;
}>([
	{
		name: "a seat",
		id: "ann",
		room: "blue",
		seats: held(["bob"], 1000),
		answer: {
			type: "seat",
			publish: ["xmoq", "meet", "blue", "ann"],
			subscribe: ["xmoq", "meet", "blue", "*"],
		},
	},
	{
		name: "the 50th is told it is full",
		id: "p49",
		room: "blue",
		seats: held(seated49, 1000),
		answer: { type: "full" },
	},
	{
		name: "a seat that ran out leaves its place to another",
		id: "p49",
		room: "blue",
		seats: new Map([...held(seated49.slice(1), 1000), ...held(["p0"], 500)]),
		answer: {
			type: "seat",
			publish: ["xmoq", "meet", "blue", "p49"],
			subscribe: ["xmoq", "meet", "blue", "*"],
		},
	},
	{
		name: "and its id",
		id: "p0",
		room: "blue",
		seats: held(["p0"], 500),
		answer: {
			type: "seat",
			publish: ["xmoq", "meet", "blue", "p0"],
			subscribe: ["xmoq", "meet", "blue", "*"],
		},
	},
	{
		name: "not for an id a seat that holds has",
		id: "bob",
		room: "blue",
		seats: held(["bob"], 1000),
		answer: { type: "refused", why: "taken" },
	},
	{
		name: "not for an empty id",
		id: "",
		room: "blue",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id over 64",
		id: "x".repeat(65),
		room: "blue",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id with a /",
		id: "a/b",
		room: "blue",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id *",
		id: "*",
		room: "blue",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id that is not text",
		id: 7,
		room: "blue",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not in a room whose name has a /",
		id: "ann",
		room: "a/b",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-meeting-room" },
	},
	{
		name: "not in a room with no name",
		id: "ann",
		room: "",
		seats: held([], 1000),
		answer: { type: "refused", why: "not-a-meeting-room" },
	},
])("a seat: $name", ({ id, room, seats, answer }) => {
	expect(seatFor(id, room, seats, 500)).toEqual(answer);
});

/** The rows of `fixtures/meeting-presence/namespaces.tsv`, which `crates/meeting-tracks` and `@xmoq/meeting` read too. */
const namespaceRows = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"meeting-presence",
		"namespaces.tsv",
	),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => row.split("\t"));

/** The fields of the shared row `seat-subscribe` of `room`. */
const seatSubscribeRow = (room: string): string[] => {
	const fields = namespaceRows.find(
		([, ofRoom, , what]) => ofRoom === room && what === "seat-subscribe",
	)?.[4];
	if (fields === undefined) throw new Error(`no seat-subscribe row of ${room}`);
	return fields.split(" ");
};

test.each(namespaceRows.filter(([, , , what]) => what === "participant"))(
	"a seat's namespaces are the shared rows: %s",
	(_name, room, id, _what, namespace) => {
		expect(seatFor(id, room ?? "", new Map(), 500)).toEqual({
			type: "seat",
			publish: (namespace ?? "").split(" "),
			subscribe: seatSubscribeRow(room ?? ""),
		});
	},
);

test.each<{ name: string; id: string; lease: unknown; holds: boolean }>([
	{
		name: "with its lease, before it runs out",
		id: "bob",
		lease: "lease-bob",
		holds: true,
	},
	{
		name: "not with another's lease",
		id: "bob",
		lease: "lease-ann",
		holds: false,
	},
	{ name: "not without a lease", id: "bob", lease: undefined, holds: false },
	{ name: "not once it ran out", id: "ann", lease: "lease-ann", holds: false },
	{
		name: "not for a seat nobody has",
		id: "cid",
		lease: "lease-cid",
		holds: false,
	},
])("renewing or leaving a seat: $name", ({ id, lease, holds }) => {
	const seats = new Map([...held(["bob"], 1000), ...held(["ann"], 500)]);
	expect(holdsSeat(id, lease, seats, 500)).toBe(holds);
});

test.each<{
	name: string;
	seatMs: string | undefined;
	renewMs: string | undefined;
	times: { seatMs: number; renewMs: number } | string;
}>([
	{
		name: "the defaults",
		seatMs: undefined,
		renewMs: "",
		times: { seatMs: 60000, renewMs: 20000 },
	},
	{
		name: "both set",
		seatMs: "30000",
		renewMs: "10000",
		times: { seatMs: 30000, renewMs: 10000 },
	},
	{
		name: "a seat of no time",
		seatMs: "0",
		renewMs: undefined,
		times: "MEETING_SEAT_MS is a whole number of milliseconds from 1, not 0",
	},
	{
		name: "a renewal of a fraction",
		seatMs: undefined,
		renewMs: "1.5",
		times: "MEETING_RENEW_MS is a whole number of milliseconds from 1, not 1.5",
	},
	{
		name: "a renewal as long as the seat",
		seatMs: "20000",
		renewMs: undefined,
		times:
			"MEETING_RENEW_MS (20000) is to be shorter than MEETING_SEAT_MS (20000)",
	},
])("the times of a seat: $name", ({ seatMs, renewMs, times }) => {
	if (typeof times === "string") {
		expect(() => seatTimesOf(seatMs, renewMs)).toThrow(new RangeError(times));
	} else expect(seatTimesOf(seatMs, renewMs)).toEqual(times);
});

test.each<{
	name: string;
	relayUrl: unknown;
	certSha256Hex: unknown;
	relay: { relayUrl: string; certSha256Hex: string } | null;
}>([
	{
		name: "both told",
		relayUrl: "https://r/",
		certSha256Hex: "ab",
		relay: { relayUrl: "https://r/", certSha256Hex: "ab" },
	},
	{
		name: "none without the hash",
		relayUrl: "https://r/",
		certSha256Hex: "",
		relay: null,
	},
	{
		name: "none without the URL",
		relayUrl: undefined,
		certSha256Hex: "ab",
		relay: null,
	},
])("the relay of the rooms: $name", ({ relayUrl, certSha256Hex, relay }) => {
	expect(relayOf(relayUrl, certSha256Hex)).toEqual(relay);
});
