// Checks the room Worker that run.mjs serves: the broadcast of a room, a
// token, the count, that it carries no chat, and a meeting of 49.
//
// Usage: node check.mjs [http://127.0.0.1:8787], with the AUTH_KEY_HEX,
// RELAY_URL, RELAY_CERT_SHA256_HEX and TRANSCRIPT_PREFIX run.mjs was given,
// if any.
const origin = process.argv[2] ?? "http://127.0.0.1:8787";
const room = `check-${Date.now()}`;
const say = (what, value) => console.log(what, JSON.stringify(value));
const fail = (what) => {
	console.log(`FAILED: ${what}`);
	process.exitCode = 1;
};

/** A client of the room's WebSocket that keeps what it was sent. */
function join(name = room) {
	const got = [];
	const socket = new WebSocket(`${origin.replace("http", "ws")}/rooms/${encodeURIComponent(name)}/ws`);
	socket.addEventListener("message", (event) => got.push(JSON.parse(event.data)));
	const opened = new Promise((resolve, reject) => {
		socket.addEventListener("open", resolve);
		socket.addEventListener("error", reject);
	});
	return { socket, got, opened };
}
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// 1. The broadcast of a room: there while its publisher's socket is.
say("before:", await (await fetch(`${origin}/rooms/${room}`)).json());
const broadcast = { relayUrl: "https://127.0.0.1:4443/", namespace: ["xmoq", "demo"], certSha256Hex: "ab".repeat(32) };
const publisher = join();
await publisher.opened;
publisher.socket.send(JSON.stringify({ type: "broadcast", broadcast }));
await wait(300);
const found = await (await fetch(`${origin}/rooms/${room}`)).json();
say("after the publisher registered:", found);
if (found.broadcast?.relayUrl !== broadcast.relayUrl || found.broadcast?.namespace?.join("/") !== "xmoq/demo") fail("the room does not give the broadcast back");
publisher.socket.close();
await wait(500);
const gone = await (await fetch(`${origin}/rooms/${room}`)).json();
say("after the publisher's socket closed:", gone);
if (gone.broadcast !== null || gone.connected !== 0) fail("the broadcast outlived its publisher");

