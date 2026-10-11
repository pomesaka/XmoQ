// Checks the room Worker that run.mjs serves: the broadcast of a room, the
// chat (order, history for one who comes later), and the count.
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
function join(name) {
	const got = [];
	const socket = new WebSocket(`${origin.replace("http", "ws")}/rooms/${room}/ws?name=${name}`);
	socket.addEventListener("message", (event) => got.push(JSON.parse(event.data)));
	const opened = new Promise((resolve, reject) => {
		socket.addEventListener("open", resolve);
		socket.addEventListener("error", reject);
	});
	return { socket, got, opened };
}
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// 1. The broadcast of a room.
say("before:", await (await fetch(`${origin}/rooms/${room}`)).json());
const broadcast = { relayUrl: "https://127.0.0.1:4443/", namespace: ["xmoq", "demo"], certSha256Hex: "ab".repeat(32) };
await fetch(`${origin}/rooms/${room}/broadcast`, { method: "PUT", body: JSON.stringify(broadcast), headers: { "content-type": "application/json" } });
const found = await (await fetch(`${origin}/rooms/${room}`)).json();
say("after the publisher registered:", found);
if (found.broadcast?.relayUrl !== broadcast.relayUrl || found.broadcast?.namespace?.join("/") !== "xmoq/demo") fail("the room does not give the broadcast back");

// 2. Chat: two send at once, 20 messages each; both see one order.
const a = join("a");
const b = join("b");
await Promise.all([a.opened, b.opened]);
for (let index = 0; index < 20; index++) {
	a.socket.send(JSON.stringify({ type: "chat", text: `a${index}` }));
	b.socket.send(JSON.stringify({ type: "chat", text: `b${index}` }));
}
await wait(1000);
const chats = (client) => client.got.filter((message) => message.type === "chat");
const order = (client) => chats(client).map((chat) => `${chat.seq}:${chat.text}`).join(" ");
say("a saw:", order(a).slice(0, 120));
if (chats(a).length !== 40 || order(a) !== order(b)) fail("the two do not see the same 40 messages in the same order");
if (chats(a).some((chat, index) => chat.seq !== index + 1)) fail("the numbers are not 1 to 40 without a gap");
for (const name of ["a", "b"]) {
	const own = chats(a).filter((chat) => chat.from === name).map((chat) => chat.text).join(" ");
	if (own !== Array.from({ length: 20 }, (_unused, index) => `${name}${index}`).join(" ")) fail(`the messages of ${name} are not in the order it sent them`);
}

// 3. One who comes later gets the last 50 at most, and the count.
for (let index = 0; index < 30; index++) a.socket.send(JSON.stringify({ type: "chat", text: `more${index}` }));
await wait(500);
const late = join("late");
await late.opened;
await wait(300);
const hello = late.got.find((message) => message.type === "hello");
say("the late one's hello:", { history: hello?.history?.length, first: hello?.history?.[0]?.seq, last: hello?.history?.at(-1)?.seq, connected: hello?.connected, broadcast: hello?.broadcast?.relayUrl });
if (hello?.history?.length !== 50 || hello.history[0].seq !== 21 || hello.history.at(-1).seq !== 70) fail("the history is not the last 50 messages, 21 to 70");
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
