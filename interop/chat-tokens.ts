// Pages join the chat through the own relay when it asks for tokens
// (`--auth-key-hex`), with the two tokens the demo's pages get from the room:
// `sub` for `xmoq/chat/r` on the connection, and `pub` for
// `xmoq/chat/r/<name>` on the PUBLISH_NAMESPACE alone (`publishNamespaceToken`
// of `joinChat`). No gatherer: one that sends tokens is not in this tree
// yet, so a page publishes the room's `lines` in its place, with a `pub`
// token for `xmoq/chat/r`. The tokens are made here with the relay's key,
// in the form `crates/relay/src/auth.rs` reads and
// `spikes/durable-objects-room/worker.js` writes.
//
// Usage: bun chat-tokens.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-24):
//
//   - The page in the gatherer's place connects with `pub` for
//     `xmoq/chat/r`, publishes `lines`, and sends line 0.
//   - Page a connects with `sub` for `xmoq/chat/r` and joins as `a` with
//     `pub` for `xmoq/chat/r/a`: its PUBLISH_NAMESPACE is accepted, it follows
//     the room (SUBSCRIBE), has line 0 by its Joining FETCH, and line 1,
//     sent then, live.
//   - Page b, with the same `sub` and `pub` for `xmoq/chat/r/a`, is refused
//     its PUBLISH_NAMESPACE as `b` with REQUEST_ERROR 0x1 (UNAUTHORIZED).
//   - Page c, with `sub` and no token of its own, is refused the same way:
//     the connection's token allows no announcing.
//   - Page d connects with `pub` for `xmoq/chat/r/d` and gives the same
//     token to `joinChat` (the library sends it once): its PUBLISH_NAMESPACE
//     is accepted, and its SUBSCRIBE for `lines` refused with 0x1.

import { createHmac } from "node:crypto";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	log,
	openPage,
	pollFor,
	type RunContext,
	runCleanups,
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

/** The key of the run, which the relay is given: 32 bytes. */
const KEY_HEX = "11".repeat(32);

const hex = (bytes: Uint8Array): string =>
	[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

/**
 * A token of the room for `role` in `namespace`, good for 10 minutes, as the
 * Token structure in hex: Alias Type USE_VALUE (0x3), Token Type 0, the
 * value (§10.2.2).
 */
function token(role: "pub" | "sub", namespace: string): string {
	const expiry = Math.floor(Date.now() / 1000) + 600;
	const signed = `v1|${namespace}|${role}|${expiry}`;
	const mac = createHmac("sha256", Buffer.from(KEY_HEX, "hex"))
		.update(signed)
		.digest("hex");
	return `0300${hex(new TextEncoder().encode(`${signed}|${mac}`))}`;
}

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

/** A line of `lines` as the gatherer writes it, in hex. */
const linePayload = (text: string): string =>
	hex(
		new TextEncoder().encode(
			JSON.stringify({ name: "g", text, sentAt: 1000, at: 1000 }),
		),
	);

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

/** "refused 0x1" for an UNAUTHORIZED refusal, else the text as it came. */
const refusal = (text: string): string =>
	/REQUEST_ERROR 0x1:/.test(text) ? "refused 0x1" : text;

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(
			`--relay is own: only the own relay checks tokens, not ${options.relay}`,
		);
	}
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
		relayArgs: ["--auth-key-hex", KEY_HEX],
	});
	let summary: string;
	try {
		const room = await connected(
			context,
			"page:lines",
			token("pub", "xmoq/chat/r"),
		);
		await withTimeout(
			room.evaluate(() => window.xmoq.publishTrack("xmoq/chat/r", "lines")),
			"the page in the gatherer's place to publish lines",
		);
		await room.evaluate(
			(payload) => window.xmoqOwn.sendObjectIn("0", payload, true, [], 0),
			linePayload("before"),
		);

		const a = await connected(context, "page:a", token("sub", "xmoq/chat/r"));
		expect(
			"page a joining with pub for xmoq/chat/r/a",
			await join(a, "a", token("pub", "xmoq/chat/r/a")),
			"joined",
		);
		const history = await pollFor(async () => {
			const seen = await report(a);
			return seen.following && seen.lines.length >= 1 ? seen : undefined;
		}, "page a to follow the room and have line 0");
		await room.evaluate(
			(payload) => window.xmoqOwn.sendObjectIn("1", payload, true, [], 0),
			linePayload("after"),
		);
		const both = await pollFor(async () => {
			const seen = await report(a);
			return seen.lines.length >= 2 ? seen : undefined;
		}, "page a to have line 1");
		const lines = both.lines
			.map((line) => `${line.number} ${line.how}`)
			.join(", ");
		expect("the lines of page a", lines, "0 history, 1 live");

		const b = await connected(context, "page:b", token("sub", "xmoq/chat/r"));
		expect(
			"page b joining as b with pub for xmoq/chat/r/a",
			refusal(await join(b, "b", token("pub", "xmoq/chat/r/a"))),
			"refused 0x1",
		);
		const c = await connected(context, "page:c", token("sub", "xmoq/chat/r"));
		expect(
			"page c joining with no token of its own",
			refusal(await join(c, "c", undefined)),
			"refused 0x1",
		);
		const ofD = token("pub", "xmoq/chat/r/d");
		const d = await connected(context, "page:d", ofD);
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
		summary = `through the own relay with --auth-key-hex, page a with sub for xmoq/chat/r on its connection and pub for xmoq/chat/r/a on its PUBLISH_NAMESPACE joined, followed the room and had ${lines} (following: ${history.following}); b with pub for another name and c with none were refused their PUBLISH_NAMESPACE with 0x1; d with pub alone announced and was refused its SUBSCRIBE with 0x1`;
	} finally {
		await runCleanups();
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