// 1b. A token, where the rooms have a key: the text the relay checks.
{
	const asker = join();
	await asker.opened;
	asker.socket.send(JSON.stringify({ type: "token-for", role: "sub", namespace: ["xmoq", "demo"] }));
	await wait(300);
	const answer = asker.got.find((message) => message.type === "token");
	say("a token for sub in xmoq/demo:", answer?.token === null ? "none (the rooms have no key)" : String(answer?.token).replace(/[0-9a-f]{64}$/, "<mac>"));
	if (answer === undefined) fail("no answer to token-for");
	if (process.env.AUTH_KEY_HEX && !/^v1\|xmoq\/demo\|sub\|\d+\|[0-9a-f]{64}$/.test(answer?.token ?? "")) fail("the token has not the form v1|namespace|role|expiry|mac");
	asker.socket.send(JSON.stringify({ type: "token-for", role: "sub", namespace: ["xmoq", "*"] }));
	await wait(300);
	if (asker.got.filter((message) => message.type === "token").length !== 1) fail("a token was made for a namespace with a field *");
	asker.socket.send(JSON.stringify({ type: "token-for", role: "sub", namespace: ["xmoq/demo"] }));
	await wait(300);
	if (asker.got.filter((message) => message.type === "token").length !== 1) fail("a token was made for a namespace with a / in a field");
	// A meeting's namespaces are join-meeting's, where the room counts.
	asker.socket.send(JSON.stringify({ type: "token-for", role: "pub", namespace: ["xmoq", "meet", room, "x"] }));
	await wait(300);
	if (asker.got.filter((message) => message.type === "token").length !== 1) fail("token-for made a token in a meeting's namespace");
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
const carried = [a, b, late].flatMap((client) => client.got).filter((message) => message.type === "chat");
say("chat messages anybody got:", carried.length);
if (carried.length !== 0) fail("the room carried a chat message");
const hello = late.got.find((message) => message.type === "hello");
say("the late one's hello:", hello);
if (hello === undefined || "history" in hello) fail("the late one's hello is missing or has a history");

// 2b. The relay of the rooms, in the hello, with no broadcast: as run.mjs
// was told (RELAY_URL, RELAY_CERT_SHA256_HEX), else null.
{
	const told = process.env.RELAY_URL && process.env.RELAY_CERT_SHA256_HEX
		? { relayUrl: process.env.RELAY_URL, certSha256Hex: process.env.RELAY_CERT_SHA256_HEX }
		: null;
	say("the relay in the late one's hello:", hello?.relay);
	if (JSON.stringify(hello?.relay) !== JSON.stringify(told)) fail(`the hello names the relay ${JSON.stringify(hello?.relay)}, not ${JSON.stringify(told)}`);
	if (hello?.broadcast !== null) fail("the late one's hello has a broadcast, where nobody broadcasts");
	// The transcript's prefix, with the room's name put in for {room}.
	const prefix = process.env.TRANSCRIPT_PREFIX
		? process.env.TRANSCRIPT_PREFIX.replaceAll("{room}", room).split("/")
		: null;
	say("the transcript in the late one's hello:", hello?.transcript);
	if (JSON.stringify(hello?.transcript) !== JSON.stringify(prefix)) fail(`the hello names the transcript ${JSON.stringify(hello?.transcript)}, not ${JSON.stringify(prefix)}`);
}

// 3. The count: the late one is told that 3 are connected.
if (hello?.connected !== 3) fail("the late one is not told that 3 are connected");
say("the count, as a was told:", a.got.filter((message) => message.type === "connected").map((message) => message.connected));

// 4. One leaves: the others are told, and the room says 2.
b.socket.close();
await wait(500);
const after = await (await fetch(`${origin}/rooms/${room}`)).json();
say("after b left:", { connected: after.connected, aWasTold: a.got.filter((message) => message.type === "connected").at(-1)?.connected });
if (after.connected !== 2) fail("the room does not say 2 after one left");
if (a.got.filter((message) => message.type === "connected").at(-1)?.connected !== 2) fail("a was not told that 2 are left");
a.socket.close();
late.socket.close();
await wait(200);

// 5. A meeting: 49 who joined get the two namespaces, the 50th is told it
// is full, and gets in when one of them has gone.
{
	const meeting = `${room}-meeting`;
	const answers = (client) => client.got.filter((message) => message.type === "meeting" || message.type === "meeting-full");
	const ask = (client, participant) => client.socket.send(JSON.stringify({ type: "join-meeting", participant }));
	const in49 = Array.from({ length: 49 }, () => join(meeting));
	await Promise.all(in49.map((client) => client.opened));
	in49.forEach((client, index) => ask(client, `p${index}`));
	await wait(1000);
	const first = answers(in49[0])[0];
	const masked = (token) => (typeof token === "string" ? token.replace(/[0-9a-f]{64}$/, "<mac>") : token);
	say("the first one's answer:", { ...first, publish: { ...first?.publish, token: masked(first?.publish?.token) }, subscribe: { ...first?.subscribe, token: masked(first?.subscribe?.token) } });
	if (in49.some((client, index) => answers(client).length !== 1 || answers(client)[0].type !== "meeting" || answers(client)[0].participant !== `p${index}`)) fail("not each of 49 got its meeting");
	if (JSON.stringify(first?.publish?.namespace) !== JSON.stringify(["xmoq", "meet", meeting, "p0"])) fail("the publish namespace is not xmoq/meet/<room>/<id>");
	if (JSON.stringify(first?.subscribe?.namespace) !== JSON.stringify(["xmoq", "meet", meeting, "*"])) fail("the subscribe namespace is not xmoq/meet/<room>/*");
	const tokens = process.env.AUTH_KEY_HEX
		? /^v1\|xmoq\/meet\/[^|]+\/p0\|pub\|\d+\|[0-9a-f]{64}$/.test(first?.publish?.token ?? "") && /^v1\|xmoq\/meet\/[^|]+\/\*\|sub\|\d+\|[0-9a-f]{64}$/.test(first?.subscribe?.token ?? "")
		: first?.publish?.token === null && first?.subscribe?.token === null;
	if (!tokens) fail("the meeting's tokens are not as the key says");

	const fiftieth = join(meeting);
	await fiftieth.opened;
	ask(fiftieth, "p49");
	await wait(300);
	say("the 50th one's answer:", answers(fiftieth));
	if (JSON.stringify(answers(fiftieth)) !== JSON.stringify([{ type: "meeting-full", most: 49 }])) fail("the 50th was not told the meeting is full");

	// Not answered: what cannot be an id, and another id on a socket that
	// joined.
	const unanswered = [
		[fiftieth, ""],
		[fiftieth, "x".repeat(65)],
		[fiftieth, "a/b"],
		[fiftieth, "*"],
		[fiftieth, 7],
		[in49[1], "another"],
	];
	for (const [client, participant] of unanswered) ask(client, participant);
	await wait(500);
	if (answers(fiftieth).length !== 1 || answers(in49[1]).length !== 1) fail("one of those not to be answered was answered");

	// One that joined, asking again while it is full: its meeting again.
	ask(in49[3], "p3");
	await wait(300);
	if (answers(in49[3]).at(-1)?.type !== "meeting" || answers(in49[3]).length !== 2) fail("one in the meeting was not answered again while it was full");

	// One goes: the 50th gets in, and its id with it.
	in49[0].socket.close();
	await wait(500);
	// With room for it, an id another open socket has is still not answered.
	ask(fiftieth, "p2");
	await wait(300);
	if (answers(fiftieth).length !== 1) fail("an id another socket has was answered");
	ask(fiftieth, "p49");
	await wait(300);
	say("the 50th one's answer after one went:", answers(fiftieth).at(-1)?.type);
	if (answers(fiftieth).at(-1)?.type !== "meeting" || answers(fiftieth).at(-1)?.participant !== "p49") fail("the 50th did not get in after one went");
	for (const client of [...in49, fiftieth]) client.socket.close();

	// A room whose name has a `/` makes no namespace of a meeting.
	const slashed = join("a/b");
	await slashed.opened;
	ask(slashed, "p0");
	await wait(300);
	if (answers(slashed).length !== 0) fail("a room whose name has a / answered join-meeting");
	slashed.socket.close();
	await wait(200);
}
console.log(process.exitCode ? "SOME CHECKS FAILED" : "ALL CHECKS PASSED");
