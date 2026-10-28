// The page of a room's garden: the floor on the canvas, the things on it
// (the room's `garden`, which `xmoq-garden` keeps), and the pet (its body,
// `state`, which `xmoq-pet` publishes as the actor `pet`). The person acts
// on the garden, never on the pet: what to put down is chosen in the bar
// (food or the ball) and put where the floor is clicked; the pet sees it
// in the garden and goes for it, or not, as it decides (`@xmoq/pet`).
//
// `?room=NAME` is the room ("demo" when left out); `?name=NAME` the name
// the page acts under (a random one when left out; `pet` is the pet's).
// `?token=no` asks the rooms for no token, as on the pages of the broadcast.

import { DEFAULT_RECONNECT_DELAY_MS, type MoqtConnection } from "@xmoq/moqt";
import {
	type Action,
	type Garden,
	type GardenActor,
	joinGarden,
	PREFIX,
	type State,
	TALK_BYTES,
} from "@xmoq/pet";
import {
	type BroadcastRoom,
	broadcastRoom,
	tokenOfARoom,
} from "@xmoq/room-client";
import {
	actionButton,
	choiceButtons,
	controlBar,
	textField,
} from "../lib/controls.ts";
import { say } from "../lib/page.ts";
import { type Broadcast, connectToRelay, tokenOf } from "../lib/relay.ts";
import { drawGarden, floorAt, type Thing } from "./garden.ts";

declare const XMOQ_DEMO: {
	roomsUrl: string | null;
	relayUrl: string;
	certSha256Hex: string;
};

const query = new URLSearchParams(location.search);
const ROOM = query.get("room") ?? "demo";
const NAME =
	query.get("name") ??
	`guest-${Array.from(crypto.getRandomValues(new Uint8Array(2)), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("")}`;

/**
 * The fields of the room's namespace, of what is under it, and of the
 * page's own, as the room is asked for tokens in them: `*` reads the
 * garden and the pet's body under it with one token.
 */
const ROOM_FIELDS = [...PREFIX, ROOM];
const UNDER_THE_ROOM = [...ROOM_FIELDS, "*"];
const OWN_FIELDS = [...ROOM_FIELDS, NAME];

/** The newest state of the pet, its times moved onto this page's clock. */
let shown: State | undefined;
/**
 * How far this page's clock is ahead of the pet's: the least of what it
 * was at the states that came. WHY the least: a state that waited on the
 * way would make the pet late by that much, and the least is the one that
 * waited the least, as a player measures its publisher
 * (`createFastestWay` of @xmoq/media). The pet is on the same machine in
 * the demo, so it is about the time a state takes to come.
 */
let ahead: number | undefined;
/** What lies on the floor, as the garden says, and when this page first saw each (a ball flies in then). */
let things: Thing[] = [];
const firstSeen = new Map<number, number>();
/** What the person put down and the garden has not told of yet, so that it shows at once. */
let putting: Thing | undefined;
/** The tool of the bar: what a click on the floor puts down. */
let tool: "feed" | "throw" = "throw";

function onState(state: State): void {
	const now = Date.now();
	ahead = Math.min(ahead ?? Number.POSITIVE_INFINITY, now - state.at);
	shown = {
		...state,
		began: state.began + ahead,
		arrives: state.arrives + ahead,
		at: state.at + ahead,
	};
}

function onGarden(garden: Garden): void {
	const now = Date.now();
	const ids = new Set(garden.things.map((thing) => thing.id));
	// Forgotten with the thing, so that the memory of a long visit stays small.
	for (const id of firstSeen.keys()) if (!ids.has(id)) firstSeen.delete(id);
	things = garden.things.map((thing) => {
		// What the page itself just put there flies from when it was put.
		const mine =
			putting !== undefined &&
			putting.kind === thing.kind &&
			putting.at[0] === thing.at[0] &&
			putting.at[1] === thing.at[1];
		if (!firstSeen.has(thing.id)) {
			firstSeen.set(
				thing.id,
				mine && putting !== undefined ? putting.since : now,
			);
		}
		return {
			kind: thing.kind,
			at: thing.at,
			since: firstSeen.get(thing.id) ?? now,
		};
	});
	putting = undefined;
}

