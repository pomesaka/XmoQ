// A try, for the open decision on how chat travels: the same lines sent
// over the room's WebSocket, as the demo does, and as a MoQ track through
// the relay, side by side. Not part of the demo's pages: nothing links to
// it, and a script drives it (`window.chat`).
//
//   /chat?room=R&name=a&peers=a,b,c
//
// **The shape over MoQ**: every participant publishes one track, `chat`,
// in a namespace of its own, `xmoq/chat/<room>/<name>`. A line is an
// Object, JSON, and all lines of a participant's session are one Group on
// one stream that stays open. The others subscribe to that track, with a
// Joining FETCH from the start of the Group, which brings what was said
// before they came from the relay's cache of the latest Group.
//
// WHO is in the room is given in the address (`peers`): the relay has no
// SUBSCRIBE_NAMESPACE, by which a participant would find the others.

import {
	decodeFetchBody,
	RequestErrorCode,
	type SubgroupWriter,
} from "@xmoq/moqt";
import { connectToRelay, HERE } from "./shared.ts";

declare const XMOQ_DEMO: { roomsUrl: string | null };

type Transport = "websocket" | "moq";
interface Seen {
	transport: Transport;
	id: string;
	from: string;
	/** When it was sent and when it came, by clocks of the one machine, in milliseconds. */
	sentAt: number;
	at: number;
	/** "live", or what brought a line from before this page came: the room's "history", the "fetch" of MoQ. */
	how: "live" | "history" | "fetch";
}

const query = new URLSearchParams(location.search);
const room = query.get("room") ?? "chat";
const name = query.get("name") ?? "a";
const peers = (query.get("peers") ?? name).split(",");
const now = (): number => performance.timeOrigin + performance.now();
const utf8 = new TextEncoder();
const text = new TextDecoder();
const log: Seen[] = [];
const say = (line: string): void => {
	const status = document.getElementById("status");
	if (status !== null) status.textContent = line;
};

const namespaceOf = (who: string): Uint8Array[] =>
	["xmoq", "chat", room, who].map((field) => utf8.encode(field));
const TRACK = utf8.encode("chat");

async function main(): Promise<void> {
	// The room's WebSocket, as the demo's pages use it.
	const roomsUrl = XMOQ_DEMO.roomsUrl;
	if (roomsUrl === null) throw new Error("no rooms run");
	const socket = new WebSocket(
		`${roomsUrl.replace("http", "ws")}rooms/${encodeURIComponent(room)}/ws?name=${encodeURIComponent(name)}`,
	);
	const noteSocket = (
		chat: { from: string; text: string },
		how: Seen["how"],
	): void => {
		const said: { id: string; sentAt: number } = JSON.parse(chat.text);
		log.push({ transport: "websocket", from: chat.from, at: now(), how, ...said });
	};
	const socketOpen = new Promise<void>((resolve) => {
		socket.addEventListener("message", (event) => {
			const message = JSON.parse(String(event.data));
			if (message.type === "hello") {
				for (const chat of message.history) noteSocket(chat, "history");
				resolve();
			}
			if (message.type === "chat") noteSocket(message, "live");
		});
	});

	// MoQ: this page's own track.
	const connection = await connectToRelay(HERE);
	let stream: SubgroupWriter | undefined;
	let nextObject = 0n;
	let writes: Promise<void> = Promise.resolve();
	// A session of this page is one Group; its number is the time, so that a page that comes again is above where it was.
	const group = BigInt(Date.now());
	connection.serve({
		subscribe(request) {
			if (text.decode(request.trackName) !== "chat" || stream !== undefined) {
				request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				return;
			}
			void request.accept().then(async (track) => {
				stream = await track.openSubgroup({
					groupId: group,
					subgroupId: { mode: "Zero" },
					properties: false,
					endOfGroup: false,
				});
			});
		},
	});
	await connection.publishNamespace(namespaceOf(name));

	// The tracks of everybody in the room, this page's own among them: a
	// line is shown when it comes back from the relay, as with the room.
	const seenOf = new Set<string>();
	const noteMoq = (from: string, payload: Uint8Array, how: Seen["how"]): void => {
		const said: { id: string; sentAt: number } = JSON.parse(text.decode(payload));
		if (seenOf.has(said.id)) return;
		seenOf.add(said.id);
		log.push({ transport: "moq", from, at: now(), how, ...said });
	};
	const follow = async (who: string): Promise<void> => {
		// The track is not there before its participant is: asked again.
		for (;;) {
			try {
				const subscription = await connection.subscribe({
					trackNamespace: namespaceOf(who),
					trackName: TRACK,
				});
				void (async () => {
					const fetch = await connection.fetch({
						range: {
							fetchType: "RelativeJoining",
							joiningRequestId: subscription.requestId,
							joiningStart: 0n,
						},
					});
					fetch.response.catch(() => {});
					const reader = decodeFetchBody(connection, fetch).getReader();
					for (;;) {
						const next = await reader.read();
						if (next.done) break;
						if (next.value.type === "object") {
							noteMoq(who, next.value.object.payload, "fetch");
						}
					}
				})().catch(() => {});
				void (async () => {
					const reader = subscription.objects.getReader();
					for (;;) {
						const next = await reader.read();
						if (next.done) break;
						noteMoq(who, next.value.object.payload, "live");
					}
				})();
				return;
			} catch {
				await new Promise((resolve) => setTimeout(resolve, 200));
			}
		}
	};
	/** Whose track this page has subscribed to. */
	const followed: string[] = [];
	for (const who of peers) void follow(who).then(() => followed.push(who));

	Object.assign(window, {
		chat: {
			log,
			followed,
			/** Resolves when the room has said hello and the relay has subscribed to this page's track. */
			async ready(): Promise<void> {
				await socketOpen;
				while (stream === undefined) {
					await new Promise((resolve) => setTimeout(resolve, 20));
				}
			},
			send(transport: Transport, id: string): void {
				const line = JSON.stringify({ id, sentAt: now() });
				if (transport === "websocket") {
					socket.send(JSON.stringify({ type: "chat", text: line }));
					return;
				}
				const to = stream;
				if (to === undefined) return;
				const objectId = nextObject;
				nextObject += 1n;
				writes = writes
					.then(() => to.write({ objectId, payload: utf8.encode(line) }))
					.catch((error: unknown) => console.log(`not written: ${String(error)}`));
			},
		},
	});
	say(`chat try: ${name} in ${room} with ${peers.join(", ")}`);
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
