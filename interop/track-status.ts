// TRACK_STATUS (draft 18 §10.14) through a relay, asked by an own client
// (`--client own-rust`, the Rust client's `--mode track-status`, or
// `--client own-browser`, `MoqtConnection.trackStatus`).
//
// Usage: bun track-status.ts --relay own|moqtail --client own-rust|own-browser [--port N] [--verbose]
//
// The own Rust client publishes the track `object` of `xmoq/phase0` with
// PUBLISH and the Track Property DYNAMIC_GROUPS = 1 (`--dynamic-groups yes`);
// it announces the namespace as well, and refuses every TRACK_STATUS it is
// asked with NOT_SUPPORTED. The client asks four times. What each answer is
// to be, written before the first run (2026-10-25), as the client prints it:
//
//   1. `object`, before any Object.
//      own: `TRACK_STATUS_OK largest=none track_properties=1`. The relay
//        answers a track with a source itself, with what its SUBSCRIBE_OK
//        would have: no LARGEST_OBJECT before a Location is known
//        (§10.2.11), and the Track Properties of the source (§9.6).
//      moqtail: `TRACK_STATUS_OK largest=0,0 track_properties=0`
//        (`fixtures/moqt-18/` capture P: {0, 0} before any Object, and no
//        Track Properties).
//   2. `object`, after a subscriber had Object 0 of Group 0.
//      own: `largest=0,0 track_properties=1`; moqtail: as in 1.
//   3. `other`, a track of the namespace nobody published. Both relays ask
//      the publisher, which announced the namespace, and it refuses.
//      own: `TRACK_STATUS_REFUSED error_code=0x10`: the refusal is passed on
//        as for a SUBSCRIBE, and a NOT_SUPPORTED becomes DOES_NOT_EXIST
//        (docs/moqt-implementation-notes.md, "TRACK_STATUS in the relay").
//      moqtail: `TRACK_STATUS_REFUSED error_code=0x3`, passed on as it came.
//   4. `nothing` of the namespace `nobody`, which nobody announced: both
//      `TRACK_STATUS_REFUSED error_code=0x10`.

import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	type LoggedProcess,
	log,
	NAMESPACE,
	openConnectedPage,
	type RunContext,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		client: { type: "string", default: "own-rust" },
		port: { type: "string", default: "48100" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

type Relay = "own" | "moqtail";
type Asker = "own-rust" | "own-browser";

/** What each of the four questions is answered with, by relay. */
const EXPECTED: Record<Relay, [string, string, string, string]> = {
	own: [
		"TRACK_STATUS_OK largest=none track_properties=1",
		"TRACK_STATUS_OK largest=0,0 track_properties=1",
		"TRACK_STATUS_REFUSED error_code=0x10",
		"TRACK_STATUS_REFUSED error_code=0x10",
	],
	moqtail: [
		"TRACK_STATUS_OK largest=0,0 track_properties=0",
		"TRACK_STATUS_OK largest=0,0 track_properties=0",
		"TRACK_STATUS_REFUSED error_code=0x3",
		"TRACK_STATUS_REFUSED error_code=0x10",
	],
};

const ANSWER = /^TRACK_STATUS_(OK|REFUSED) .*$/;

/** One TRACK_STATUS, as the client prints its answer. */
async function asked(
	context: RunContext,
	asker: Asker,
	page: Page | undefined,
	namespace: string,
	name: string,
): Promise<string> {
	if (asker === "own-browser" && page !== undefined) {
		return withTimeout(
			page.evaluate(([ns, track]) => window.xmoqOwn.trackStatus(ns, track), [
				namespace,
				name,
			] satisfies [string, string]),
			`the page's TRACK_STATUS for ${name}`,
		);
	}
	const client = spawnRustClient(
		context,
		`status:${name}`,
		"own-rust",
		"track-status",
		name,
		["--namespace", namespace],
	);
	const [line] = await client.waitForLine(ANSWER);
	return line;
}

async function main(): Promise<void> {
	const relay = options.relay;
	const asker = options.client;
	if (relay !== "own" && relay !== "moqtail") {
		throw new Error(`--relay is own or moqtail, not ${relay}`);
	}
	if (asker !== "own-rust" && asker !== "own-browser") {
		throw new Error(`--client is own-rust or own-browser, not ${asker}`);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay,
			port: options.port,
			clients:
				asker === "own-browser" ? ["own-rust", "own-browser"] : ["own-rust"],
			verbose: options.verbose,
		});
		const publisher: LoggedProcess = spawnRustClient(
			context,
			"pub:own-rust",
			"own-rust",
			"publish",
			"object",
			["--payload-hex", "6869", "--dynamic-groups", "yes"],
		);
		await publisher.waitForLine(/^PUBLISHED /);
		const page =
			asker === "own-browser"
				? await openConnectedPage(context, "asker:own-browser", "own-browser")
				: undefined;
		const answers: string[] = [];
		answers.push(await asked(context, asker, page, NAMESPACE, "object"));
		const subscriber = spawnRustClient(
			context,
			"sub:own-rust",
			"own-rust",
			"subscribe",
			"object",
			["--objects", "1"],
		);
		await subscriber.waitForLine(/^SUBSCRIBED /);
		publisher.child.stdin.write("send\n");
		await subscriber.waitForLine(/^OBJECT /);
		answers.push(await asked(context, asker, page, NAMESPACE, "object"));
		answers.push(await asked(context, asker, page, NAMESPACE, "other"));
		answers.push(await asked(context, asker, page, "nobody", "nothing"));
		const wanted = EXPECTED[relay];
		answers.forEach((answer, index) => {
			if (answer !== wanted[index]) {
				throw new Error(
					`question ${index + 1} was answered with "${answer}", not "${wanted[index]}"`,
				);
			}
		});
		summary = `${asker} asked the ${relay} relay four times: ${answers.join("; ")}`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ relay, client: asker })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED track-status: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
