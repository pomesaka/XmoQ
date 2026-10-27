// The room of a broadcast, as the pages of the broadcast go by it: where
// its relay and its one broadcast are, tokens, and the chat of the room.

import { type ChatRoom, type Reactions, roomFields } from "@xmoq/chat";
import { DEFAULT_RECONNECT_DELAY_MS, waitFor } from "@xmoq/moqt";
import {
	type BroadcastRoom,
	broadcastRoom,
	tokenOfARoom,
} from "@xmoq/room-client";
import { startChat } from "../lib/chat.ts";
import { AGAIN } from "../lib/page.ts";
import { type Broadcast, connectToRelay, tokenOf } from "../lib/relay.ts";

/**
 * How many lines of the chat a page shows; the oldest go. The participant
 * catches up as many by default (`CATCH_UP_LINES` of `@xmoq/chat`). Not a
 * flag: what a page holds is bounded whatever the address says.
 */
const SHOWN_LINES = 200;

/** How long a reaction stands on the page. */
const REACTION_SHOWN_MS = 2_000;
/** The most reactions that stand at once: how many come in two seconds is the room's to decide. */
const MOST_SHOWN_REACTIONS = 50;

/** The reactions shown since the page opened. */
let reacted = 0;

/**
 * Shows the reactions of a while in `#reactions`, each emoji with how many
 * (`👍×12`), for `REACTION_SHOWN_MS`, and counts them for a script that
 * looks at the page (`data-reactions`, all that were shown).
 */
function showReactions(reactions: Reactions): void {
	const shown = document.getElementById("reactions");
	for (const [emoji, count] of reactions.counts) {
		reacted += count;
		if (shown === null) continue;
		const one = document.createElement("span");
		one.textContent = count === 1 ? emoji : `${emoji}×${count}`;
		shown.append(one);
		while (shown.childElementCount > MOST_SHOWN_REACTIONS) {
			shown.firstElementChild?.remove();
		}
		setTimeout(() => one.remove(), REACTION_SHOWN_MS);
	}
	document.body.dataset.reactions = String(reacted);
}

/** The relay, and the rooms where they run, as `serve.ts` wrote them into the page. */
declare const XMOQ_DEMO: {
	relayUrl: string;
	certSha256Hex: string;
	roomsUrl: string | null;
};
/** The room of the page, `?room=NAME`: "demo" when left out. */
export const ROOM_NAME =
	new URLSearchParams(location.search).get("room") ?? "demo";
/**
 * The broadcast of the page's room where no rooms run: the relay this page
 * was served with, and the namespace a room of that name names
 * (`xmoq/broadcast/<name>`, `packages/room/src/worker.ts`).
 */
export const HERE: Broadcast = {
	relayUrl: XMOQ_DEMO.relayUrl,
	certSha256Hex: XMOQ_DEMO.certSha256Hex,
	namespace: ["xmoq", "broadcast", ROOM_NAME],
};

/** The room of a broadcast, as the page goes by it. */
export interface Room {
	/** Where the room's one broadcast is, whether anybody publishes there yet or not. */
	broadcast: Broadcast;
	/**
	 * The prefix a transcriber listens under for the broadcast, the fields
	 * of `xmoq/transcript/<prefix>` the room gives a token in; null where
	 * the room names none. WHY from the room: it gives tokens in its own
	 * prefix alone (`tokenForNamespace` of `packages/room/src/core/rules.ts`).
	 */
	transcript: string[] | null;
	/**
	 * Asks the room for a token for the relay (an experiment in
	 * authorization): the Token structure as `@xmoq/moqt` carries it, or
	 * undefined where the rooms give none, or where the room could not be
	 * asked (the relay then refuses, and the page connects again).
	 * `?token=no` asks for none, to see the relay refuse.
	 */
	token(
		role: "pub" | "sub",
		namespace: string[],
	): Promise<Uint8Array | undefined>;
}

