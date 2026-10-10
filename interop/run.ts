// Interop driver: moves one MOQT Object through a relay between two clients,
// at least one of which is an own client, and compares the received payload
// with the sent one. `matrix.ts` runs every pairing with it.
//
// Usage: bun run.ts --publisher <client> --subscriber <client> [--relay moqtail|own]
//                   [--subscribers N] [--port N] [--subscribe-track NAME]
//                   [--await-publish-done] [--verbose]
//   <client>: own-browser | own-rust | moqtail-browser | moqtail-rust
//   --subscribers: how many subscribers of that kind subscribe; each has to get
//     the Object.
//   --subscribe-track: track the subscriber asks for (default: the published one).
//     Another name makes the relay answer REQUEST_ERROR, and the run fails by design.
//   --await-publish-done: after the transfer, makes the publisher leave and
//     waits until the subscriber, which has to be an own client, has seen
//     PUBLISH_DONE.
//
// The moqtail relay and clients live under peers/moqtail, the own relay in
// crates/relay and the own Rust client in crates/moqt-client; build them with
// the steps in README.md.

import { randomBytes } from "node:crypto";
import { parseArgs } from "node:util";
import { startPublisher, startSubscriber } from "./clients.ts";
import {
	type Client,
	DISCONNECT_TIMEOUT_MS,
	isOwnClient,
	log,
	parseClient,
	runCleanups,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		publisher: { type: "string", default: "own-browser" },
		subscriber: { type: "string", default: "moqtail-rust" },
		relay: { type: "string", default: "moqtail" },
		subscribers: { type: "string", default: "1" },
		port: { type: "string", default: "44330" },
		"await-publish-done": { type: "boolean", default: false },
		"subscribe-track": { type: "string", default: TRACK },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

interface RunResult {
	publisher: Client;
	subscriber: Client;
	/** How many subscribers of that kind took part. */
	subscribers: number;
	/** Whether every subscriber received the sent payload. */
	match: boolean;
	sentText: string;
	receivedText: string;
	sentHex: string;
	/** What the first subscriber that differs received, or the sent payload. */
	receivedHex: string;
	/** What each subscriber reported about the PUBLISH_DONE, with --await-publish-done. */
	subscriptionEnd: string[] | null;
	relay: string;
	relayVersion: string;
	/** Null in a run between two Rust clients. */
	browserVersion: string | null;
}

async function transferOneObject(
	publisher: Client,
	subscriber: Client,
	subscriberCount: number,
	subscribeTrack: string,
): Promise<RunResult> {
	const payload = Buffer.from(
		`xmoq-phase0b ${new Date().toISOString()} ${randomBytes(8).toString("hex")}`,
	);
	const payloadHex = payload.toString("hex");
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: [publisher, subscriber],
		verbose: options.verbose,
	});

	const run = { context, payloadHex };
	const awaitsPublishDone = options["await-publish-done"];
	if (awaitsPublishDone && !isOwnClient(subscriber)) {
		throw new Error(
			`--await-publish-done needs an own subscriber, not ${subscriber}`,
		);
	}

	// The publisher offers the track first: the relay rejects a SUBSCRIBE for
	// a track nobody has published.
	const publishing = await startPublisher(publisher, run);
	// The subscribers are live before the Object is sent: the Largest Object
	// filter does not replay what was published earlier.
	const subscribing = [];
	for (let index = 0; index < subscriberCount; index++) {
		const label =
			subscriberCount === 1
				? `sub:${subscriber}`
				: `sub${index + 1}:${subscriber}`;
		subscribing.push(
			await startSubscriber(subscriber, label, run, {
				track: subscribeTrack,
				// A subscriber that is not asked how its subscription ended leaves
				// once it has the Object.
				...(awaitsPublishDone ? {} : { leavesAfterObjects: 1 }),
			}),
		);
	}
	const group = await withTimeout(publishing.send(), "the publisher to send");
	const received = await Promise.all(
		subscribing.map((each) => each.object(group)),
	);
	const receivedHex = received.find((hex) => hex !== payloadHex) ?? payloadHex;
	let subscriptionEnd: string[] | null = null;
	if (awaitsPublishDone) {
		await publishing.end();
		subscriptionEnd = await Promise.all(
			subscribing.map(async (each) => {
				// WHY the longer wait: moqtail-rust exits right after it closes
				// its session, and where the close was not sent the relay ended
				// the track 12 seconds later (seen once for this pairing).
				const outcome = await each.outcome(DISCONNECT_TIMEOUT_MS);
				if (outcome.type !== "publish-done") {
					throw new Error(
						`${each.label}: the subscription ended without PUBLISH_DONE: ${outcome.reported}`,
					);
				}
				return outcome.reported;
			}),
		);
	}

	return {
		publisher,
		subscriber,
		subscribers: subscriberCount,
		match: receivedHex === payloadHex,
		sentText: payload.toString(),
		receivedText: Buffer.from(receivedHex, "hex").toString(),
		sentHex: payloadHex,
		receivedHex,
		subscriptionEnd,
		relay: options.relay,
		relayVersion: context.relayVersion,
		browserVersion: context.pages?.browser.version() ?? null,
	};
}

async function main(): Promise<void> {
	const publisher = parseClient("publisher", options.publisher);
	const subscriber = parseClient("subscriber", options.subscriber);
	if (!isOwnClient(publisher) && !isOwnClient(subscriber)) {
		throw new Error(
			"one side has to be an own client; a moqtail pair checks nothing of this repository",
		);
	}
	// WHY the digits are checked: `Number("")` is 0.
	if (!/^[1-9][0-9]?$/.test(options.subscribers)) {
		throw new Error(`--subscribers "${options.subscribers}" is not 1 to 99`);
	}
	const subscriberCount = Number(options.subscribers);
	let result: RunResult;
	try {
		result = await transferOneObject(
			publisher,
			subscriber,
			subscriberCount,
			options["subscribe-track"],
		);
	} finally {
		await runCleanups();
	}

	log("driver", `RESULT ${JSON.stringify(result)}`);
	if (!result.match)
		throw new Error("received payload differs from the sent payload");
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.publisher} to ${options.subscriber} through the ${options.relay} relay: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
