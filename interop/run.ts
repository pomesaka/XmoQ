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
//   --update-subgroup-timeout-ms: the subscriber, own-rust, sends a
//     REQUEST_UPDATE with this SUBGROUP_DELIVERY_TIMEOUT once it is subscribed;
//     the run goes on when the relay has answered REQUEST_OK.
//   --await-publish-done: after the transfer, makes the publisher leave and
//     waits until the subscriber, which has to be an own client, has seen
//     PUBLISH_DONE.
//
// The moqtail relay and clients live under peers/moqtail, the own relay in
// crates/relay and the own Rust client in crates/moqt-client; build them with
// the steps in README.md.

import { randomBytes } from "node:crypto";
import { parseArgs } from "node:util";
import { type Subscriber, startPublisher, startSubscriber } from "./clients.ts";
import {
	type Client,
	DISCONNECT_TIMEOUT_MS,
	isOwnClient,
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	parseClient,
	runCleanups,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { PageProperty } from "./xmoq-page.ts";

const { values: options } = parseArgs({
	options: {
		publisher: { type: "string", default: "own-browser" },
		subscriber: { type: "string", default: "moqtail-rust" },
		relay: { type: "string", default: "moqtail" },
		subscribers: { type: "string", default: "1" },
		port: { type: "string", default: "24330" },
		"await-publish-done": { type: "boolean", default: false },
		// The Object carries Object Properties, and every subscriber has to
		// get them as they were sent. Between own clients only.
		"object-properties": { type: "boolean", default: false },
		// The publisher announces the namespace and sends no PUBLISH: it
		// sends on the subscription the relay asks it for. own-browser only.
		"announce-only": { type: "boolean", default: false },
		"subscribe-track": { type: "string", default: TRACK },
		// The subscriber, own-rust, asks for this SUBGROUP_DELIVERY_TIMEOUT
		// with a REQUEST_UPDATE once it is subscribed, and the relay has to
		// accept it before the Object is sent.
		"update-subgroup-timeout-ms": { type: "string", default: "" },
		// The subscribers connect to a second relay, the own one, whose
		// upstream is the relay of the run (`--upstream`), on the run's port
		// plus MOQLIVEMOCK_SIDE_PORT_OFFSET; the publisher stays with the
		// relay of the run.
		"through-edge": { type: "boolean", default: false },
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

/**
 * What the Object of `--object-properties` carries: one property of each
 * form of a Key-Value-Pair (draft 18 §1.4.3), in ascending order of type,
 * which is the order on the wire. Neither type is a property draft 18
 * defines, so a relay has to pass them unchanged (§2.5: "MUST NOT be
 * modified, MUST be forwarded"). The values are made up. **Nothing here
 * reads them as LOC**: the two numbers are ones a revision of LOC has used
 * (Video Config and Timestamp in draft-ietf-moq-loc-04), and stay as they
 * are whichever revision `@xmoq/loc` follows.
 */
const OBJECT_PROPERTIES: PageProperty[] = [
	// Odd type: a length and bytes.
	{ type: "13", bytesHex: "0142e01fffe100096742e01f8c8d40500000" },
	// Even type: one variable-length integer, here one that takes 8 bytes.
	{ type: "16", varint: "1760140800123456" },
];

/**
 * Throws unless the subscriber has the Object's properties as they were
 * sent. The messages tell apart what a relay can have done to them.
 */
async function expectProperties(
	subscriber: Subscriber,
	group: number,
	sent: PageProperty[],
): Promise<void> {
	const received = await subscriber.properties(group);
	if (JSON.stringify(received) === JSON.stringify(sent)) return;
	const what =
		received.length === 0
			? "arrived without Object Properties: they were dropped on the way"
			: "arrived with other Object Properties than were sent";
	throw new Error(
		`${subscriber.label}: the Object of Group ${group} ${what}; sent ${JSON.stringify(sent)}, received ${JSON.stringify(received)}`,
	);
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
	const withProperties = options["object-properties"];
	if (withProperties && !(isOwnClient(publisher) && isOwnClient(subscriber))) {
		throw new Error(
			`--object-properties needs own clients on both sides, the ones that send properties with content and report the ones they get, not ${publisher} and ${subscriber}`,
		);
	}
	const awaitsPublishDone = options["await-publish-done"];
	if (awaitsPublishDone && !isOwnClient(subscriber)) {
		throw new Error(
			`--await-publish-done needs an own subscriber, not ${subscriber}`,
		);
	}

	const announceOnly = options["announce-only"];
	if (announceOnly && publisher !== "own-browser") {
		throw new Error(
			`--announce-only needs own-browser as the publisher, the page that serves a track it is asked for, not ${publisher}`,
		);
	}
	// The publisher offers the track first: the relay rejects a SUBSCRIBE for
	// a track nobody has published, or announced the namespace of.
	const publishing = await startPublisher(publisher, run, {
		...(withProperties ? { properties: OBJECT_PROPERTIES } : {}),
		...(announceOnly ? { servesWhenAsked: "at-once" } : {}),
	});
	const edge = options["through-edge"]
		? await startSecondOwnRelay(
				String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
				context,
			)
		: undefined;
	const subscribersRun =
		edge === undefined
			? run
			: { context: { ...context, relayUrl: edge.url }, payloadHex };
	// The subscribers are live before the Object is sent: the Largest Object
	// filter does not replay what was published earlier.
	const subscribing = [];
	for (let index = 0; index < subscriberCount; index++) {
		const label =
			subscriberCount === 1
				? `sub:${subscriber}`
				: `sub${index + 1}:${subscriber}`;
		subscribing.push(
			await startSubscriber(subscriber, label, subscribersRun, {
				track: subscribeTrack,
				// A subscriber that is not asked how its subscription ended leaves
				// once it has the Object.
				...(awaitsPublishDone ? {} : { leavesAfterObjects: 1 }),
				...(options["update-subgroup-timeout-ms"] === ""
					? {}
					: {
							updatesSubgroupTimeoutMs: Number(
								options["update-subgroup-timeout-ms"],
							),
						}),
			}),
		);
	}
	// A publisher that only announced has a subscription to send on once the
	// relay has asked it, which a subscriber makes it do.
	await withTimeout(
		publishing.serving(),
		"the publisher to accept the SUBSCRIBE the relay sends it",
	);
	const group = await withTimeout(publishing.send(), "the publisher to send");
	const received = await Promise.all(
		subscribing.map(async (each) => {
			try {
				return await each.object(group);
			} catch (error) {
				if (!withProperties) throw error;
				// The Object did not come. Whether the session is still open
				// tells a stream that was dropped from a session that was closed.
				const state = await each.state().catch((failed) => String(failed));
				throw new Error(
					`${each.label} did not get the Object with Object Properties (${error instanceof Error ? error.message : String(error)}); its state: ${state}`,
				);
			}
		}),
	);
	if (withProperties) {
		for (const each of subscribing) {
			await expectProperties(each, group, OBJECT_PROPERTIES);
		}
	}
	const receivedHex = received.find((hex) => hex !== payloadHex) ?? payloadHex;
	let subscriptionEnd: string[] | null = null;
	if (awaitsPublishDone) {
		await publishing.end();
		subscriptionEnd = await Promise.all(
			subscribing.map(async (each) => {
				// WHY the longer wait: moqtail-rust exits right after it closes
				// its session, and where the close was not sent the relay ended
				// the track by its idle timeout only (seen once for this pairing).
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
