/// <reference path="./file-system-access.d.ts" />
// The folder a page of a meeting writes into for an agent, with the File
// System Access API of Chromium: one root the person chooses once, which is
// remembered (IndexedDB), and under it a folder for each meeting the page
// attends, `<root>/<room>/<id>/`, laid out as `xmoq-agent --out` lays its
// own: `AGENT.md`, `events/`, `inbox.txt` and `frames/<when the writing
// began>/`.

import { AGENT_GUIDE, GUIDE_FILE } from "../core/agent-guide.ts";
import {
	type AgentEvent,
	agentLine,
	EVENTS_DIR,
	eventFileName,
	eventFileNumber,
	nextEventFile,
} from "../core/agent-lines.ts";

/**
 * How long a line waits to be written to a file of `events/`, with the
 * lines after it, in ms. **Provisional.** WHY: each wait is a file; a
 * second is as long as the inbox waits too, and as `xmoq-agent` waits
 * (`FLUSH_EVERY`).
 */
export const FLUSH_EVERY_MS = 1000;

/** How many times a file is read before its NotFoundError is let through (`readWhole`). */
const READS_OF_A_FILE = 3;

/**
 * The bytes of a file of the folder, read again where it was written anew
 * while it was read. WHY: a writable's `close` moves what was written over
 * the file, and in the page's own storage (OPFS) that deletes the old
 * backing file of the name (`ObfuscatedFileUtil::CopyOrMoveFile`, "Delete
 * target entry's old backing file", Chromium's main, read on 2026-10-26).
 * A `File` taken before it then fails to be read with a NotFoundError: in
 * Chromium of Playwright 1.64.0, 24 and 32 of about 560 reads did, while
 * the file was written 300 times. The file under its name is whole all the
 * while, and the next `getFile` has it.
 */
export async function readWhole(
	file: Pick<FileSystemFileHandle, "getFile">,
): Promise<Uint8Array> {
	for (let read = 1; ; read++) {
		try {
			return new Uint8Array(await (await file.getFile()).arrayBuffer());
		} catch (error) {
			const replaced =
				error instanceof DOMException && error.name === "NotFoundError";
			if (!replaced || read >= READS_OF_A_FILE) throw error;
		}
	}
}

const DATABASE = "xmoq-agent-folder";
const STORE = "handles";
const ROOT_KEY = "root";

/** Where the root stands for this page now. */
export type RootState =
	/** The browser has no File System Access (not Chromium). */
	| { type: "unsupported" }
	/** None was chosen, or what was is gone. */
	| { type: "none" }
	/** Chosen, and the page may write in it. */
	| { type: "granted"; root: FileSystemDirectoryHandle }
	/** Chosen, and the person is to allow it again, with a click (`allowRoot`). */
	| { type: "to-allow"; root: FileSystemDirectoryHandle };

/** The one store of handles, opened and made where it is not. */
function database(): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const opening = indexedDB.open(DATABASE, 1);
		opening.onupgradeneeded = () => {
			opening.result.createObjectStore(STORE);
		};
		opening.onsuccess = () => resolve(opening.result);
		opening.onerror = () => reject(opening.error);
	});
}

async function stored(): Promise<FileSystemDirectoryHandle | undefined> {
	const db = await database();
	try {
		return await new Promise((resolve, reject) => {
			const getting = db
				.transaction(STORE, "readonly")
				.objectStore(STORE)
				.get(ROOT_KEY);
			getting.onsuccess = () => {
				const value: unknown = getting.result;
				resolve(value instanceof FileSystemDirectoryHandle ? value : undefined);
			};
			getting.onerror = () => reject(getting.error);
		});
	} finally {
		db.close();
	}
}

async function store(root: FileSystemDirectoryHandle): Promise<void> {
	const db = await database();
	try {
		await new Promise<void>((resolve, reject) => {
			const putting = db
				.transaction(STORE, "readwrite")
				.objectStore(STORE)
				.put(root, ROOT_KEY);
			putting.onsuccess = () => resolve();
			putting.onerror = () => reject(putting.error);
		});
	} finally {
		db.close();
	}
}

const READ_WRITE: FileSystemHandlePermissionDescriptor = { mode: "readwrite" };

/** The root that was chosen before, and whether the page may write in it now. Asks the person nothing. */
export async function rememberedRoot(): Promise<RootState> {
	if (window.showDirectoryPicker === undefined) return { type: "unsupported" };
	const root = await stored();
	if (root === undefined) return { type: "none" };
	const state = await root.queryPermission?.(READ_WRITE);
	return state === "granted"
		? { type: "granted", root }
		: { type: "to-allow", root };
}

/**
 * Has the person choose the root, and remembers it. **To be called from a
 * click**: the picker opens only on a gesture. It opens in Documents, and
 * remembers where it was last (`id`). Rejects where the person closes it.
 */
