import { expect, test } from "bun:test";
import {
	broadcastNamespaces,
	relayOf,
	type SeatAnswer,
	seatFor,
	tokenForNamespace,
	transcriptPrefix,
} from "../src/core/rules.ts";

/** The room "blue", the demo's broadcast, and the two transcripts of the demo. */
const BLUE = {
	room: "blue",
	broadcasts: [["xmoq", "demo"]],
	transcripts: [
		["xmoq", "demo"],
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
		asked: ["xmoq", "chat", "blue"],
		namespace: ["xmoq", "chat", "blue"],
	},
	{
		name: "pub as one in the room's chat",
		role: "pub",
		asked: ["xmoq", "chat", "blue", "ann"],
		namespace: ["xmoq", "chat", "blue", "ann"],
	},
	{
		name: "sub in a transcript of the broadcast",
		role: "sub",
		asked: ["xmoq", "transcript", "xmoq", "demo"],
		namespace: ["xmoq", "transcript", "xmoq", "demo"],
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
		asked: ["xmoq", "demo"],
		namespace: ["xmoq", "demo"],
	},
	{
		name: "pub in a broadcast",
		role: "pub",
		asked: ["xmoq", "demo"],
		namespace: ["xmoq", "demo"],
	},
	{
		name: "fields that are not text are made text",
		role: "pub",
		asked: ["xmoq", "chat", "blue", 7],
		namespace: ["xmoq", "chat", "blue", "7"],
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
		asked: ["xmoq", "chat", "red"],
		namespace: undefined,
	},
	{
		name: "not pub as one in another room's chat",
		role: "pub",
		asked: ["xmoq", "chat", "red", "ann"],
		namespace: undefined,
	},
	{
		name: "not pub in the room's chat itself",
		role: "pub",
		asked: ["xmoq", "chat", "blue"],
		namespace: undefined,
	},
	{
		name: "not pub under one in the chat",
		role: "pub",
		asked: ["xmoq", "chat", "blue", "ann", "x"],
		namespace: undefined,
	},
	{
		name: "not sub as one in the chat",
		role: "sub",
		asked: ["xmoq", "chat", "blue", "ann"],
		namespace: undefined,
	},
	{
		name: "not pub in a transcript",
		role: "pub",
		asked: ["xmoq", "transcript", "xmoq", "demo"],
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
		asked: ["xmoq", "demo", "x"],
		namespace: undefined,
	},
	{
		name: "not with a field *",
		role: "sub",
		asked: ["xmoq", "chat", "*"],
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
		tokenForNamespace("sub", ["xmoq", "demo"], { ...BLUE, room: "a/b" }),
	).toBeUndefined();
});

test.each<{
	name: string;
	setting: string | undefined;
	namespaces: string[][];
}>([
	{ name: "none where it is not set", setting: undefined, namespaces: [] },
	{ name: "none where it is empty", setting: "", namespaces: [] },
	{
		name: "several",
		setting: "xmoq/demo,xmoq/other/one",
		namespaces: [
			["xmoq", "demo"],
			["xmoq", "other", "one"],
		],
	},
])("the namespaces of broadcasts: $name", ({ setting, namespaces }) => {
	expect(broadcastNamespaces(setting)).toEqual(namespaces);
});

test.each<{ name: string; setting: string }>([
	{ name: "an empty field", setting: "xmoq//demo" },
	{ name: "a field *", setting: "xmoq/*" },
	{ name: "an empty namespace among them", setting: "xmoq/demo," },
])("BROADCAST_NAMESPACES is an error: $name", ({ setting }) => {
	expect(() => broadcastNamespaces(setting)).toThrow(RangeError);
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

/** The others of a room of 49 seated, p0 to p48. */
const seated49 = Array.from({ length: 49 }, (_, index) => `p${index}`);

test.each<{
	name: string;
	id: unknown;
	room: string;
	mine: string | undefined;
	others: (string | undefined)[];
	answer: SeatAnswer;
}>([
	{
		name: "a seat, counted from now on",
		id: "ann",
		room: "blue",
		mine: undefined,
		others: [undefined, "bob"],
		answer: {
			type: "seat",
			publish: ["xmoq", "meet", "blue", "ann"],
			subscribe: ["xmoq", "meet", "blue", "*"],
			counted: true,
		},
	},
	{
		name: "the same id on the same socket again, full or not",
		id: "p3",
		room: "blue",
		mine: "p3",
		others: seated49.filter((id) => id !== "p3"),
		answer: {
			type: "seat",
			publish: ["xmoq", "meet", "blue", "p3"],
			subscribe: ["xmoq", "meet", "blue", "*"],
			counted: false,
		},
	},
	{
		name: "the 50th is told it is full",
		id: "p49",
		room: "blue",
		mine: undefined,
		others: seated49,
		answer: { type: "full" },
	},
	{
		name: "not for an empty id",
		id: "",
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id over 64",
		id: "x".repeat(65),
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id with a /",
		id: "a/b",
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id *",
		id: "*",
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for an id that is not text",
		id: 7,
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-participant" },
	},
	{
		name: "not for another id on a socket that joined",
		id: "other",
		room: "blue",
		mine: "ann",
		others: [],
		answer: { type: "refused", why: "another-participant" },
	},
	{
		name: "not for an id another socket has",
		id: "bob",
		room: "blue",
		mine: undefined,
		others: ["bob"],
		answer: { type: "refused", why: "taken" },
	},
	{
		name: "not in a room whose name has a /",
		id: "ann",
		room: "a/b",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-meeting-room" },
	},
	{
		name: "not in a room with no name",
		id: "ann",
		room: "",
		mine: undefined,
		others: [],
		answer: { type: "refused", why: "not-a-meeting-room" },
	},
])("join-meeting: $name", ({ id, room, mine, others, answer }) => {
	expect(seatFor(id, room, mine, others)).toEqual(answer);
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
