import { expect, test } from "bun:test";
import { agentLine } from "../src/core/agent-lines.ts";
import { createAgentBridge } from "../src/web/agent-bridge.ts";
import type { AgentFolder } from "../src/web/agent-folder.ts";

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
/** Lets what was set going run to where it waits again. */
const settle = async (): Promise<void> => {
	for (let turn = 0; turn < 10; turn++) {
		await new Promise((resolve) => setTimeout(resolve, 0));
	}
};

/**
 * A folder of the test: the lines written, as their `type` and the fields
 * that tell them apart, and an inbox that is set by the test.
 */
function folderOf(inbox: { now: string }): {
	folder: AgentFolder;
	written: string[];
} {
	const written: string[] = [];
	return {
		written,
		folder: {
			path: "demo/me",
			began: 0,
			write(event) {
				// The first and the last line are the test below's.
				if (event.type === "reader") return;
				// The line as the page writes it, without its clock.
				written.push(agentLine(event, 0).replace(/"at":0,?/, ""));
			},
			async writeFile() {},
			readInbox: async () => utf8(inbox.now),
		},
	};
}

test.each<{
	name: string;
	/** What the inbox holds when the bridge begins. */
	before: string;
	/** What it holds after. */
	after: string;
	said: string[];
	written: string[];
}>([
	{
		name: "a line is said in the chat, and what was there before is passed over",
		before: "old\n",
		after: "old\nhello\n//frame\n",
		said: ["hello", "/frame"],
		written: [],
	},
	{
		name: "a line that is not said is an error",
		before: "",
		after: "too fast\n",
		said: ["too fast"],
		written: ['{"message":"not said: kept back","type":"error"}'],
	},
	{
		name: "a command that is none is an error, as for xmoq-agent",
		before: "",
		after: "/quit\n",
		said: [],
		written: [
			'{"message":"no command /quit: /frame is one, and // begins a line that is said","type":"error"}',
		],
	},
	{
		name: "a picture of nobody named, and of somebody with none, are errors",
		before: "",
		after: "/frame\n/frame p1\n",
		said: [],
		written: [
			'{"message":"/frame: nobody spoke yet; name one, /frame <id>","type":"error"}',
			'{"message":"/frame p1: no picture of p1 came","type":"error"}',
		],
	},
	{
		name: "a line over 4,096 bytes is not read",
		before: "",
		after: `${"x".repeat(4097)}\n`,
		said: [],
		written: [
			'{"message":"a line of inbox.txt over 4096 bytes was not read","type":"error"}',
		],
	},
])("$name", async ({ before, after, said, written }) => {
	const inbox = { now: before };
	const { folder, written: lines } = folderOf(inbox);
	const sayings: string[] = [];
	/** The wait of the bridge between two reads, which the test ends. */
	let waited: (() => void) | undefined;
	const bridge = createAgentBridge({
		folder,
		name: "me",
		room: "meeting/demo",
		id: "me",
		async say(text) {
			sayings.push(text);
			return text === "too fast" ? { why: "kept back" } : { sentAt: 1 };
		},
		sleep: () =>
			new Promise((resolve) => {
				waited = resolve;
			}),
	});
	await settle();
	inbox.now = after;
	waited?.();
	await settle();
	bridge.stop();
	expect({ said: sayings, written: lines }).toEqual({ said, written });
});

test("an inbox that cannot be read is written of once while it goes on", async () => {
	const written: string[] = [];
	let waited: (() => void) | undefined;
	let reads = 0;
	const bridge = createAgentBridge({
		folder: {
			path: "demo/me",
			began: 0,
			write: (event) => {
				if (event.type !== "reader") written.push(event.type);
			},
			async writeFile() {},
			async readInbox() {
				reads += 1;
				if (reads > 1) throw new Error("the file is gone");
				return new Uint8Array();
			},
		},
		name: "me",
		room: "meeting/demo",
		id: "me",
		say: async () => ({ sentAt: 1 }),
		sleep: () =>
			new Promise((resolve) => {
				waited = resolve;
			}),
	});
	for (let read = 0; read < 3; read++) {
		await settle();
		waited?.();
	}
	await settle();
	bridge.stop();
	expect({ reads: reads > 3, written }).toEqual({
		reads: true,
		written: ["error"],
	});
});

test.each<{
	name: string;
	/** The lines that come back from the gatherer, after the agent said `hello` at 5. */
	back: { name: string; text: string; sentAt: number }[];
	/** Which of them were written as the agent's. */
	byAgent: boolean[];
}>([
	{
		name: "the agent's line comes back as the agent's",
		back: [{ name: "me", text: "hello", sentAt: 5 }],
		byAgent: [true],
	},
	{
		name: "the person's line under the same name, with another sentAt, is not",
		back: [
			{ name: "me", text: "hello", sentAt: 6 },
			{ name: "me", text: "hello", sentAt: 5 },
		],
		byAgent: [false, true],
	},
	{
		name: "another's line with the same sentAt and text is not",
		back: [{ name: "you", text: "hello", sentAt: 5 }],
		byAgent: [false],
	},
	{
		name: "a line that comes back twice is the agent's once",
		back: [
			{ name: "me", text: "hello", sentAt: 5 },
			{ name: "me", text: "hello", sentAt: 5 },
		],
		byAgent: [true, false],
	},
])("$name", async ({ back, byAgent }) => {
	const inbox = { now: "" };
	const marked: boolean[] = [];
	let waited: (() => void) | undefined;
	const bridge = createAgentBridge({
		folder: {
			path: "demo/me",
			began: 0,
			write(event) {
				if (event.type === "chat") marked.push(event.byAgent === true);
			},
			async writeFile() {},
			readInbox: async () => utf8(inbox.now),
		},
		name: "me",
		room: "meeting/demo",
		id: "me",
		say: async () => ({ sentAt: 5 }),
		sleep: () =>
			new Promise((resolve) => {
				waited = resolve;
			}),
	});
	await settle();
	inbox.now = "hello\n";
	waited?.();
	await settle();
	for (const [number, line] of back.entries()) {
		bridge.chat({ number: BigInt(number), at: 9, ...line });
	}
	bridge.stop();
	expect(marked).toEqual(byAgent);
});

test("the first line names the room and the page's id, and the last is written once, at stop", async () => {
	const written: string[] = [];
	const bridge = createAgentBridge({
		folder: {
			path: "demo/me",
			began: 0,
			write: (event) => {
				written.push(agentLine(event, 0).replace(/"at":0,?/, ""));
			},
			async writeFile() {},
			readInbox: async () => new Uint8Array(),
		},
		name: "me",
		room: "meeting/demo",
		id: "me",
		say: async () => ({ sentAt: 1 }),
		sleep: () => new Promise(() => {}),
	});
	bridge.participant("joined", "p1");
	bridge.stop();
	bridge.stop();
	await settle();
	expect(written).toEqual([
		'{"id":"me","room":"meeting/demo","state":"began","type":"reader"}',
		'{"id":"p1","state":"joined","type":"participant"}',
		'{"state":"ended","type":"reader"}',
	]);
});
