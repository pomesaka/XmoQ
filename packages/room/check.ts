// Checks the room Worker that run.ts serves: the broadcast of a room, a
// token, the count, that it carries no chat, and the seats of a meeting
// of 49.
//
// Usage: node check.ts [http://127.0.0.1:8787], with the AUTH_KEY_HEX,
// RELAY_URL, RELAY_CERT_SHA256_HEX, TRANSCRIPT_PREFIX and
// MEETING_TRANSCRIPT_PREFIX run.ts was given, if any.

import {
	leaveSeat,
	meetingRoom,
	renewSeat,
	type Seat,
	takeSeat,
} from "@xmoq/room-client";

const origin = process.argv[2] ?? "http://127.0.0.1:8787";
const room = `check-${Date.now()}`;
const say = (what: string, value: unknown): void =>
	console.log(what, JSON.stringify(value));
const fail = (what: string): void => {
	console.log(`FAILED: ${what}`);
	process.exitCode = 1;
};

/** The value at `path` in what the room said, or undefined. */
function at(value: unknown, ...path: (string | number)[]): unknown {
	let here = value;
	for (const key of path) {
		if (typeof here !== "object" || here === null) return undefined;
		here = Object.getOwnPropertyDescriptor(here, key)?.value;
	}
	return here;
}
const same = (a: unknown, b: unknown): boolean =>
	JSON.stringify(a) === JSON.stringify(b);
const textOf = (value: unknown): string =>
	typeof value === "string" ? value : "";

/** A client of the room's WebSocket that keeps what it was sent. */
function join(name = room): {
	socket: WebSocket;
	got: unknown[];
	opened: Promise<unknown>;
} {
	const got: unknown[] = [];
	const socket = new WebSocket(
		`${origin.replace("http", "ws")}/rooms/${encodeURIComponent(name)}/ws`,
	);
	socket.addEventListener("message", (event) =>
		got.push(JSON.parse(String(event.data))),
	);
	const opened = new Promise((resolve, reject) => {
		socket.addEventListener("open", resolve);
		socket.addEventListener("error", reject);
	});
	return { socket, got, opened };
}
const wait = (ms: number): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, ms));
const ofType = (got: unknown[], ...types: string[]): unknown[] =>
	got.filter((message) => types.includes(textOf(at(message, "type"))));
const masked = (token: unknown): unknown =>
	typeof token === "string" ? token.replace(/[0-9a-f]{64}$/, "<mac>") : token;
const roomNow = async (name = room): Promise<unknown> =>
	(await fetch(`${origin}/rooms/${name}`)).json();

// 1. The broadcast of a room: there while its publisher's socket is.
say("before:", await roomNow());
const broadcast = {
	relayUrl: "https://127.0.0.1:4443/",
	namespace: ["xmoq", "demo"],
	certSha256Hex: "ab".repeat(32),
};
const publisher = join();
await publisher.opened;
publisher.socket.send(JSON.stringify({ type: "broadcast", broadcast }));
await wait(300);
const found = await roomNow();
say("after the publisher registered:", found);
if (
	at(found, "broadcast", "relayUrl") !== broadcast.relayUrl ||
	!same(at(found, "broadcast", "namespace"), ["xmoq", "demo"])
) {
	fail("the room does not give the broadcast back");
}
publisher.socket.close();
await wait(500);
const gone = await roomNow();
say("after the publisher's socket closed:", gone);
if (at(gone, "broadcast") !== null || at(gone, "connected") !== 0)
	fail("the broadcast outlived its publisher");

