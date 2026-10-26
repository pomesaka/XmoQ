// Checks the room Worker that run.ts serves: the broadcast of a room, a
// token, the count, that it carries no chat, and a meeting of 49.
//
// Usage: node check.ts [http://127.0.0.1:8787], with the AUTH_KEY_HEX,
// RELAY_URL, RELAY_CERT_SHA256_HEX, TRANSCRIPT_PREFIX and
// MEETING_TRANSCRIPT_PREFIX run.ts was given, if any.

// A module, for the `await`s at its top level.
export {};

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

// 1b. A token, where the rooms have a key: the text the relay checks.
{
	const asker = join();
	await asker.opened;
	const tokens = (): number => ofType(asker.got, "token").length;
	asker.socket.send(
		JSON.stringify({
			type: "token-for",
			role: "sub",
			namespace: ["xmoq", "demo"],
		}),
	);
	await wait(300);
	const answer = ofType(asker.got, "token")[0];
	const token = at(answer, "token");
	say(
		"a token for sub in xmoq/demo:",
		token === null ? "none (the rooms have no key)" : masked(token),
	);
	if (answer === undefined) fail("no answer to token-for");
	if (
		process.env.AUTH_KEY_HEX &&
		!/^v1\|xmoq\/demo\|sub\|\d+\|[0-9a-f]{64}$/.test(textOf(token))
	) {
		fail("the token has not the form v1|namespace|role|expiry|mac");
	}
	asker.socket.send(
		JSON.stringify({
			type: "token-for",
			role: "sub",
			namespace: ["xmoq", "*"],
		}),
	);
	await wait(300);
	if (tokens() !== 1) fail("a token was made for a namespace with a field *");
	asker.socket.send(
		JSON.stringify({
			type: "token-for",
			role: "sub",
			namespace: ["xmoq/demo"],
		}),
	);
	await wait(300);
	if (tokens() !== 1)
		fail("a token was made for a namespace with a / in a field");
	// A meeting's namespaces are join-meeting's, where the room counts.
	asker.socket.send(
		JSON.stringify({
			type: "token-for",
			role: "pub",
			namespace: ["xmoq", "meet", room, "x"],
		}),
	);
	await wait(300);
	if (tokens() !== 1) fail("token-for made a token in a meeting's namespace");
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

// 5. A meeting: 49 who joined get the two namespaces, the 50th is told it
// is full, and gets in when one of them has gone.
{
	const meeting = `${room}-meeting`;
	type Client = ReturnType<typeof join>;
	const answers = (client: Client): unknown[] =>
		ofType(client.got, "meeting", "meeting-full");
	const ask = (client: Client, participant: unknown): void =>
		client.socket.send(JSON.stringify({ type: "join-meeting", participant }));
	const in49 = Array.from({ length: 49 }, () => join(meeting));
	await Promise.all(in49.map((client) => client.opened));
	in49.forEach((client, index) => {
		ask(client, `p${index}`);
	});
	await wait(1000);
	const [p0, p1, , p3] = in49;
	if (p0 === undefined || p1 === undefined || p3 === undefined)
		throw new Error("49 were not made");
	const first = answers(p0)[0];
	say("the first one's answer:", {
		...(typeof first === "object" ? first : {}),
		publish: {
			namespace: at(first, "publish", "namespace"),
			token: masked(at(first, "publish", "token")),
		},
		subscribe: {
			namespace: at(first, "subscribe", "namespace"),
			token: masked(at(first, "subscribe", "token")),
		},
	});
	if (
		in49.some((client, index) => {
			const got = answers(client);
			return (
				got.length !== 1 ||
				at(got[0], "type") !== "meeting" ||
				at(got[0], "participant") !== `p${index}`
			);
		})
	) {
		fail("not each of 49 got its meeting");
	}
	if (!same(at(first, "publish", "namespace"), ["xmoq", "meet", meeting, "p0"]))
		fail("the publish namespace is not xmoq/meet/<room>/<id>");
	if (
		!same(at(first, "subscribe", "namespace"), ["xmoq", "meet", meeting, "*"])
	)
		fail("the subscribe namespace is not xmoq/meet/<room>/*");
	const tokens = process.env.AUTH_KEY_HEX
		? /^v1\|xmoq\/meet\/[^|]+\/p0\|pub\|\d+\|[0-9a-f]{64}$/.test(
				textOf(at(first, "publish", "token")),
			) &&
			/^v1\|xmoq\/meet\/[^|]+\/\*\|sub\|\d+\|[0-9a-f]{64}$/.test(
				textOf(at(first, "subscribe", "token")),
			)
		: at(first, "publish", "token") === null &&
			at(first, "subscribe", "token") === null;
	if (!tokens) fail("the meeting's tokens are not as the key says");

	const fiftieth = join(meeting);
	await fiftieth.opened;
	ask(fiftieth, "p49");
	await wait(300);
	say("the 50th one's answer:", answers(fiftieth));
	if (!same(answers(fiftieth), [{ type: "meeting-full", most: 49 }]))
		fail("the 50th was not told the meeting is full");

	// Not answered: what cannot be an id, and another id on a socket that
	// joined.
	const unanswered: [Client, unknown][] = [
		[fiftieth, ""],
		[fiftieth, "x".repeat(65)],
		[fiftieth, "a/b"],
		[fiftieth, "*"],
		[fiftieth, 7],
		[p1, "another"],
	];
	for (const [client, participant] of unanswered) ask(client, participant);
	await wait(500);
	if (answers(fiftieth).length !== 1 || answers(p1).length !== 1)
		fail("one of those not to be answered was answered");

	// One that joined, asking again while it is full: its meeting again.
	ask(p3, "p3");
	await wait(300);
	if (
		at(answers(p3).at(-1), "type") !== "meeting" ||
		answers(p3).length !== 2
	) {
		fail("one in the meeting was not answered again while it was full");
	}

	// One goes: the 50th gets in, and its id with it.
	p0.socket.close();
	await wait(500);
	// With room for it, an id another open socket has is still not answered.
	ask(fiftieth, "p2");
	await wait(300);
	if (answers(fiftieth).length !== 1)
		fail("an id another socket has was answered");
	ask(fiftieth, "p49");
	await wait(300);
	const last = answers(fiftieth).at(-1);
	say("the 50th one's answer after one went:", at(last, "type"));
	if (at(last, "type") !== "meeting" || at(last, "participant") !== "p49")
		fail("the 50th did not get in after one went");
	for (const client of [...in49, fiftieth]) client.socket.close();

	// A room whose name has a `/` makes no namespace of a meeting.
	const slashed = join("a/b");
	await slashed.opened;
	ask(slashed, "p0");
	await wait(300);
	if (answers(slashed).length !== 0)
		fail("a room whose name has a / answered join-meeting");
	slashed.socket.close();
	await wait(200);
}
console.log(process.exitCode ? "SOME CHECKS FAILED" : "ALL CHECKS PASSED");
