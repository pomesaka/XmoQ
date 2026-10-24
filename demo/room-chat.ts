// The chat under the picture of both pages, over MoQ tracks alone (Q36
// and Q49, decided on 2026-10-24; the room's WebSocket carries no chat).
//
// The page is a participant (`joinChat` of `@xmoq/chat`): it announces
// `xmoq/chat/<room>/<name>`, says its lines on `said`, and reads the
// room's `lines` from the gatherer (`xmoq-chat`, which `serve.ts` starts),
// with the newest 16 by a Joining FETCH when it comes.

import {
	type ChatLimits,
	type ChatParticipant,
	chatLimits,
	joinChat,
	type KeptBack,
	type SayOutcome,
} from "@xmoq/chat";
import type { MoqtConnection } from "@xmoq/moqt";

declare const XMOQ_DEMO: { chatTracks: boolean };

/**
 * How many lines the page shows; the oldest go.
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
 * WHY the time the line was taken (the gatherer's `at`) and not the
 * sender's `sentAt`: the lines are shown in the order they were taken,
 * which is the one order every viewer sees, and the times taken go up with it. `sentAt` is the
 * sender's clock, which nobody checks: a sender whose clock is wrong, or
 * who writes another time, would put a line hours away from the ones beside
 * it, and two lines out of the order of their times. WHY NOT both: the
 * difference is the sender's clock against the gatherer's plus the way
 * there, which a person reading the chat cannot tell apart.
 */
const shownTime = (at: number): string => new Date(at).toTimeString().slice(0, 8);

/**
 * Shows the chat of the room `room` as `name`, and says what is typed in
 * the form, on a connection of `connectForChat`. Does nothing where the
 * page has no chat (`serve.ts` runs no gatherer).
 */
export function startChat(
	room: string,
	name: string,
	connectForChat: () => Promise<MoqtConnection>,
): void {
	const log = document.getElementById("chat");
	const note = document.getElementById("chat-note");
	if (!XMOQ_DEMO.chatTracks || log === null) return;
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

	const add = (line: { name: string; text: string; at: number; number: bigint; how: string }): void => {
		const shown = document.createElement("div");
		shown.dataset.number = line.number.toString();
		shown.dataset.how = line.how;
		shown.textContent = `${shownTime(line.at)} ${line.number} ${line.name}: ${line.text}`;
		// A line is placed by its number: the history of the Joining FETCH
		// can come after a line said meanwhile.
		const after = [...log.querySelectorAll<HTMLElement>("div[data-number]")].find(
			(other) => BigInt(other.dataset.number ?? "0") > line.number,
		);
		log.insertBefore(shown, after ?? null);
		while (log.childElementCount > SHOWN_LINES) log.firstElementChild?.remove();
		if (after === undefined) log.scrollTop = log.scrollHeight;
		document.body.dataset.chatLines = String(log.childElementCount);
	};

	let participant: ChatParticipant | undefined;
	void keepInTheRoom(
		connectForChat,
		{ room, name, limits },
		(joined) => {
			participant = joined;
		},
		add,
		say,
	);

	const form = document.querySelector("form");
	form?.addEventListener("submit", (event) => {
		event.preventDefault();
		const input = form.querySelector("input");
		if (input === null || input.value === "") return;
		const text = input.value;
		void (async () => {
			// The text stays in the field where it was not sent, to be sent again.
			if (participant === undefined) {
				say("not sent: not in the room's chat yet");
				return;
			}
			const outcome: SayOutcome = await participant.say(text);
			if (outcome.type === "kept-back") {
				say(KEPT_BACK[outcome.why](limits));
				return;
			}
			if (outcome.type === "not-sent") {
				say(`not sent: ${outcome.why}`);
				return;
			}
			input.value = "";
			say("sent");
		})();
	});
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
