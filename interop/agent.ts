// The reader of a room for an agent (`crates/agent`, `xmoq-agent`) through
// the own relay and the gatherer (`xmoq-chat`): a page takes part in the chat
// of room r as `p`, and `xmoq-agent --chat-room broadcast/r --name claude` writes what
// happens as lines of JSON on stdout and in the files of `<dir>/events/`.
//
// Usage: bun agent.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-25):
//
//   - The page joined before the reader came: the reader's SUBSCRIBE_NAMESPACE
//     for `xmoq/chat/broadcast/r` is told of `xmoq/chat/broadcast/r/p`, and it writes
//     {"type":"joined","name":"p"}; nothing joins for its own name `claude`.
//   - The page says `hello from p`: the reader writes a `chat` line from p
//     with that text and the number the gatherer gave it (0).
//   - A line of the reader's stdin, `hello from claude`, reaches the page as
//     a line of `claude` (1), and the reader writes it as well.
//   - `//slash` is said as `/slash` (2); `/frame` and `/nothing` say nothing
//     and are written as `error` lines.
//   - The page leaves the chat: {"type":"left","name":"p"}.
//   - events.jsonl has the same lines as stdout, in the same order.
//
// Changed once `xmoq-picture` was in (2026-10-25): the reader runs with
// `--frames off`, there being no broadcast, and `/frame` is written as the
// error `frame: --frames off`. The pictures are another pairing's.
//
// Changed when a `broadcast` line came to say that the broadcast's catalog
// came (2026-10-26): nobody publishes `xmoq/broadcast/r` here, and none is
// written.
//
// Changed when the reader came to write a file of `events/` a second in
// place of `events.jsonl`, to read `<dir>/inbox.txt` beside stdin, and to
// mark its own lines (2026-10-27): a line added to `inbox.txt`, `hello from
// the inbox`, reaches the page as a line of `claude` (3); the lines of
// `claude` have `"by":"agent"` and that of p has none; the files of
// `events/`, in the order of their names, have the same lines as stdout.
// The reader puts `AGENT.md`, the text of `fixtures/agent-lines/AGENT.md`,
// in `--out`, and says on stderr the line to paste, with the folder in full.

import {
	appendFileSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	agentEventFiles,
	type LoggedProcess,
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnAgent,
	spawnChatGatherer,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, PageChatReport, XmoqPage } from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24399" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

/** The lines of JSON the reader wrote on stdout, without their `at`. */
function events(agent: LoggedProcess): string[] {
	return agent
		.matches(/^\{.*\}$/)
		.map((match) => match[0].replace(/"at":\d+,?/, ""));
}

/** Waits for the reader to write a line that has `fragment` in it. */
function waitForEvent(agent: LoggedProcess, fragment: string): Promise<string> {
	return pollFor(
		() => events(agent).find((line) => line.includes(fragment)),
		`xmoq-agent to write ${fragment}`,
	);
}

const report = (page: Page): Promise<PageChatReport> =>
	page.evaluate(() => window.xmoqOwn.chatReport());

