import { expect, test } from "bun:test";
import {
	relayOf,
	type SeatAnswer,
	seatFor,
	tokenForNamespace,
	transcriptPrefix,
} from "../src/core/rules.ts";

test.each<{ name: string; asked: unknown[]; namespace: string[] | undefined }>([
	{
		name: "a namespace is answered",
		asked: ["xmoq", "demo"],
		namespace: ["xmoq", "demo"],
	},
	{
		name: "fields that are not text are made text",
		asked: ["xmoq", 7],
		namespace: ["xmoq", "7"],
	},
	{ name: "not with a field *", asked: ["xmoq", "*"], namespace: undefined },
	{
		name: "not with a / in a field",
		asked: ["xmoq/demo"],
		namespace: undefined,
	},
	{
		name: "not in a meeting",
		asked: ["xmoq", "meet", "blue", "x"],
		namespace: undefined,
	},
	{
		name: "not for the meetings",
		asked: ["xmoq", "meet"],
		namespace: undefined,
	},
])("token-for: $name", ({ asked, namespace }) => {
	expect(tokenForNamespace(asked)).toEqual(namespace);
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
		answer: { type: "unanswered" },
	},
	{
		name: "not for an id over 64",
		id: "x".repeat(65),
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not for an id with a /",
		id: "a/b",
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not for an id *",
		id: "*",
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not for an id that is not text",
		id: 7,
		room: "blue",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not for another id on a socket that joined",
		id: "other",
		room: "blue",
		mine: "ann",
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not for an id another socket has",
		id: "bob",
		room: "blue",
		mine: undefined,
		others: ["bob"],
		answer: { type: "unanswered" },
	},
	{
		name: "not in a room whose name has a /",
		id: "ann",
		room: "a/b",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
	},
	{
		name: "not in a room with no name",
		id: "ann",
		room: "",
		mine: undefined,
		others: [],
		answer: { type: "unanswered" },
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