// 1b. Tokens, where the rooms have a key: the text the relay checks, in the
// namespaces of the room alone, and an answer to every ask, with token null
// where there is none, so that a reader that goes by the order goes on.
{
	const asker = join();
	await asker.opened;
	const broadcasts = (process.env.BROADCAST_NAMESPACES ?? "").split(",");
	const asks: { role: "pub" | "sub"; namespace: string[]; given: boolean }[] = [
		{ role: "sub", namespace: ["xmoq", "chat", room], given: true },
		{ role: "pub", namespace: ["xmoq", "chat", room, "ann"], given: true },
		{
			role: "sub",
			namespace: ["xmoq", "demo"],
			given: broadcasts.includes("xmoq/demo"),
		},
		{ role: "sub", namespace: ["xmoq", "*"], given: false },
		{ role: "sub", namespace: ["xmoq/demo"], given: false },
		// A meeting's namespaces are join-meeting's, where the room counts.
		{ role: "pub", namespace: ["xmoq", "meet", room, "x"], given: false },
		// None above the room, which a SUBSCRIBE_NAMESPACE would list every
		// room under, and none of another room.
		{ role: "sub", namespace: ["xmoq"], given: false },
		{ role: "sub", namespace: ["xmoq", "chat"], given: false },
		{ role: "sub", namespace: ["xmoq", "chat", `${room}-other`], given: false },
	];
	for (const { role, namespace } of asks) {
		asker.socket.send(JSON.stringify({ type: "token-for", role, namespace }));
	}
	await wait(500);
	const answers = ofType(asker.got, "token");
	say(
		"the tokens:",
		answers.map((answer) => [
			at(answer, "namespace"),
			masked(at(answer, "token")),
		]),
	);
	if (answers.length !== asks.length)
		fail(`${answers.length} answers to ${asks.length} asks for a token`);
	asks.forEach(({ role, namespace, given }, index) => {
		const answer = answers[index];
		const token = at(answer, "token");
		const what = `${role} in ${namespace.join("/")}`;
		if (!same(at(answer, "namespace"), namespace))
			fail(
				`the answer to ${what} names ${JSON.stringify(at(answer, "namespace"))}`,
			);
		if (!given || !process.env.AUTH_KEY_HEX) {
			if (token !== null) fail(`a token was made for ${what}`);
			return;
		}
		const form = new RegExp(
			`^v1\\|${namespace.join("/").replaceAll("*", "\\*")}\\|${role}\\|\\d+\\|[0-9a-f]{64}$`,
		);
		if (!form.test(textOf(token)))
			fail(
				`the token for ${what} has not the form v1|namespace|role|expiry|mac`,
			);
	});
	asker.socket.close();
	await wait(200);
}

// 2. No chat (it is on MoQ tracks): a {type: "chat"} message goes to
// nobody, and one who comes later is told no history.
const a = join();
const b = join();
await Promise.all([a.opened, b.opened]);
a.socket.send(JSON.stringify({ type: "chat", text: "hello" }));
await wait(500);
const late = join();
await late.opened;
await wait(300);
const carried = [a, b, late].flatMap((client) => ofType(client.got, "chat"));
say("chat messages anybody got:", carried.length);
if (carried.length !== 0) fail("the room carried a chat message");
const hello = ofType(late.got, "hello")[0];
say("the late one's hello:", hello);
if (hello === undefined || at(hello, "history") !== undefined)
	fail("the late one's hello is missing or has a history");

// 2b. The relay of the rooms, in the hello, with no broadcast: as run.ts
// was told (RELAY_URL, RELAY_CERT_SHA256_HEX), else null.
{
	const told =
		process.env.RELAY_URL && process.env.RELAY_CERT_SHA256_HEX
			? {
					relayUrl: process.env.RELAY_URL,
					certSha256Hex: process.env.RELAY_CERT_SHA256_HEX,
				}
			: null;
	say("the relay in the late one's hello:", at(hello, "relay"));
	if (!same(at(hello, "relay"), told))
		fail(
			`the hello names the relay ${JSON.stringify(at(hello, "relay"))}, not ${JSON.stringify(told)}`,
		);
	if (at(hello, "broadcast") !== null)
		fail("the late one's hello has a broadcast, where nobody broadcasts");
	// The transcripts' prefixes, of the broadcast and of the meeting, with the
	// room's name put in for {room}.
	for (const [key, setting] of [
		["transcript", process.env.TRANSCRIPT_PREFIX],
		["meetingTranscript", process.env.MEETING_TRANSCRIPT_PREFIX],
	] satisfies [string, string | undefined][]) {
		const prefix = setting
			? setting.replaceAll("{room}", room).split("/")
			: null;
		say(`the ${key} in the late one's hello:`, at(hello, key));
		if (!same(at(hello, key), prefix))
			fail(
				`the hello names the ${key} ${JSON.stringify(at(hello, key))}, not ${JSON.stringify(prefix)}`,
			);
	}
}

// 3. The count: the late one is told that 3 are connected.
if (at(hello, "connected") !== 3)
	fail("the late one is not told that 3 are connected");
const counts = (got: unknown[]): unknown[] =>
	ofType(got, "connected").map((message) => at(message, "connected"));
say("the count, as a was told:", counts(a.got));