/** Waits for the page to have the line `number`, and gives "number name text". */
async function pageLine(page: Page, number: number): Promise<string> {
	const line = await pollFor(async () => {
		const { lines } = await report(page);
		return lines.find((each) => each.number === String(number));
	}, `the page to have line ${number}`);
	return `${line.number} ${line.name} ${line.text}`;
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-agent-"));
	const out = join(directory, "out");
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const gatherer = spawnChatGatherer(
			context,
			"gatherer",
			join(directory, "chat.sqlite3"),
		);
		await gatherer.waitForLine(/^WATCHING prefix=xmoq\/chat$/);
		const page = await openConnectedPage(context, "page:p", "own-browser");
		await withTimeout(
			page.evaluate(() => window.xmoqOwn.joinChat("r", "p")),
			"the page to join",
		);
		await gatherer.waitForLine(/^SUBSCRIBED room="broadcast\/r" name="p"$/);

		const agent = spawnAgent(context, "agent:claude", [
			"--namespace",
			"xmoq/broadcast/r",
			"--chat-room",
			"broadcast/r",
			"--name",
			"claude",
			"--out",
			out,
			"--frames",
			"off",
			"--seconds",
			"60",
		]);
		const call = `Read ${join(realpathSync(directory), "out")}/AGENT.md and take part in the room as it says.`;
		await agent.waitForLine(
			new RegExp(`^${call.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`),
		);
		expect(
			"AGENT.md of --out",
			readFileSync(join(out, "AGENT.md"), "utf8"),
			readFileSync(
				join(import.meta.dir, "..", "fixtures", "agent-lines", "AGENT.md"),
				"utf8",
			),
		);
		await waitForEvent(agent, '"type":"joined"');
		await gatherer.waitForLine(
			/^SUBSCRIBED room="broadcast\/r" name="claude"$/,
		);

		const said: unknown = JSON.parse(
			await page.evaluate(() => window.xmoqOwn.chatSay("hello from p")),
		);
		expect(
			"what became of the page's line",
			JSON.stringify(said).includes('"sent"') ? "sent" : JSON.stringify(said),
			"sent",
		);
		await waitForEvent(agent, '"text":"hello from p"');

		agent.child.stdin.write("hello from claude\n");
		expect(
			"the page's line 1",
			await pageLine(page, 1),
			"1 claude hello from claude",
		);
		await waitForEvent(agent, '"text":"hello from claude"');
		agent.child.stdin.write("//slash\n");
		expect("the page's line 2", await pageLine(page, 2), "2 claude /slash");
		// Written once the line came back, so that the errors come after it.
		await waitForEvent(agent, '"text":"/slash"');
		appendFileSync(join(out, "inbox.txt"), "hello from the inbox\n");
		expect(
			"the page's line 3",
			await pageLine(page, 3),
			"3 claude hello from the inbox",
		);
		await waitForEvent(agent, '"text":"hello from the inbox"');
		agent.child.stdin.write("/frame\n/nothing\n");
		await waitForEvent(agent, '"message":"no command /nothing');

		await page.evaluate(() => window.xmoqOwn.chatLeave());
		await waitForEvent(agent, '"type":"left"');

		const written = events(agent);
		expect(
			"the joined line",
			written[0] ?? "none",
			'{"name":"p","type":"joined"}',
		);
		const ofP = written.find((line) => line.includes('"from":"p"')) ?? "none";
		for (const fragment of ['"number":0', '"text":"hello from p"']) {
			if (!ofP.includes(fragment)) {
				throw new Error(`the chat line of p: ${ofP}, without ${fragment}`);
			}
		}
		if (ofP.includes('"by"')) {
			throw new Error(`the chat line of p is marked: ${ofP}`);
		}
		const ofClaude = written.filter((line) => line.includes('"from":"claude"'));
		if (
			ofClaude.length !== 3 ||
			!ofClaude.every((line) => line.includes('"by":"agent"'))
		) {
			throw new Error(
				`the chat lines of claude, each marked: ${JSON.stringify(ofClaude)}`,
			);
		}
		const kinds = written
			.map((line) => /"type":"([a-z_]+)"/.exec(line)?.[1] ?? "?")
			.join(",");
		expect(
			"the kinds of lines written",
			kinds,
			"joined,chat,chat,chat,chat,error,error,left",
		);
		expect(
			"the error of /frame",
			written[5] ?? "none",
			'{"message":"frame: --frames off","type":"error"}',
		);
		if (written.some((line) => line.includes('"name":"claude"'))) {
			throw new Error("the reader wrote its own coming or leaving");
		}
		// The lines of the last second are in a file a second later.
		const inFiles = await pollFor(() => {
			const lines = agentEventFiles(out);
			return lines.length >= written.length ? lines : undefined;
		}, "the files of events/ to have every line of stdout");
		expect(
			"the files of events/, set against stdout",
			JSON.stringify(inFiles),
			JSON.stringify(written),
		);
		summary = `through the own relay and the gatherer, xmoq-agent wrote ${written.length} lines of JSON (${kinds}): p, who had joined before it came, as joined; p's line as chat 0; its own stdin lines as chat 1 and 2 (//slash said as /slash) and a line of its inbox.txt as chat 3, which the page had, its own marked as the agent's; /frame and an unknown command as errors; p's leaving as left; the files of events/ had the same lines; AGENT.md was in --out, and the line to paste on stderr`;
	} finally {
		await runCleanups();
		rmSync(directory, { recursive: true, force: true });
	}
	log("driver", `RESULT ${JSON.stringify({ relay: options.relay })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED agent: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
