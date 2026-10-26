// The chat of a room shown in a page, over MoQ tracks alone (decided on
// 2026-10-24): a participant (`keepInTheRoom`) that announces
// `xmoq/chat/<kind>/<room>/<name>`, says what is typed in a form on
// `said`, and shows the room's `lines` from the gatherer, by their numbers,
// with the newest by a Joining FETCH when it comes and the ones it missed
// when it subscribes or connects again; where it missed more, one dimmed
// line says so above them. Which elements, which limits and how many lines
// are the page's to say: an application gives them.

import type { GoAwayNotice } from "@xmoq/moqt";
import type { LinesGap } from "../core/history.ts";
import type { ChatLimits, KeptBack } from "../core/limits.ts";
import type { ChatRoom } from "../core/names.ts";
import {
	type ChatConnection,
	keepInTheRoom,
} from "../moqt/keep-in-the-room.ts";
import type {
	ChatParticipant,
	ReceivedLine,
	SayOutcome,
} from "../moqt/participant.ts";

/** The elements the chat is shown in. */
export interface ChatView {
	/** The lines, one `div` each, the oldest first; `data-lines` says how many. */
	log: HTMLElement;
	/** One line on how the chat goes, and why a line was not sent. */
	note?: HTMLElement | null;
	/** A form with one input, whose text is said when it is sent. */
	form?: HTMLFormElement | null;
}

export interface ShowChatOptions {
	room: ChatRoom;
	name: string;
	/**
	 * A connection for the chat, with the GOAWAY that ended the session
	 * before, where one did. WHY a connection of its own, and not one of a
	 * broadcast: the participant names the `serve` handlers of its
	 * connection, which a publisher needs for its own tracks, and the chat
	 * goes on while a viewer changes its broadcast or connects again for it.
	 */
	connect(goAway?: GoAwayNotice): Promise<ChatConnection>;
	/** What a line may be: the gatherer's (`chatLimits`), as it is run. */
	limits: ChatLimits;
	/** How many lines it missed are fetched at the most (`catchUpLines`). */
	catchUpLines: number;
	/** How many lines are shown; the oldest go. Bounds what the page holds. */
	shownLines: number;
	/** Called once for each line that comes, before it is shown. */
	onLine?(line: ReceivedLine): void;
}

/** Says a line in the chat from elsewhere than its form (an agent's inbox). */
export interface ChatSaying {
	/** Why it was not sent (without a "not sent"), or undefined where it was. */
	say(text: string): Promise<string | undefined>;
}

const KEPT_BACK: Record<KeptBack, (limits: ChatLimits) => string> = {
	"too-long": (limits) => `longer than ${limits.textBytes} bytes`,
	"too-fast": (limits) =>
		`more than ${limits.linesPerSecond} lines in a second`,
	"not-well-formed": () => "the text has a broken character",
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
const shownTime = (at: number): string =>
	new Date(at).toTimeString().slice(0, 8);

/**
 * Shows the chat of `options.room` as `options.name` in `view`, and says
 * what is typed in its form.
 */
export function showChat(
	{ log, note, form }: ChatView,
	{
		room,
		name,
		connect,
		limits,
		catchUpLines,
		shownLines,
		onLine,
	}: ShowChatOptions,
): ChatSaying {
	const say = (text: string): void => {
		if (note) note.textContent = text;
	};

	/** The newest line number shown, for the participant of the next connection (`lastHad`). */
	let lastHad: bigint | undefined;
	/**
	 * Places `shown` by its number: the history of the Joining FETCH can
	 * come after a line said meanwhile. A gap has the number of the first
	 * line after it (`before`), and goes before that line: a line goes
	 * before the first element of a larger number, a gap before the first
	 * of the same number or larger.
	 */
	const place = (shown: HTMLElement, number: bigint, isGap: boolean): void => {
		const after = [
			...log.querySelectorAll<HTMLElement>("div[data-number]"),
		].find((other) => {
			const theirs = BigInt(other.dataset.number ?? "0");
			return isGap ? theirs >= number : theirs > number;
		});
		log.insertBefore(shown, after ?? null);
		while (log.childElementCount > shownLines) log.firstElementChild?.remove();
		if (after === undefined) log.scrollTop = log.scrollHeight;
		log.dataset.lines = String(log.querySelectorAll("div[data-how]").length);
	};
	const add = (line: ReceivedLine): void => {
		onLine?.(line);
		if (lastHad === undefined || line.number > lastHad) lastHad = line.number;
		const shown = document.createElement("div");
		shown.dataset.number = line.number.toString();
		shown.dataset.how = line.how;
		shown.textContent = `${shownTime(line.at)} ${line.number} ${line.name}: ${line.text}`;
		place(shown, line.number, false);
	};
	const addGap = (gap: LinesGap): void => {
		const shown = document.createElement("div");
		shown.dataset.number = gap.before.toString();
		shown.dataset.gap = "";
		shown.style.opacity = "0.6";
		shown.textContent = `lines ${gap.after + 1n} to ${gap.before - 1n} were not fetched`;
		place(shown, gap.before, true);
	};

	let participant: ChatParticipant | undefined;
	// Each participant is told the newest line shown (`lastHad`), so that it
	// fetches the lines the page missed while it was away.
	void keepInTheRoom({
		connect,
		join: () => ({
			room,
			name,
			limits,
			catchUpLines,
			lastHad,
			onLine: add,
			onGap: addGap,
			onMalformed: (number, why) =>
				say(`line ${number} of the track is no line: ${why}`),
			onStatus: (status) =>
				say(
					`chat over the track: ${status.heard ? "the gatherer listens" : "nobody listens yet"}, ${status.following ? "following the room" : `not following the room (${status.lastEnd ?? "not yet"})`}`,
				),
		}),
		onState(state) {
			if (state.type === "joined") {
				participant = state.participant;
				return;
			}
			participant = undefined;
			if (state.type === "session-ended")
				say("chat over the track: the session ended; connecting again");
			if (state.type === "moved")
				say(
					`chat over the track: the relay sent it to ${state.goAway.newSessionUri || "itself"}`,
				);
			if (state.type === "failed")
				say(`chat over the track: ${String(state.error).slice(0, 120)}`);
		},
	});

	/** Says `text`; why it was not sent (without a "not sent"), or undefined where it was. */
	const sayLine = async (text: string): Promise<string | undefined> => {
		if (participant === undefined) return "not in the room's chat yet";
		const outcome: SayOutcome = await participant.say(text);
		if (outcome.type === "kept-back") return KEPT_BACK[outcome.why](limits);
		if (outcome.type === "not-sent") return outcome.why;
		return undefined;
	};
	form?.addEventListener("submit", (event) => {
		event.preventDefault();
		const input = form.querySelector("input");
		if (input === null || input.value === "") return;
		const text = input.value;
		void (async () => {
			// The text stays in the field where it was not sent, to be sent again.
			const why = await sayLine(text);
			if (why !== undefined) {
				say(`not sent: ${why}`);
				return;
			}
			input.value = "";
			say("sent");
		})();
	});
	return { say: sayLine };
}
