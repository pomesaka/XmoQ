// The chat under the picture of both pages, by two ways at once while the
// chat moves from the room's WebSocket to MoQ tracks (decided on
// 2026-10-24): a line typed is sent on both, and the lines of both are
// shown, each marked with the way it came, "track" or "ws".
//
// Over the tracks the page is a participant (`joinChat` of `@xmoq/chat`):
// it announces `xmoq/chat/<room>/<name>`, says its lines on `said`, and
// reads the room's `lines` from the gatherer (`xmoq-chat`, which `serve.ts`
// starts), with the newest 16 by a Joining FETCH when it comes.

import {
	type ChatLimits,
	type ChatParticipant,
	chatLimits,
	joinChat,
	type KeptBack,
	keepBack,
	RateWindow,
	type SayOutcome,
} from "@xmoq/chat";
import type { MoqtConnection } from "@xmoq/moqt";

declare const XMOQ_DEMO: { chatTracks: boolean };

/** A line of the room's WebSocket (`spikes/durable-objects-room/worker.js`). */
export interface SocketLine {
	seq: number;
	from: string;
	text: string;
	/** The room's clock when it took the line; the history before 2026-10-24 has none. */
	at?: number;
}

/**
 * How many lines the page shows, of both ways together; the oldest go.
 * Not a flag: what a page holds is bounded whatever the address says.
 */
const SHOWN_LINES = 200;

/** How long the page waits before it connects again for the chat, as the pages do for the broadcast. */
const AGAIN_MS = 2_000;

/**
 * `?chatTextBytes=N` and `?chatLinesPerSecond=N`: the limits of a line,
 * which the gatherer has from its flags (`--text-bytes`,
 * `--lines-per-second`). The page keeps a line over them back and says so,
 * as the gatherer would drop it; set them as the gatherer is run. The
 * defaults are the gatherer's.
 */
function limitsOfTheAddress(): ChatLimits {
	const query = new URLSearchParams(location.search);
	const given: Partial<ChatLimits> = {};
	const textBytes = query.get("chatTextBytes");
	const linesPerSecond = query.get("chatLinesPerSecond");
	if (textBytes !== null) given.textBytes = Number(textBytes);
	if (linesPerSecond !== null) given.linesPerSecond = Number(linesPerSecond);
	return chatLimits(given);
}

const KEPT_BACK: Record<KeptBack, (limits: ChatLimits) => string> = {
	"too-long": (limits) => `not sent: longer than ${limits.textBytes} bytes`,
	"too-fast": (limits) =>
		`not sent: more than ${limits.linesPerSecond} lines in a second`,
	"not-well-formed": () => "not sent: the text has a broken character",
};

/**
 * The time shown with a line, HH:MM:SS by this page's clock.
 *
 * WHY the time the line was taken (the gatherer's `at`, and the room's
 * `at` for a line of the WebSocket) and not the sender's `sentAt`: the
 * lines are shown in the order they were taken, which is the one order
 * every viewer sees, and the times taken go up with it. `sentAt` is the
 * sender's clock, which nobody checks: a sender whose clock is wrong, or
 * who writes another time, would put a line hours away from the ones beside
 * it, and two lines out of the order of their times. WHY NOT both: the
 * difference is the sender's clock against the gatherer's plus the way
 * there, which a person reading the chat cannot tell apart.
 */
const shownTime = (at: number | undefined): string =>
	at === undefined ? "--:--:--" : new Date(at).toTimeString().slice(0, 8);

/**
 * Shows the chat of the room `room` as `name`, and sends what is typed in
 * the form, on the WebSocket with `sendOnSocket` and on a connection of
 * `connectForChat`. Returns what shows a line of the room's WebSocket.
 */
