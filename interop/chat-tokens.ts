// Pages join the chat through the own relay when it asks for tokens
// (`--auth-key-hex`), with the two tokens the demo's pages get from the room:
// `sub` for `xmoq/chat/r` on the connection, and `pub` for
// `xmoq/chat/r/<name>` on the PUBLISH_NAMESPACE alone (`publishNamespaceToken`
// of `joinChat`). The gatherer (`xmoq-chat`) runs with the same key and makes
// tokens of its own (`sub` and `pub` in `xmoq/chat/*`), as in the demo with
// `--auth`. The pages' tokens are made here with the relay's key, in the form
// `crates/room-token` reads and writes and `spikes/durable-objects-room/worker.js`
// writes.
//
// Usage: bun chat-tokens.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run with the gatherer
// (2026-10-25):
//
//   - The gatherer with `--auth-key-hex` is told of `xmoq/chat`
//     (`WATCHING`): the relay took its SETUP and its SUBSCRIBE_NAMESPACE.
//   - Pages a and e connect with `sub` for `xmoq/chat/r` and join as `a` and
//     `e` with `pub` for their own name: their PUBLISH_NAMESPACE is
//     accepted, the gatherer subscribes to each `said` (`SUBSCRIBED`), and
//     each follows the room's `lines`, which the gatherer publishes.
//   - a says a line, then e: the gatherer numbers them 0 and 1, and both
//     pages have both, live.
//   - Page f comes then with the same two tokens: its Joining FETCH, which
//     the relay passes to the gatherer, brings 0 and 1 as history.
//   - Page b, with `sub` and `pub` for `xmoq/chat/r/a`, is refused its
//     PUBLISH_NAMESPACE as `b` with REQUEST_ERROR 0x1 (UNAUTHORIZED).
//   - Page c, with `sub` and no token of its own, is refused the same way:
//     the connection's token allows no announcing.
//   - Page d connects with `pub` for `xmoq/chat/r/d` and gives the same
//     token to `joinChat` (the library sends it once): its PUBLISH_NAMESPACE
//     is accepted, and its SUBSCRIBE for `lines` refused with 0x1.
//   - Nothing of b, c or d reaches the gatherer as a line.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join as joinPath } from "node:path";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	captureGroup,
	type LoggedProcess,
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openPage,
	pollFor,
	type RunContext,
	roomToken,
	runCleanups,
	spawnChatGatherer,
	startRun,
	startSecondOwnRelay,
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
		// The pages connect to a second own relay with the same key, below
		// the relay of the run (`--upstream`), on the run's port plus
		// MOQLIVEMOCK_SIDE_PORT_OFFSET; the gatherer stays with the relay of
		// the run. The relay below checks the pages' tokens, passes their
		// announcements up with a token of its own, and takes the gatherer's
		// SUBSCRIBEs through the relay above without one; what has to be
		// seen is as without it.
		"through-edge": { type: "boolean", default: false },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The key of the run, which the relay is given: 32 bytes. */
const KEY_HEX = "11".repeat(32);

/** A token of the room for `role` in `namespace`, with the run's key (`roomToken`). */
const token = (role: "pub" | "sub", namespace: string): string =>
	roomToken(KEY_HEX, role, namespace);

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

const report = (page: Page): Promise<PageChatReport> =>
	page.evaluate(() => window.xmoqOwn.chatReport());

async function connected(
	context: RunContext,
	label: string,
	tokenHex: string,
): Promise<Page> {
	const page = await openPage(context, label, "own-browser");
	await withTimeout(
		page.evaluate(
			([url, hash, tokenOf]) =>
				window.xmoqOwn.connectWithToken(url, hash, tokenOf),
			[context.relayUrl, context.certSha256Hex, tokenHex] satisfies [
				string,
				string,
				string,
			],
		),
		`${label} to connect`,
	);
	return page;
}

/** Has the page join room r as `name`: "joined", or the error it was refused with. */
async function join(
	page: Page,
	name: string,
	publishNamespaceTokenHex: string | undefined,
): Promise<string> {
	try {
		await withTimeout(
			page.evaluate(
				([as, tokenOf]) => window.xmoqOwn.joinChat("r", as, undefined, tokenOf),
				[name, publishNamespaceTokenHex] satisfies [string, string | undefined],
			),
			`page ${name} to join`,
		);
		return "joined";
	} catch (error) {
		return error instanceof Error ? error.message : String(error);
	}
}

/** Opens a page with both tokens, has it join room r as `name`, and waits until the gatherer listens to it and it follows the room. */
async function participant(
	context: RunContext,
	gatherer: LoggedProcess,
	name: string,
): Promise<Page> {
	const page = await connected(
		context,
		`page:${name}`,
		token("sub", "xmoq/chat/r"),
	);
	expect(
		`page ${name} joining with pub for xmoq/chat/r/${name}`,
		await join(page, name, token("pub", `xmoq/chat/r/${name}`)),
		"joined",
	);
	await gatherer.waitForLine(
		new RegExp(`^SUBSCRIBED room="r" name="${name}"$`),
	);
	await pollFor(async () => {
		const { heard, following } = await report(page);
		return heard && following ? true : undefined;
	}, `page ${name} to be heard and to follow the room`);
	return page;
}

