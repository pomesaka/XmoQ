// A page of a meeting as a reader for an agent on the same machine: what
// happens in the meeting is written into its folder as `xmoq-agent` writes
// it, and the lines the agent writes into `inbox.txt` are said in the chat,
// or ask for a picture. The person and the agent share the page's seat:
// what the agent says is said under the page's name.

import type { FrameOrigin } from "@xmoq/media";
import {
	MAX_INBOX_LINE_BYTES,
	newInboxLines,
	readInboxLine,
} from "../core/agent-inbox.ts";
import { type AgentEvent, framePath } from "../core/agent-lines.ts";
import type { AgentFolder } from "./agent-folder.ts";

/**
 * How often `inbox.txt` is read again, in ms. **Provisional.** WHY read
 * again: a page is told of no change to a file (the File System Access API
 * has no observer in Chromium 156), so a line an agent wrote waits until
 * then. A second is as long as a person waits for a line of the chat.
 */
export const INBOX_EVERY_MS = 1000;

export interface AgentBridgeOptions {
	folder: AgentFolder;
	/** Says a line in the chat as the page; resolves with why it was not sent, or undefined where it was. */
	say(text: string): Promise<string | undefined>;
	/** `INBOX_EVERY_MS` when left out. */
	inboxEveryMs?: number;
	/** How the page waits between two reads: a timer when left out. For a test that moves the reads on by itself. */
	sleep?(ms: number): Promise<void>;
}

export interface AgentBridge {
	/** A participant came or left (the `came` and `left` of `MeetingEvent`). */
	participant(state: "joined" | "left", id: string): void;
	/** A line of the room's chat. */
	chat(line: {
		number: bigint;
		name: string;
		text: string;
		sentAt: number;
		at: number;
	}): void;
	/** Something said in the meeting, as the transcriber read it. */
	speech(piece: {
		number: bigint;
		from: readonly string[];
		text: string;
		startUs: number;
		endUs: number;
		at: number;
	}): void;
	/** A frame of `id`, which the caller still closes: the latest of each is kept for `/frame`. */
	frame(id: string, frame: VideoFrame, at: FrameOrigin | undefined): void;
	/** Stops reading the inbox; what was written stays. */
	stop(): void;
}

/** The latest picture of a participant, drawn as it came. */
interface Latest {
	canvas: OffscreenCanvas;
	at: FrameOrigin | undefined;
}

/**
 * Writes the meeting into `folder`, and reads its inbox every
 * `inboxEveryMs` until `stop`. A line of the inbox is read as a line of
 * stdin of `xmoq-agent` (`readInboxLine`): said in the chat, or `/frame
 * <id>` for a picture of `id`; `/frame` alone is of whoever spoke last.
 * What cannot be done is an `error` line, as the CLI writes one.
 */
export function createAgentBridge(options: AgentBridgeOptions): AgentBridge {
	const { folder } = options;
	const write = (event: AgentEvent): void => folder.write(event);
	const latest = new Map<string, Latest>();
	/** The participant whose words came last, for `/frame` alone. */
	let spokeLast: string | undefined;
	let taken = 0;
	let offset = 0;
	let head: Uint8Array = new Uint8Array();
	/** The last failure to read the inbox that was written, so that one that goes on is written once. */
	let failing: string | undefined;
	let stopped = false;

	const picture = async (id: string | undefined): Promise<void> => {
		const whose = id ?? spokeLast;
		if (whose === undefined) {
			write({
				type: "error",
				message: "/frame: nobody spoke yet; name one, /frame <id>",
			});
			return;
		}
		const kept = latest.get(whose);
		if (kept === undefined) {
			write({
				type: "error",
				message: `/frame ${whose}: no picture of ${whose} came`,
			});
			return;
		}
		if (kept.at === undefined) {
			write({
				type: "error",
				message: `/frame ${whose}: where its picture came from was not kept`,
			});
			return;
		}
		taken += 1;
		const path = framePath(folder.began, taken, kept.at.group, kept.at.object);
		await folder.writeFile(
			path,
			await kept.canvas.convertToBlob({ type: "image/png" }),
		);
		write({
			type: "frame",
			path,
			group: kept.at.group,
			object: kept.at.object,
			timestampUs: kept.at.timestampUs,
			width: kept.canvas.width,
			height: kept.canvas.height,
			asked: "command",
			id: whose,
		});
	};

	const act = async (line: string | undefined): Promise<void> => {
		if (line === undefined) {
			write({
				type: "error",
				message: `a line of inbox.txt over ${MAX_INBOX_LINE_BYTES} bytes was not read`,
			});
			return;
		}
		const read = readInboxLine(line);
		switch (read.type) {
			case "nothing":
				return;
			case "say": {
				const why = await options.say(read.text);
				if (why !== undefined)
					write({ type: "error", message: `not said: ${why}` });
				return;
			}
			case "frame":
				await picture(read.id);
				return;
			case "unknown":
				write({
					type: "error",
					message: `no command /${read.name}: /frame is one, and // begins a line that is said`,
				});
		}
	};

	/** Reads what is new in the inbox, and does it, in order. */
	const readInbox = async (): Promise<void> => {
		const read = newInboxLines(await folder.readInbox(), offset, head);
		offset = read.offset;
		head = read.head;
		failing = undefined;
		for (const line of read.lines) await act(line);
	};
	// WHY what is in the inbox when the page begins is passed over: it was
	// written for a page before, which did it or did not.
	const reading = (async () => {
		const first = newInboxLines(await folder.readInbox(), 0);
		offset = first.offset;
		head = first.head;
		const sleep =
			options.sleep ??
			((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
		while (!stopped) {
			await sleep(options.inboxEveryMs ?? INBOX_EVERY_MS);
			if (stopped) return;
			await readInbox().catch((error: unknown) => {
				const message = `inbox.txt: ${error instanceof Error ? error.message : String(error)}`;
				// WHY once: a file that was taken away fails every second, and
				// the lines an agent reads would be these alone.
				if (message === failing) return;
				failing = message;
				write({ type: "error", message });
			});
		}
	})();
	void reading;

	return {
		participant(state, id) {
			// The picture of one who left is not to be taken for a picture now.
			if (state === "left") latest.delete(id);
			write({ type: "participant", state, id });
		},
		chat: (line) =>
			write({
				type: "chat",
				number: line.number,
				from: line.name,
				text: line.text,
				sentAt: line.sentAt,
				gatheredAt: line.at,
			}),
		speech(piece) {
			spokeLast = piece.from.at(-1);
			write({
				type: "speech",
				number: piece.number,
				from: piece.from,
				text: piece.text,
				startUs: piece.startUs,
				endUs: piece.endUs,
				readAt: piece.at,
			});
		},
		frame(id, frame, at) {
			let kept = latest.get(id);
			if (
				kept === undefined ||
				kept.canvas.width !== frame.displayWidth ||
				kept.canvas.height !== frame.displayHeight
			) {
				kept = {
					canvas: new OffscreenCanvas(frame.displayWidth, frame.displayHeight),
					at,
				};
				latest.set(id, kept);
			}
			// WHY drawn and not kept: a frame that is held keeps the decoder's
			// memory, and the decoder stops when it has none to give.
			kept.canvas.getContext("2d")?.drawImage(frame, 0, 0);
			kept.at = at;
		},
		stop() {
			stopped = true;
		},
	};
}
