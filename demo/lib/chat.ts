// The chat under the picture of the pages, as the demo shows it
// (`showChat` of `@xmoq/chat`): its elements are `#chat`, `#chat-note` and
// the page's form, and its limits come from the address. Where `serve.ts`
// runs no gatherer, the page has no chat.

import {
	type ChatConnection,
	type ChatLimits,
	type ChatRoom,
	type ChatSaying,
	catchUpLines,
	chatLimits,
	type ReceivedLine,
	showChat,
} from "@xmoq/chat";
import type { GoAwayNotice } from "@xmoq/moqt";

declare const XMOQ_DEMO: { chatTracks: boolean };

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

/**
 * `?chatCatchUpLines=N`: how many of the lines it missed the page fetches
 * at the most when it subscribes or connects again, from 1 to 1,000
 * (`catchUpLines` of `@xmoq/chat`); 200 when left out. More than the page
 * shows are fetched and the oldest go.
 */
function catchUpOfTheAddress(): number {
	const given = new URLSearchParams(location.search).get("chatCatchUpLines");
	return catchUpLines(given === null ? undefined : Number(given));
}

/**
 * Shows the chat of `room` as `name` on connections of `connectForChat`,
 * the newest `shownLines` of it; undefined where the page has no chat.
 */
export function startChat(
	room: ChatRoom,
	name: string,
	connectForChat: (goAway?: GoAwayNotice) => Promise<ChatConnection>,
	{ shownLines, onLine }: { shownLines: number; onLine?(line: ReceivedLine): void },
): ChatSaying | undefined {
	const log = document.getElementById("chat");
	const note = document.getElementById("chat-note");
	if (!XMOQ_DEMO.chatTracks || log === null) return undefined;
	const say = (text: string): void => {
		if (note !== null) note.textContent = text;
	};
	let limits: ChatLimits;
	try {
		limits = limitsOfTheAddress();
	} catch (error) {
		say(
			`the chat's limits of the address: ${String(error)}; the defaults are used`,
		);
		limits = chatLimits();
	}
	let catchUp: number;
	try {
		catchUp = catchUpOfTheAddress();
	} catch (error) {
		say(
			`the chat's catching up of the address: ${String(error)}; the default is used`,
		);
		catchUp = catchUpLines();
	}
	return showChat(
		{ log, note, form: document.querySelector("form") },
		{
			room,
			name,
			connect: connectForChat,
			limits,
			catchUpLines: catchUp,
			shownLines,
			...(onLine === undefined ? {} : { onLine }),
		},
	);
}
