// Two own relays that both ask for tokens (`--auth-key-hex`, the same key):
// the relay of the run, and a second one below it (`--upstream`), on the
// run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET. A page publishes at the relay
// above with a token of the room, and pages at the relay below subscribe
// with theirs. The relay below makes tokens of its own with the key for what
// it sends upstream (`own_token` of `crates/relay/src/auth.rs`): its SETUP
// and its SUBSCRIBE, in the namespace `*`.
//
// Usage: bun auth-edge.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run:
//
//   - The publishing page connects to the relay above with `pub` for
//     `xmoq/auth/*` and publishes `xmoq/auth`/`object`.
//   - A page at the relay below with `sub` for `xmoq/auth/*` subscribes: the
//     relay below connects upstream and is let in with its own token (no
//     `UNAUTHORIZED`, no session closed for authorization at the relay
//     above), its SUBSCRIBE is accepted, and the page gets the Object the
//     publisher sends.
//   - A page at the relay below with `sub` for `xmoq/other/*` is refused its
//     SUBSCRIBE by the relay below with REQUEST_ERROR 0x1 (UNAUTHORIZED):
//     the relay below checks its own viewers as before.

import { createHmac } from "node:crypto";
import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openPage,
	type RunContext,
	runCleanups,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24396" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The key of the run, which both relays are given: 32 bytes. */
const KEY_HEX = "22".repeat(32);
const NAMESPACE = "xmoq/auth";
const TRACK = "object";
const PAYLOAD_HEX = "61757468";

const hex = (bytes: Uint8Array): string =>
	[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

/**
 * A token of the room for `role` in `namespace`, good for 10 minutes, as the
 * Token structure in hex: Alias Type USE_VALUE (0x3), Token Type 0, the
 * value (§10.2.2), as `chat-tokens.ts` makes it.
 */
function token(role: "pub" | "sub", namespace: string): string {
	const expiry = Math.floor(Date.now() / 1000) + 600;
	const signed = `v1|${namespace}|${role}|${expiry}`;
	const mac = createHmac("sha256", Buffer.from(KEY_HEX, "hex"))
		.update(signed)
		.digest("hex");
	return `0300${hex(new TextEncoder().encode(`${signed}|${mac}`))}`;
}

async function connected(
	context: RunContext,
	url: string,
	label: string,
	tokenHex: string,
): Promise<Page> {
	const page = await openPage(context, label, "own-browser");
	await withTimeout(
		page.evaluate(
			([at, hash, tokenOf]) =>
				window.xmoqOwn.connectWithToken(at, hash, tokenOf),
			[url, context.certSha256Hex, tokenHex] satisfies [string, string, string],
		),
		`${label} to connect`,
	);
	return page;
}

/** Has the page subscribe: "ok", or the error it was refused with. */
async function subscribed(page: Page): Promise<string> {
	try {
		await withTimeout(
			page.evaluate(
				([namespace, track]) => window.xmoq.subscribe(namespace, track),
				[NAMESPACE, TRACK] satisfies [string, string],
			),
			"the SUBSCRIBE to be answered",
		);
		return "ok";
	} catch (error) {
		return error instanceof Error ? error.message : String(error);
	}
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	let summary: string;
	try {
		const auth = ["--auth-key-hex", KEY_HEX];
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
			relayArgs: auth,
		});
		const edge = await startSecondOwnRelay(
			String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
			context,
			auth,
		);
		const publisher = await connected(
			context,
			context.relayUrl,
			"publisher",
			token("pub", `${NAMESPACE}/*`),
		);
		await withTimeout(
			publisher.evaluate(
				([namespace, track]) => window.xmoq.publishTrack(namespace, track),
				[NAMESPACE, TRACK] satisfies [string, string],
			),
			"the page to publish",
		);

		const viewer = await connected(
			context,
			edge.url,
			"viewer",
			token("sub", `${NAMESPACE}/*`),
		);
		const answer = await subscribed(viewer);
		if (answer !== "ok") {
			throw new Error(`the viewer at the relay below was refused: ${answer}`);
		}
		await withTimeout(
			publisher.evaluate(
				(payload) => window.xmoq.sendObject(payload),
				PAYLOAD_HEX,
			),
			"the publisher to send",
		);
		const received = await withTimeout(
			viewer.evaluate(() => window.xmoq.firstObject()),
			"the viewer's Object",
		);
		if (received.payloadHex !== PAYLOAD_HEX) {
			throw new Error(
				`the viewer got ${received.payloadHex}, not ${PAYLOAD_HEX}`,
			);
		}
		// The fence: the stream that carried the Object down. A refusal of the
		// relay below's SUBSCRIBE, or of its session, came before it.
		const turnedDown = await context.relay.linesBefore(
			{ line: /^OPEN_DATA session=\d+ out=\d+ track_alias=\d+ group=\d+$/ },
			/^(SEND session=\d+ stream=\d+ REQUEST_ERROR error_code=0x1\b|CLOSE_SESSION )/,
		);
		if (turnedDown.length > 0) {
			throw new Error(
				`the relay above turned something of the relay below down: ${turnedDown.map(([line]) => line).join("; ")}`,
			);
		}

		const elsewhere = await connected(
			context,
			edge.url,
			"elsewhere",
			token("sub", "xmoq/other/*"),
		);
		const refused = await subscribed(elsewhere);
		// As `chat-tokens.ts` reads a refusal of the page's library.
		if (!/REQUEST_ERROR 0x1:/.test(refused)) {
			throw new Error(
				`the viewer with a token for another namespace was to be refused with UNAUTHORIZED (0x1), not: ${refused}`,
			);
		}

		summary = `both relays asked for tokens: the relay below was let in upstream with its own, its viewer with a token of the room got the Object of a page publishing above, and a viewer with a token for another namespace was refused at the relay below (${refused})`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ auth: "through-edge" })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED auth through an edge: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
