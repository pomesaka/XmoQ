// Interop driver: a subscriber asks for a new Group with NEW_GROUP_REQUEST
// (transport §10.2.13), and the own relay passes it on to the publisher.
//
// Usage: bun new-group.ts --publisher own-rust|own-browser [--relay own] [--port N] [--verbose]
//
// The publisher publishes the track with the Track Property DYNAMIC_GROUPS =
// 1 (§12.6). An own-rust subscriber then sends its SUBSCRIBE with
// NEW_GROUP_REQUEST = 0. The relay has to send the publisher a
// REQUEST_UPDATE with that parameter on the stream of its PUBLISH, the
// publisher has to answer REQUEST_OK and report the request to its
// application, and the subscription has to go on: the Object the publisher
// sends afterwards reaches the subscriber.
//
// Neither publisher begins a Group for the request: own-rust publishes what
// this driver tells it to, and the page of this run has one Object to send.
// What is checked is that the request arrives and breaks nothing.
//
// The run waits for lines of the relay's log (`interop/disconnect.ts`).

import { parseArgs } from "node:util";
import {
	log,
	NAMESPACE,
	openConnectedPage,
	pollFor,
	type RunContext,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		publisher: { type: "string", default: "own-rust" },
		port: { type: "string", default: "24420" },
		// The matrix names the relay for every run; this one has the own relay only.
		relay: { type: "string", default: "own" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const PAYLOAD_HEX = "6e65772067726f7570";

/** A publisher that has published the track: how to learn what it was asked, and to send its Object. */
interface Publishing {
	/** Resolves with the value of the first NEW_GROUP_REQUEST it reports. */
	asked(): Promise<string>;
	send(): Promise<void>;
}

async function publish(
	context: RunContext,
	publisher: string,
): Promise<Publishing> {
	if (publisher === "own-browser") {
		const page = await openConnectedPage(
			context,
			"pub:own-browser",
			"own-browser",
		);
		await withTimeout(
			page.evaluate(
				([namespace, track]) =>
					window.xmoqOwn.publishDynamicTrack(namespace, track),
				[NAMESPACE, TRACK] satisfies [string, string],
			),
			"the page to publish the track",
		);
		return {
			asked: () =>
				pollFor(
					async () =>
						(await page.evaluate(() => window.xmoqOwn.newGroupRequests()))[0],
					"the page to be told of a NEW_GROUP_REQUEST",
				),
			send: async () => {
				await withTimeout(
					page.evaluate(([payload]) => window.xmoq.sendObject(payload), [
						PAYLOAD_HEX,
					] satisfies [string]),
					"the page to send its Object",
				);
			},
		};
	}
	if (publisher !== "own-rust") {
		throw new Error(`--publisher is own-rust or own-browser, not ${publisher}`);
	}
	const rust = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish",
		TRACK,
		[
			"--payload-hex",
			PAYLOAD_HEX,
			"--dynamic-groups",
			"yes",
			"--timeout-secs",
			"40",
		],
	);
	await rust.waitForLine(/^PUBLISHED /);
	return {
		asked: async () => {
			const [, group = ""] = await rust.waitForLine(
				/^NEW_GROUP_REQUESTED group=(\d+)$/,
			);
			return group;
		},
		send: () => {
			rust.child.stdin.write("send\n");
			return Promise.resolve();
		},
	};
}

async function run(context: RunContext, publisher: string): Promise<string> {
	const publishing = await publish(context, publisher);
	const subscriber = spawnRustClient(
		context,
		"sub:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		["--new-group-request", "0", "--objects", "1", "--timeout-secs", "40"],
	);
	await subscriber.waitForLine(/^SUBSCRIBED /);
	// The publisher is session 0. The relay sends it the update on the
	// stream of its PUBLISH, and reads its REQUEST_OK there.
	const [sent] = await context.relay.waitForLine(
		/^SEND session=0 stream=\d+ REQUEST_UPDATE$/,
	);
	const group = await publishing.asked();
	if (group !== "0") {
		throw new Error(`the publisher was to be asked with 0, not ${group}`);
	}
	// The subscription goes on, and with it the session of the publisher:
	// the relay took the answer on the stream of the PUBLISH for what it is.
	await publishing.send();
	const [, payload = ""] = await subscriber.waitForLine(
		/^OBJECT .*payload_hex=([0-9a-f]+)/,
	);
	if (payload !== PAYLOAD_HEX) {
		throw new Error(
			`the subscriber was to get the publisher's Object, not ${payload}`,
		);
	}
	if (context.relay.hasPrinted(/^(CLOSE_SESSION|SESSION_FAILED) /)) {
		throw new Error("the relay was to close no session");
	}
	return `own-rust subscribed with NEW_GROUP_REQUEST = 0 to a track ${publisher} published with DYNAMIC_GROUPS = 1: the relay sent the publisher a REQUEST_UPDATE on the stream of its PUBLISH (${sent}), the publisher answered REQUEST_OK and reported the request with the value ${group}, and the Object it sent afterwards reached the subscriber`;
}

async function main(): Promise<void> {
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients:
				options.publisher === "own-browser"
					? ["own-browser", "own-rust"]
					: ["own-rust"],
			verbose: options.verbose,
		});
		summary = await run(context, options.publisher);
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ publisher: options.publisher })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED new-group ${options.publisher}: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