// 4. One leaves: the others are told, and the room says 2.
b.socket.close();
await wait(500);
const after = await roomNow();
say("after b left:", {
	connected: at(after, "connected"),
	aWasTold: counts(a.got).at(-1),
});
if (at(after, "connected") !== 2)
	fail("the room does not say 2 after one left");
if (counts(a.got).at(-1) !== 2) fail("a was not told that 2 are left");
a.socket.close();
late.socket.close();
await wait(200);

// 5. A meeting, over HTTP with `@xmoq/room-client` (so that what the room
// answers is what the client reads): 49 get a seat with its two namespaces,
// the 50th is told it is full, and gets in when one leaves; refusals say why;
// a seat is renewed with its lease, and not with another's.
{
	const meeting = { rooms: `${origin}/`, name: `${room}-meeting` };
	const described = await meetingRoom(meeting);
	say("the meeting:", described);
	if (
		!same(described.namespace, ["xmoq", "meet", meeting.name]) ||
		described.most !== 49
	) {
		fail("the meeting does not say its namespace and its 49 seats");
	}
	const relayTold =
		process.env.RELAY_URL && process.env.RELAY_CERT_SHA256_HEX
			? {
					url: process.env.RELAY_URL,
					certSha256Hex: process.env.RELAY_CERT_SHA256_HEX,
				}
			: null;
	if (!same(described.relay, relayTold)) {
		fail("the meeting does not name the relay it was told");
	}
	const seats: Seat[] = [];
	for (let index = 0; index < 49; index++) {
		const answer = await takeSeat(meeting, `p${index}`);
		if (answer.type === "seat") seats.push(answer.seat);
	}
	const [first] = seats;
	if (seats.length !== 49 || first === undefined) {
		fail("not each of 49 got its seat");
	} else {
		say("the first one's seat:", {
			...first,
			publishToken: masked(first.publishToken),
			subscribeToken: masked(first.subscribeToken),
			lease: "<lease>",
		});
		const tokens = process.env.AUTH_KEY_HEX
			? /^v1\|xmoq\/meet\/[^|]+\/p0\|pub\|\d+\|[0-9a-f]{64}$/.test(
					textOf(first.publishToken),
				) &&
				/^v1\|xmoq\/meet\/[^|]+\/\*\|sub\|\d+\|[0-9a-f]{64}$/.test(
					textOf(first.subscribeToken),
				)
			: first.publishToken === null && first.subscribeToken === null;
		if (!tokens) fail("the meeting's tokens are not as the key says");
		if (first.renewMs !== 20_000 || first.expiresAt - Date.now() > 60_000) {
			fail("a seat does not hold 60 s and is not renewed every 20 s");
		}
	}
	const fiftieth = await takeSeat(meeting, "p49");
	say("the 50th one's answer:", fiftieth);
	if (!same(fiftieth, { type: "full", most: 49 })) {
		fail("the 50th was not told the meeting is full");
	}
	const refusals: [unknown, unknown][] = [];
	for (const id of ["", "x".repeat(65), "a/b", "*", "p1"]) {
		refusals.push([id, await takeSeat(meeting, id)]);
	}
	say("the refusals:", refusals);
	if (
		!same(
			refusals.map(([, answer]) => at(answer, "why")),
			[
				"not-a-participant",
				"not-a-participant",
				"not-a-participant",
				"not-a-participant",
				"taken",
			],
		)
	) {
		fail("one of those to be refused was not refused, or not for its reason");
	}
	if (first !== undefined) {
		const renewed = await renewSeat(meeting, first);
		if (renewed.type !== "seat" || renewed.seat.expiresAt <= first.expiresAt) {
			fail("a seat was not renewed with its lease");
		}
		const stolen = await renewSeat(meeting, { ...first, lease: "another" });
		if (stolen.type !== "gone") fail("a seat was renewed with another's lease");
		// One goes: the 50th gets in.
		await leaveSeat(meeting, first);
		const after = await takeSeat(meeting, "p49");
		say("the 50th one's answer after one left:", after.type);
		if (after.type !== "seat") fail("the 50th did not get in after one left");
	}
	// A room whose name has a `*` makes no namespace of a meeting.
	const starred = await takeSeat({ ...meeting, name: "a*b" }, "p0");
	if (!same(starred, { type: "refused", why: "not-a-meeting-room" })) {
		fail("a room whose name has a * did not refuse a seat");
	}
}
console.log(process.exitCode ? "SOME CHECKS FAILED" : "ALL CHECKS PASSED");
