// Pages chat over the tracks of the chat through a relay and the gatherer
// (`crates/chat`, `xmoq-chat`): each page is a participant with `joinChat`
// of `@xmoq/chat`, which the demo's pages use. It announces
// `xmoq/chat/r/<name>`, says its lines on `said`, and reads the room's
// `lines` with SUBSCRIBE and a Joining FETCH for the newest 16.
//
// Usage: bun chat-pages.ts [--relay own|moqtail] [--port N] [--verbose]
//
// Through moqtail's relay 0.15.0 the first SUBSCRIBE of page a for the
// room is refused with DOES_NOT_EXIST: that relay does not hold it under
// RENDEZVOUS_TIMEOUT until the gatherer's PUBLISH comes
// (docs/moqt-implementation-notes.md), and the page asks again 2 s later.
// The gatherer is not started again here, which that relay did not always
// answer (`chat.ts`).
//
// What has to be seen, written before the first run (2026-10-24):
//
//   - Pages a and b join room r; the gatherer follows each (`SUBSCRIBED`),
//     and each page has the gatherer's subscription to its `said` and its
//     own subscription to the room's `lines`.
//   - a, b, a, b say a line each, one after the other: the gatherer numbers
//     them 0 to 3, and both pages get all four, live, with those numbers and
//     names.
//   - a says a line of 513 bytes: the page keeps it back (`too-long`) and
//     the gatherer never sees it.
//   - b says 6 lines at once, 1.5 s after its last: the page sends 5 (4 to
//     8) and keeps the sixth back (`too-fast`); the gatherer drops none.
//   - a says 11 more, 250 ms apart (9 to 19).
//   - Page c comes then: its Joining FETCH brings the newest 16 lines, 4 to
//     19, as history, and nothing older. Through the own relay the gatherer
//     answers it (`FETCH_ANSWERED ... start=4,0 end=19,1 lines=16`;
//     added after the first run, which had it).
//   - c says a line: numbered 20, and all three pages get it live.
//   - Every page has each line once, and says by which way it came: a and
//     b had 0 to 20 live, c 4 to 19 as history and 20 live.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	captureGroup,
	type LoggedProcess,
	log,
	openConnectedPage,
	pollFor,
	type RunContext,
	runCleanups,
	spawnChatGatherer,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, PageChatReport } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24398" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const ROOM = "r";

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

const report = (page: Page): Promise<PageChatReport> =>
	page.evaluate(() => window.xmoqOwn.chatReport());

/** Opens a page, has it join the room as `name`, and waits until the gatherer listens to it and it follows the room. */
async function participant(
	context: RunContext,
	gatherer: LoggedProcess,
	name: string,
): Promise<Page> {
	const page = await openConnectedPage(context, `page:${name}`, "own-browser");
	await withTimeout(
		page.evaluate(([room, as]) => window.xmoqOwn.joinChat(room, as), [
			ROOM,
			name,
		] satisfies [string, string]),
		`page ${name} to join the room`,
	);
	await gatherer.waitForLine(
		new RegExp(`^SUBSCRIBED room="${ROOM}" name="${name}"$`),
	);
	await pollFor(async () => {
		const { heard, following } = await report(page);
		return heard && following ? true : undefined;
	}, `page ${name} to be heard and to follow the room`);
	return page;
}

/** Has the page say a line, and returns what became of it: "sent", or "kept-back <why>", or "not-sent <why>". */
async function say(page: Page, text: string): Promise<string> {
	const outcome: unknown = JSON.parse(
		await page.evaluate((line) => window.xmoqOwn.chatSay(line), text),
	);
	if (typeof outcome !== "object" || outcome === null) {
		throw new Error(`no outcome: ${String(outcome)}`);
	}
	const type = "type" in outcome ? String(outcome.type) : "?";
	return "why" in outcome ? `${type} ${String(outcome.why)}` : type;
}

const taken = /^LINE room="r" number=(\d+) name="([^"]*)" /;

/** Has the page say a line that is to be sent, and waits for the gatherer to number it. */
async function sayOne(
	gatherer: LoggedProcess,
	page: Page,
	name: string,
	number: number,
): Promise<void> {
	const before = gatherer.matches(taken).length;
	expect(
		`what became of line ${number} of ${name}`,
		await say(page, `${name}${number}`),
		"sent",
	);
	const numbered = await pollFor(
		() => gatherer.matches(taken)[before],
		`the gatherer to number line ${number}`,
	);
	expect(
		"the line numbered",
		`${captureGroup(numbered, 1)} ${captureGroup(numbered, 2)}`,
		`${number} ${name}`,
	);
}

