// The WebSocket of a room (`spikes/durable-objects-room/worker.js`) as the
// pages of the demo hold it: opened again whenever it closes, and what is
// asked of the room matched to its answer.
//
// WHY opened again: the tokens for the relay, and a meeting's seat, are
// asked of the room for every connection to the relay. A page whose socket
// had closed asked on it for ever, and the connections it would make again
// (its broadcast, what it watches, the chat, the transcript) waited with it.

import { createNextSession, DEFAULT_RECONNECT_DELAY_MS } from "@xmoq/moqt";

export interface RoomSocket<M extends { type: string }> {
	/** Sends `message` where the socket is open; nothing where it is not. */
	send(message: object): void;
	/**
	 * Sends `message` where the socket is open, or as it opens, and resolves
	 * with the first message of a type of `answers` after those that answer
	 * what was asked before. Undefined where the socket is closed, or closes
	 * before the answer. WHY by the order: the room answers token-for and
	 * join-meeting in the order they came, and names no asker in its answer.
	 */
	ask(message: object, answers: M["type"][]): Promise<M | undefined>;
}

/**
 * Opens the room's socket at `url`. `open` is called each time it opens,
 * before what was asked meanwhile is sent, and `close` each time it closes;
 * the next try is made `DEFAULT_RECONNECT_DELAY_MS` after the one before
 * began (`createNextSession`).
 */
export function openRoomSocket<M extends { type: string }>(
	url: string,
	on: { message(message: M): void; open?(): void; close?(): void },
): RoomSocket<M> {
	const next = createNextSession(DEFAULT_RECONNECT_DELAY_MS, performance.now());
	/** Who waits for an answer, in the order they asked. */
	let asked: { answers: M["type"][]; resolve(message: M | undefined): void }[] =
		[];
	/** What was asked while the socket was opening, sent as it opens. */
	let unsent: string[] = [];
	let socket = connect();
	function connect(): WebSocket {
		const opened = new WebSocket(url);
		opened.addEventListener("open", () => {
			on.open?.();
			for (const text of unsent.splice(0)) opened.send(text);
		});
		opened.addEventListener("message", (event) => {
			const message: M = JSON.parse(String(event.data));
			const at = asked.findIndex(({ answers }) => answers.includes(message.type));
			if (at !== -1) asked.splice(at, 1)[0]?.resolve(message);
			on.message(message);
		});
		// A socket that could not be opened has its `close` as well, after
		// its `error`.
		opened.addEventListener("close", () => {
			const unanswered = asked;
			asked = [];
			unsent = [];
			for (const { resolve } of unanswered) resolve(undefined);
			on.close?.();
			setTimeout(() => {
				socket = connect();
			}, next.after("ended", performance.now()));
		});
		return opened;
	}
	return {
		send(message) {
			if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
		},
		ask(message, answers) {
			if (socket.readyState !== WebSocket.OPEN && socket.readyState !== WebSocket.CONNECTING) {
				return Promise.resolve(undefined);
			}
			const answered = new Promise<M | undefined>((resolve) => {
				asked.push({ answers, resolve });
			});
			if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
			else unsent.push(JSON.stringify(message));
			return answered;
		},
	};
}

/** §10.2.2: Alias Type USE_VALUE (0x3), Token Type 0, the text the room gave. */
export const tokenOf = (text: string | null): Uint8Array | undefined =>
	text === null
		? undefined
		: new Uint8Array([0x03, 0x00, ...new TextEncoder().encode(text)]);