export async function chooseRoot(): Promise<FileSystemDirectoryHandle> {
	if (window.showDirectoryPicker === undefined) {
		throw new Error("this browser has no showDirectoryPicker: Chromium has it");
	}
	const root = await window.showDirectoryPicker({
		id: "xmoq",
		mode: "readwrite",
		startIn: "documents",
	});
	await store(root);
	return root;
}

/**
 * Asks the person to let the page write in `root` again. **To be called
 * from a click**, as the picker. True where it may.
 */
export async function allowRoot(
	root: FileSystemDirectoryHandle,
): Promise<boolean> {
	return (await root.requestPermission?.(READ_WRITE)) === "granted";
}

/** The folder of one meeting the page attends, for an agent. */
export interface AgentFolder {
	/** `<room>/<id>`, under the root: what an agent is told to read. */
	readonly path: string;
	/** When the writing began (ms), which names the folder of its pictures. */
	readonly began: number;
	/** Adds a line to the next file of `events/`, behind the ones before. Never throws; a failure is told to `onError`. */
	write(event: AgentEvent): void;
	/** Writes a picture at `path`, from the folder (`framePath`), before its line is written. */
	writeFile(path: string, data: Blob): Promise<void>;
	/** `inbox.txt` as it is now; empty where there is none. */
	readInbox(): Promise<Uint8Array>;
}

/**
 * Opens, and makes where it is not, `<root>/<room>/<id>/` with its
 * `events/`, `inbox.txt` and `frames/<began>/`, and puts `AGENT.md` there
 * anew. WHY anew: the guide is of this version of the page, and one left
 * by a call before may tell of another.
 */
export async function openAgentFolder(
	root: FileSystemDirectoryHandle,
	room: string,
	id: string,
	options: { now?: () => number; onError?(error: unknown): void } = {},
): Promise<AgentFolder> {
	const now = options.now ?? Date.now;
	const folder = await (
		await root.getDirectoryHandle(room, { create: true })
	).getDirectoryHandle(id, { create: true });
	const events = await folder.getDirectoryHandle(EVENTS_DIR, { create: true });
	const inbox = await folder.getFileHandle("inbox.txt", { create: true });
	const guide = await (
		await folder.getFileHandle(GUIDE_FILE, { create: true })
	).createWritable();
	try {
		await guide.write(AGENT_GUIDE);
	} finally {
		await guide.close();
	}
	/** The number of the newest file of `events/`, which the next is named after (`nextEventFile`). */
	let newest: number | undefined;
	for await (const name of events.keys()) {
		const number = eventFileNumber(name);
		if (number !== undefined && (newest === undefined || number > newest)) {
			newest = number;
		}
	}
	const began = now();
	/** The lines that wait to be written, and the write under way. */
	let waiting: string[] = [];
	/** A flush is set for the end of this second. */
	let flushing = false;
	let writing: Promise<void> = Promise.resolve();
	const flush = async (): Promise<void> => {
		if (waiting.length === 0) return;
		const text = waiting.join("");
		waiting = [];
		const number = nextEventFile(now(), newest);
		newest = number;
		// WHY a file of its own for every flush, and not one file added to:
		// Chromium writes into a swap file, which keeps a copy of the file
		// (`keepExistingData`), and moves it over the file at `close`
		// (`FileSystemAccessSafeMoveHelper` in
		// content/browser/file_system_access/file_system_access_file_writer_impl.cc,
		// Chromium's main, read on 2026-10-25). One file would be copied
		// whole once a flush, and be another file after it, which `tail -F`
		// of macOS prints again from its beginning (run on 2026-10-27).
		// What a reader sees meanwhile: the file is there, empty, from
		// `getFileHandle`, and has all its lines from `close`, the swap
		// file moved over it; never a part of them. A
		// reader takes an empty file for one being written
		// (`fixtures/agent-lines/README.md`).
		const file = await events.getFileHandle(eventFileName(number), {
			create: true,
		});
		const writable = await file.createWritable();
		try {
			await writable.write(text);
		} finally {
			await writable.close();
		}
	};
	const fileAt = async (path: string): Promise<FileSystemFileHandle> => {
		const names = path.split("/");
		const file = names.pop();
		if (file === undefined) throw new Error(`no file in ${path}`);
		let at = folder;
		for (const name of names) {
			at = await at.getDirectoryHandle(name, { create: true });
		}
		return at.getFileHandle(file, { create: true });
	};
	return {
		path: `${room}/${id}`,
		began,
		write(event) {
			try {
				waiting.push(`${agentLine(event, now())}\n`);
			} catch (error) {
				options.onError?.(error);
				return;
			}
			// The lines of a second go in one flush: each copies the file.
			if (flushing) return;
			flushing = true;
			setTimeout(() => {
				flushing = false;
				writing = writing.then(flush).catch((error: unknown) => {
					options.onError?.(error);
				});
			}, FLUSH_EVERY_MS);
		},
		async writeFile(path, data) {
			const writable = await (await fileAt(path)).createWritable();
			try {
				await writable.write(data);
			} finally {
				await writable.close();
			}
		},
		readInbox: () => readWhole(inbox),
	};
}
