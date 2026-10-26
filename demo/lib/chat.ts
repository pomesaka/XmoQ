// The chat under the picture of the pages, as the demo shows it
// (`showChat` of `@xmoq/chat`): its elements are `#chat`, `#chat-note` and
// the page's form, and its limits are those `serve.ts` started the gatherer
// with. Where `serve.ts` runs no gatherer, the page has no chat.

import {
	type ChatConnection,
	type ChatLimits,
	type ChatRoom,
	type ChatSaying,
	catchUpLines,
	type ReceivedLine,
	showChat,
} from "@xmoq/chat";
import type { GoAwayNotice } from "@xmoq/moqt";

/**
 * Whether the gatherer runs, and the limits of a line it was started with
 * (`--chat-text-bytes`, `--chat-lines-per-second` of `serve.ts`): a page
 * keeps a line over them back and says so, as the gatherer would drop it.
 */
declare const XMOQ_DEMO: { chatTracks: boolean; chatLimits: ChatLimits };

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
			limits: XMOQ_DEMO.chatLimits,
			catchUpLines: catchUp,
			shownLines,
			...(onLine === undefined ? {} : { onLine }),
		},
	);
}
