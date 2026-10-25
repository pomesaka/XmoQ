// What an agent writes into `inbox.txt` of a page's folder: a line of the
// chat, or a command, as a line of stdin of `xmoq-agent`
// (`crates/agent/src/input.rs`), so that an agent says the same to either.

/** What a line asks for. */
export type InboxLine =
	/** A line said in the chat. */
	| { type: "say"; text: string }
	/** `/frame <id>`: a picture of `id` now; `/frame` alone, of nobody named. */
	| { type: "frame"; id: string | undefined }
	/** A command that is none of these, with its name. */
	| { type: "unknown"; name: string }
	/** An empty line, which asks for nothing. */
	| { type: "nothing" };

/** The longest line read, in bytes, as for stdin of `xmoq-agent`. Longer is an `error` line and not read. */
export const MAX_INBOX_LINE_BYTES = 4096;

/**
 * Reads one line, without its line feed. A carriage return before the line
 * feed (a line from Windows) is taken off as well. A line that begins with
 * `/` is a command; one that begins with `//` is said with one `/` taken
 * off, so that the chat can have a line that begins with `/`.
 */
export function readInboxLine(line: string): InboxLine {
	const text = line.endsWith("\r") ? line.slice(0, -1) : line;
	if (text === "") return { type: "nothing" };
	if (text.startsWith("//")) return { type: "say", text: text.slice(1) };
	if (!text.startsWith("/")) return { type: "say", text };
	// The words after the first that a command takes are let be, as
	// `xmoq-agent` lets them.
	const [name = "", id] = text
		.slice(1)
		.split(/\s+/)
		.filter((word) => word !== "");
	return name === "frame" ? { type: "frame", id } : { type: "unknown", name };
}

/** What is new in the inbox since `offset`, a count of bytes. */
export interface InboxRead {
	/** The whole lines after `offset`, as UTF-8; a line over `MAX_INBOX_LINE_BYTES` is `undefined`. */
	lines: (string | undefined)[];
	/** Where the next read begins: after the last line feed. */
	offset: number;
	/** The first bytes of the file as it was read (`INBOX_HEAD_BYTES` at most), to tell a file written anew at the next read. */
	head: Uint8Array;
}

/** How many first bytes of the inbox are kept to tell a file written anew from one added to. */
export const INBOX_HEAD_BYTES = 64;

/**
 * The whole lines of `bytes` (the file as it is now) after `offset`. A last
 * line without its line feed is left for the next read: the agent may be
 * writing it. **A file shorter than `offset`, or one that does not begin
 * as `head` did, was written anew** (`>` in place of `>>`), and is read
 * from its beginning. WHY by bytes: the file is read again and again,
 * and a count of characters would split a character that a read cut.
 */
export function newInboxLines(
	bytes: Uint8Array,
	offset: number,
	head: Uint8Array = new Uint8Array(),
): InboxRead {
	const same = head.every((byte, at) => bytes[at] === byte);
	const from = bytes.length < offset || !same ? 0 : offset;
	const decoder = new TextDecoder();
	const lines: (string | undefined)[] = [];
	let start = from;
	for (let at = from; at < bytes.length; at++) {
		if (bytes[at] !== 0x0a) continue;
		lines.push(
			at - start > MAX_INBOX_LINE_BYTES
				? undefined
				: decoder.decode(bytes.subarray(start, at)),
		);
		start = at + 1;
	}
	return {
		lines,
		offset: start,
		head: bytes.slice(0, Math.min(INBOX_HEAD_BYTES, start)),
	};
}