/** "first-last how" for each run of lines of one way, by number; the lines a page had, each once. */
function described(lines: PageChatReport["lines"]): string {
	const sorted = [...lines].sort((a, b) => Number(a.number) - Number(b.number));
	const numbers = sorted.map((line) => line.number);
	if (new Set(numbers).size !== numbers.length) {
		throw new Error(`a line came twice: ${numbers.join(",")}`);
	}
	const runs: string[] = [];
	let first = sorted[0];
	for (let index = 1; index <= sorted.length; index++) {
		const line = sorted[index];
		const last = sorted[index - 1];
		if (first === undefined || last === undefined) break;
		const follows =
			line !== undefined &&
			line.how === first.how &&
			Number(line.number) === Number(last.number) + 1;
		if (follows) continue;
		runs.push(`${first.number}-${last.number} ${first.how}`);
		first = line;
	}
	return runs.join(", ");
}

/** Waits for the page to have `count` lines. */
async function lines(
	page: Page,
	name: string,
	count: number,
): Promise<PageChatReport> {
	return pollFor(async () => {
		const seen = await report(page);
		return seen.lines.length >= count ? seen : undefined;
	}, `page ${name} to have ${count} lines`);
}

async function main(): Promise<void> {
	if (options.relay !== "own" && options.relay !== "moqtail") {
		throw new Error(`--relay is own or moqtail, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-chat-pages-"));
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

		const a = await participant(context, gatherer, "a");
		const b = await participant(context, gatherer, "b");
		const turns: [Page, string, number][] = [
			[a, "a", 0],
			[b, "b", 1],
			[a, "a", 2],
			[b, "b", 3],
		];
		for (const [page, name, number] of turns) {
			await sayOne(gatherer, page, name, number);
		}
		for (const [page, name] of [
			[a, "a"],
			[b, "b"],
		] as const) {
			const seen = await lines(page, name, 4);
			expect(
				`the lines page ${name} had`,
				seen.lines
					.map((line) => line.number)
					.sort()
					.join(","),
				"0,1,2,3",
			);
		}

		expect(
			"a line of 513 bytes",
			await say(a, "x".repeat(513)),
			"kept-back too-long",
		);

		await new Promise((resolve) => setTimeout(resolve, 1_500));
		const burst = await Promise.all(
			Array.from({ length: 6 }, (_, index) => say(b, `fast${index}`)),
		);
		expect(
			"6 lines at once",
			burst.join(","),
			"sent,sent,sent,sent,sent,kept-back too-fast",
		);
		const ofB = await pollFor(() => {
			const numbered = gatherer
				.matches(taken)
				.filter((match) => Number(captureGroup(match, 1)) >= 4);
			return numbered.length >= 5 ? numbered : undefined;
		}, "the gatherer to number the 5 lines of b");
		expect(
			"the lines of the burst",
			ofB.map((match) => captureGroup(match, 1)).join(","),
			"4,5,6,7,8",
		);
		for (let number = 9; number <= 19; number++) {
			await sayOne(gatherer, a, "a", number);
			await new Promise((resolve) => setTimeout(resolve, 250));
		}
		if (gatherer.matches(/^LINE_DROPPED /).length > 0) {
			throw new Error("the gatherer dropped a line the pages sent");
		}

		const c = await participant(context, gatherer, "c");
		const history = await lines(c, "c", 16);
		expect(
			"what page c had on coming",
			described(history.lines),
			"4-19 history",
		);
		// The own relay has the latest Group alone, and passes the FETCH to
		// the gatherer; moqtail's relay 0.15.0 answered it from what it held,
		// and the gatherer was not asked (in each of 12 runs).
		const answered = gatherer
			.matches(/^FETCH_ANSWERED room="r" (.*)$/)
			.map((match) => captureGroup(match, 1))
			.join("; ");
		expect(
			"what the gatherer answered",
			answered,
			options.relay === "own"
				? "start=4,0 end=19,1 lines=16 unknown_through=none"
				: "",
		);
		await sayOne(gatherer, c, "c", 20);

		const ways: string[] = [];
		for (const [page, name, wanted] of [
			[a, "a", "0-20 live"],
			[b, "b", "0-20 live"],
			[c, "c", "4-19 history, 20-20 live"],
		] as const) {
			const seen = await lines(page, name, name === "c" ? 17 : 21);
			expect(`the lines of page ${name}`, described(seen.lines), wanted);
			if (seen.malformed.length > 0) {
				throw new Error(
					`page ${name} had no line of ${seen.malformed.join(", ")}`,
				);
			}
			ways.push(`${name}: ${described(seen.lines)}`);
		}
		summary = `through the ${options.relay} relay and the gatherer, pages a and b said lines on their tracks and had the room's lines live, numbered by the gatherer; a line of 513 bytes and the sixth of 6 at once were kept back by the page; page c, coming after 20 lines, had the newest 16 by its Joining FETCH and the next live (${ways.join("; ")})`;
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
		`FAILED chat-pages: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