const taken = /^LINE room="r" number=(\d+) name="([^"]*)" /;

/** Has the page say a line, and waits for the gatherer to number it `number`. */
async function sayOne(
	gatherer: LoggedProcess,
	page: Page,
	name: string,
	number: number,
): Promise<void> {
	const before = gatherer.matches(taken).length;
	const outcome: unknown = JSON.parse(
		await page.evaluate(
			(line) => window.xmoqOwn.chatSay(line),
			`${name}${number}`,
		),
	);
	const type =
		typeof outcome === "object" && outcome !== null && "type" in outcome
			? String(outcome.type)
			: JSON.stringify(outcome);
	expect(`what became of the line of ${name}`, type, "sent");
	const numbered = await pollFor(
		() => gatherer.matches(taken)[before],
		`the gatherer to number the line of ${name}`,
	);
	expect(
		"the line numbered",
		`${captureGroup(numbered, 1)} ${captureGroup(numbered, 2)}`,
		`${number} ${name}`,
	);
}

/** "number how" for each line the page has, by number. */
async function linesOf(
	page: Page,
	count: number,
	label: string,
): Promise<string> {
	const seen = await pollFor(async () => {
		const { lines } = await report(page);
		return lines.length >= count ? lines : undefined;
	}, `${label} to have ${count} lines`);
	return [...seen]
		.sort((x, y) => Number(x.number) - Number(y.number))
		.map((line) => `${line.number} ${line.name} ${line.how}`)
		.join(", ");
}

/** "refused 0x1" for an UNAUTHORIZED refusal, else the text as it came. */
const refusal = (text: string): string =>
	/REQUEST_ERROR 0x1:/.test(text) ? "refused 0x1" : text;

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(
			`--relay is own: only the own relay checks tokens, not ${options.relay}`,
		);
	}
	const directory = mkdtempSync(joinPath(tmpdir(), "xmoq-chat-tokens-"));
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
			relayArgs: ["--auth-key-hex", KEY_HEX],
		});
		const pages: RunContext = options["through-edge"]
			? {
					...context,
					relayUrl: (
						await startSecondOwnRelay(
							String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
							context,
							["--auth-key-hex", KEY_HEX],
						)
					).url,
				}
			: context;
		const gatherer = spawnChatGatherer(
			context,
			"gatherer",
			joinPath(directory, "chat.sqlite3"),
			["--auth-key-hex", KEY_HEX],
		);
		await gatherer.waitForLine(/^WATCHING prefix=xmoq\/chat$/);

		const a = await participant(pages, gatherer, "a");
		const e = await participant(pages, gatherer, "e");
		await sayOne(gatherer, a, "a", 0);
		await sayOne(gatherer, e, "e", 1);
		const live = "0 a live, 1 e live";
		expect("the lines of page a", await linesOf(a, 2, "page a"), live);
		expect("the lines of page e", await linesOf(e, 2, "page e"), live);

		const f = await participant(pages, gatherer, "f");
		expect(
			"the lines of page f, which came later",
			await linesOf(f, 2, "page f"),
			"0 a history, 1 e history",
		);
		const [answered] = gatherer.matches(/^FETCH_ANSWERED room="r" (.*)$/);
		if (answered === undefined) {
			throw new Error("the gatherer answered no FETCH for page f");
		}

		const b = await connected(pages, "page:b", token("sub", "xmoq/chat/r"));
		expect(
			"page b joining as b with pub for xmoq/chat/r/a",
			refusal(await join(b, "b", token("pub", "xmoq/chat/r/a"))),
			"refused 0x1",
		);
		const c = await connected(pages, "page:c", token("sub", "xmoq/chat/r"));
		expect(
			"page c joining with no token of its own",
			refusal(await join(c, "c", undefined)),
			"refused 0x1",
		);
		const ofD = token("pub", "xmoq/chat/r/d");
		const d = await connected(pages, "page:d", ofD);
		expect(
			"page d joining with pub on the connection and the same on the request",
			await join(d, "d", ofD),
			"joined",
		);
		const ended = await pollFor(async () => {
			const { lastEnd } = await report(d);
			return lastEnd === null ? undefined : lastEnd;
		}, "the SUBSCRIBE of page d to be answered");
		expect(
			"the SUBSCRIBE of page d, with pub alone",
			refusal(ended.replace(/^refused: /, "")),
			"refused 0x1",
		);
		const others = gatherer
			.matches(taken)
			.map((match) => captureGroup(match, 2))
			.filter((name) => name !== "a" && name !== "e");
		expect("the lines the gatherer took of others", others.join(","), "");
		const where = options["through-edge"]
			? "with the pages at an own relay below the gatherer's, both"
			: "through the own relay";
		summary = `${where} with --auth-key-hex and the gatherer with the same key, pages a and e with sub for xmoq/chat/r on their connection and pub for their name on their PUBLISH_NAMESPACE joined, and each had both lines live (${live}); page f, which came later, had them by its Joining FETCH, which the gatherer answered (${answered[1]}); b with pub for another name and c with none were refused their PUBLISH_NAMESPACE with 0x1; d with pub alone announced and was refused its SUBSCRIBE with 0x1`;
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
		`FAILED chat-tokens: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