/** The room's broadcast: where its relay is, and how to ask it for tokens. */
async function theRoom(): Promise<{
	relay: Broadcast;
	token(
		role: "pub" | "sub",
		namespace: string[],
	): Promise<Uint8Array | undefined>;
}> {
	const here: Broadcast = {
		relayUrl: XMOQ_DEMO.relayUrl,
		certSha256Hex: XMOQ_DEMO.certSha256Hex,
		namespace: ROOM_FIELDS,
	};
	if (XMOQ_DEMO.roomsUrl === null)
		return { relay: here, token: async () => undefined };
	const address = { rooms: XMOQ_DEMO.roomsUrl, name: ROOM };
	let described: BroadcastRoom | undefined;
	while (described === undefined) {
		try {
			described = await broadcastRoom(address);
		} catch (error) {
			say(`the room: ${String(error).slice(0, 120)}; asking again`);
			await new Promise((resolve) =>
				setTimeout(resolve, DEFAULT_RECONNECT_DELAY_MS),
			);
		}
	}
	return {
		relay:
			described.relay === null
				? here
				: {
						relayUrl: described.relay.url,
						certSha256Hex: described.relay.certSha256Hex,
						namespace: ROOM_FIELDS,
					},
		async token(role, namespace) {
			if (query.get("token") === "no") return undefined;
			try {
				return tokenOf(
					await tokenOfARoom("broadcasts", address, role, namespace),
				);
			} catch {
				return undefined;
			}
		},
	};
}

/** The actor of the session there is now; undefined between sessions. */
let participant: GardenActor | undefined;

/** Joins the pet, and again after each session that ended, as long as the page is open. */
async function keepJoined(): Promise<void> {
	const room = await theRoom();
	for (;;) {
		let connection: MoqtConnection | undefined;
		try {
			const [sub, pub] = await Promise.all([
				room.token("sub", UNDER_THE_ROOM),
				room.token("pub", OWN_FIELDS),
			]);
			connection = await connectToRelay(room.relay, sub);
			participant = await joinGarden(connection, {
				room: ROOM,
				name: NAME,
				...(pub === undefined ? {} : { publishNamespaceToken: pub }),
				onGarden,
				onState,
			});
			say(`in the garden “${ROOM}” as ${NAME}`);
			const close = await connection.closed;
			say(`the session ended (${close.type}); connecting again`);
		} catch (error) {
			say(`not connected: ${String(error).slice(0, 120)}; connecting again`);
		}
		participant = undefined;
		await new Promise((resolve) =>
			setTimeout(resolve, DEFAULT_RECONNECT_DELAY_MS),
		);
	}
}

/** Does `action` to the pet; whether it was sent. */
async function act(action: Action): Promise<boolean> {
	if (participant === undefined) {
		say("not connected yet");
		return false;
	}
	const outcome = await participant.act(action);
	if (outcome.type === "sent") return true;
	say(`not sent: ${outcome.why}`);
	return false;
}

function main(): void {
	const canvas = document.querySelector("canvas");
	const context = canvas?.getContext("2d");
	if (canvas === null || context === null || context === undefined) {
		throw new Error("no canvas");
	}
	canvas.addEventListener("click", (event) => {
		const box = canvas.getBoundingClientRect();
		const at = floorAt(
			canvas.width,
			canvas.height,
			((event.clientX - box.left) * canvas.width) / box.width,
			((event.clientY - box.top) * canvas.height) / box.height,
		);
		if (at === undefined) return;
		putting = {
			kind: tool === "feed" ? "bowl" : "ball",
			at,
			since: Date.now(),
		};
		void act({ kind: tool, to: at });
	});
	const bar = controlBar();
	choiceButtons(bar, {
		choices: [
			{ id: "throw", label: "ball" },
			{ id: "feed", label: "food" },
		],
		chosen: tool,
		choose(id) {
			tool = id === "feed" ? "feed" : "throw";
		},
	});
	actionButton(bar, {
		id: "pet",
		label: "pet",
		press: async () => void (await act({ kind: "pet" })),
	});
	actionButton(bar, {
		id: "call",
		label: "call",
		press: async () => void (await act({ kind: "call" })),
	});
	textField(bar, {
		id: "talk",
		label: "say",
		size: 24,
		send: (text) =>
			new TextEncoder().encode(text).length > TALK_BYTES
				? Promise.resolve(false)
				: act({ kind: "talk", text }),
	});
	const frame = (): void => {
		const now = Date.now();
		drawGarden(
			context,
			shown,
			putting === undefined ? things : [...things, putting],
			now,
		);
		// For a script that looks at the page.
		document.body.dataset.pet = JSON.stringify(shown ?? null);
		document.body.dataset.things = JSON.stringify(things);
		requestAnimationFrame(frame);
	};
	requestAnimationFrame(frame);
	void keepJoined();
}

main();