export function startChat(
	room: string,
	name: string,
	sendOnSocket: (text: string) => void,
	connectForChat: () => Promise<MoqtConnection>,
): { showSocketLine(line: SocketLine): void } {
	const log = document.getElementById("chat");
	const note = document.getElementById("chat-note");
	const say = (text: string): void => {
		if (note !== null) note.textContent = text;
	};
	let limits: ChatLimits;
	try {
		limits = limitsOfTheAddress();
	} catch (error) {
		say(`the chat's limits of the address: ${String(error)}; the defaults are used`);
		limits = chatLimits();
	}

	const add = (
		path: "track" | "ws",
		line: { name: string; text: string; at: number | undefined; number: string; how: string },
	): void => {
		if (log === null) return;
		const shown = document.createElement("div");
		shown.dataset.path = path;
		shown.dataset.number = line.number;
		shown.dataset.how = line.how;
		shown.textContent = `${shownTime(line.at)} [${path}] ${line.number} ${line.name}: ${line.text}`;
		// The lines of the tracks are placed by their number among the
		// lines of the tracks: the history of the Joining FETCH can come
		// after a line said meanwhile. A line of the WebSocket comes last.
		const after =
			path === "track"
				? [...log.querySelectorAll<HTMLElement>('div[data-path="track"]')].find(
						(other) => BigInt(other.dataset.number ?? "0") > BigInt(line.number),
					)
				: undefined;
		log.insertBefore(shown, after ?? null);
		while (log.childElementCount > SHOWN_LINES) log.firstElementChild?.remove();
		if (after === undefined) log.scrollTop = log.scrollHeight;
		document.body.dataset.chatLines = String(log.childElementCount);
	};

	let participant: ChatParticipant | undefined;
	if (XMOQ_DEMO.chatTracks) {
		void keepInTheRoom(
			connectForChat,
			{ room, name, limits },
			(joined) => {
				participant = joined;
			},
			(line) => add("track", { ...line, number: line.number.toString() }),
			say,
		);
	} else {
		say("chat over the room's WebSocket alone: no gatherer runs (with --auth it has no token)");
	}

	// The rate of a page whose chat goes over the WebSocket alone, counted
	// as the participant counts its own.
	const window = new RateWindow(limits.linesPerSecond);
	const form = document.querySelector("form");
	form?.addEventListener("submit", (event) => {
		event.preventDefault();
		const input = form.querySelector("input");
		if (input === null || input.value === "") return;
		const text = input.value;
		void (async () => {
			const outcome: SayOutcome | undefined = await participant?.say(text);
			const kept =
				outcome === undefined
					? keepBack(text, limits, window, Date.now())
					: outcome.type === "kept-back"
						? outcome.why
						: undefined;
			if (kept !== undefined) {
				// The text stays in the field, to be cut and sent again.
				say(KEPT_BACK[kept](limits));
				return;
			}
			sendOnSocket(text);
			input.value = "";
			say(
				outcome === undefined
					? "sent on the ws"
					: outcome.type === "sent"
						? "sent on the track and the ws"
						: `sent on the ws; not on the track: ${outcome.why}`,
			);
		})();
	});

	return {
		showSocketLine(line) {
			add("ws", {
				name: line.from,
				text: line.text,
				at: line.at,
				number: String(line.seq),
				how: "ws",
			});
		},
	};
}

/**
 * Connects for the chat, joins the room, and does so again `AGAIN_MS` after
 * the session ended, for as long as the page is open.
 *
 * WHY a connection of its own, and not the one of the broadcast: the
 * participant names the `serve` handlers of its connection, which the
 * publishing page needs for its own tracks, and the chat goes on while the
 * watching page changes its broadcast or connects again for it.
 */
async function keepInTheRoom(
	connectForChat: () => Promise<MoqtConnection>,
	where: { room: string; name: string; limits: ChatLimits },
	joined: (participant: ChatParticipant | undefined) => void,
	onLine: Parameters<typeof joinChat>[1]["onLine"],
	say: (text: string) => void,
): Promise<void> {
	for (;;) {
		try {
			const connection = await connectForChat();
			const participant = await joinChat(connection, {
				...where,
				onLine,
				onMalformed: (number, why) => say(`line ${number} of the track is no line: ${why}`),
				onStatus: (status) =>
					say(
						`chat over the track: ${status.heard ? "the gatherer listens" : "nobody listens yet"}, ${status.following ? "following the room" : `not following the room (${status.lastEnd ?? "not yet"})`}`,
					),
			});
			joined(participant);
			await connection.closed;
			joined(undefined);
			say("chat over the track: the session ended; connecting again");
		} catch (error) {
			joined(undefined);
			say(`chat over the track: ${String(error).slice(0, 120)}`);
		}
		await new Promise((resolve) => setTimeout(resolve, AGAIN_MS));
	}
}