/**
 * Asks the room `?room=NAME` of the rooms `serve.ts` started where its
 * relay and its broadcast are, and starts the chat of that room (over MoQ
 * tracks). Where no rooms run, the broadcast is `HERE`, and no token is
 * given. A room that does not answer is asked again after `AGAIN`, and the
 * line `#room` says so: the page has nothing to play or publish without
 * it. Nothing more is asked of the room afterwards: what happens in it
 * (a broadcast that begins) comes over MoQ (decided on 2026-10-26).
 */
export async function joinRoom(as: string): Promise<Room> {
	const line = document.getElementById("room");
	const roomsUrl = XMOQ_DEMO.roomsUrl;
	if (roomsUrl === null) {
		return {
			broadcast: HERE,
			transcript: HERE.namespace,
			token: async () => undefined,
		};
	}
	const address = { rooms: roomsUrl, name: ROOM_NAME };
	let described: BroadcastRoom | undefined;
	while (described === undefined) {
		try {
			described = await broadcastRoom(address);
		} catch (error) {
			if (line !== null) {
				line.textContent = `room “${ROOM_NAME}”: ${String(error).slice(0, 120)}; asking again every ${AGAIN}`;
			}
			await waitFor(DEFAULT_RECONNECT_DELAY_MS, new AbortController().signal);
		}
	}
	if (line !== null) line.textContent = `room “${ROOM_NAME}”`;
	const relay = described.relay ?? {
		url: HERE.relayUrl,
		certSha256Hex: HERE.certSha256Hex,
	};
	const broadcast: Broadcast = {
		relayUrl: relay.url,
		certSha256Hex: relay.certSha256Hex,
		namespace: described.namespace,
	};
	const token: Room["token"] = async (role, namespace) => {
		// `?token=no`: asks the room for no token, to see the relay refuse.
		if (new URLSearchParams(location.search).get("token") === "no") {
			return undefined;
		}
		try {
			return tokenOf(
				await tokenOfARoom("broadcasts", address, role, namespace),
			);
		} catch {
			return undefined;
		}
	};
	// Two tokens each time it connects for the chat, as the participant
	// needs them (`publishNamespaceToken` of `joinChat`): `sub` for the
	// room's `lines`, which the connection carries, and `pub` for its own
	// namespace, which only its PUBLISH_NAMESPACE carries. WHY asked again
	// each time: a token runs out (10 minutes), and the relay checks the
	// SETUP of the new connection. With --no-auth the room gives none, and
	// both are undefined. After a GOAWAY the next session goes to its URI
	// with the same certificate's hash, as the broadcast's does (the relay
	// that sends it away, `--when-full`, names one with the same certificate).
	const chatRoom: ChatRoom = { kind: "broadcast", name: ROOM_NAME };
	const chat = startChat(
		chatRoom,
		as,
		async (goAway) => {
			const [sub, pub] = await Promise.all([
				token("sub", roomFields(chatRoom)),
				token("pub", [...roomFields(chatRoom), as]),
			]);
			const to =
				goAway === undefined || goAway.newSessionUri === ""
					? broadcast
					: { ...broadcast, relayUrl: goAway.newSessionUri };
			return {
				connection: await connectToRelay(to, sub),
				...(pub === undefined ? {} : { publishNamespaceToken: pub }),
			};
		},
		{
			shownLines: SHOWN_LINES,
			onReactions: showReactions,
			onPresent: (present) => {
				if (line !== null) {
					line.textContent = `room “${ROOM_NAME}”: ${present.count} in the chat`;
				}
				document.body.dataset.present = String(present.count);
			},
		},
	);
	for (const button of document.querySelectorAll<HTMLButtonElement>(
		"button[data-reaction]",
	)) {
		button.addEventListener("click", () => {
			const emoji = button.dataset.reaction ?? "";
			void chat?.react(emoji).then((why) => {
				if (why !== undefined) button.title = `not sent: ${why}`;
			});
		});
	}
	return { broadcast, transcript: described.transcript, token };
}
