// Checks the room Worker that run.mjs serves: the broadcast of a room, a
// token, the count, and that it carries no chat.
//
// Usage: node check.mjs [http://127.0.0.1:8787]
const origin = process.argv[2] ?? "http://127.0.0.1:8787";
const room = `check-${Date.now()}`;
const say = (what, value) => console.log(what, JSON.stringify(value));
const fail = (what) => {
	console.log(`FAILED: ${what}`);
	process.exitCode = 1;
};

/** A client of the room's WebSocket that keeps what it was sent. */
function join() {
	const got = [];
	const socket = new WebSocket(`${origin.replace("http", "ws")}/rooms/${room}/ws`);
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
console.log(process.exitCode ? "SOME CHECKS FAILED" : "ALL CHECKS PASSED");
