// The two publishers that answer a FETCH, as the drivers of the FETCH runs
// start them (fetch.ts for a page that fetches, fetch-rust.ts for the own
// Rust client).

import { type Publisher, startPublisher, startSubscriber } from "./clients.ts";
import {
	type LoggedProcess,
	type RunContext,
	spawnMoqtailTestPublisher,
	withTimeout,
} from "./harness.ts";
import type { PageFetchServing } from "./xmoq-page.ts";

/**
 * How many Groups moqtail's test client publishes, three Objects each, four
 * a second: 30 seconds, of which a run takes a few.
 */
const TEST_PUBLISHER_GROUPS = 40;

/** How many Groups the publishing page has sent, Object 0 of each, when `publishFromAPage` returns. */
export const PAGE_GROUPS_BEFORE = 3;

/**
 * The first Group a page with "serve-unsent" sends: the one after the three
 * it has and never sent, {100, 0} to {102, 0}.
 *
 * WHY not Group 0, as every other page: the page says in its SUBSCRIBE_OK
 * that its Largest Object is {102, 0}, and a subscriber with the Largest
 * Object filter, which both subscribers of the run have, is sent nothing
 * that is not after that (draft 18 §5.1.2). Sending Groups below what it
 * said was its largest is also nothing a publisher does. So the three are
 * Objects the page published before the relay subscribed.
 */
const FIRST_GROUP_AFTER_UNSENT = 103;

/**
 * Starts moqtail's test client, which publishes by itself, and waits until
 * it is past Group 2: the ranges of the runs are behind it then.
 */
export async function startTestPublisher(
	context: RunContext,
): Promise<LoggedProcess> {
	const publisher = spawnMoqtailTestPublisher(
		context,
		"pub:moqtail-test",
		TEST_PUBLISHER_GROUPS,
	);
	await publisher.waitForLine(/Track published/);
	await publisher.waitForLine(/Stream flushed for group 2$/);
	return publisher;
}

/**
 * Starts a page that announces the namespace and serves the track and its
 * FETCHes, has the relay ask it for the track by a subscriber of its own,
 * and has it send Object 0 of the first Groups to that subscriber. The page
 * is the first client of the run, and that subscriber the second.
 */
export async function publishFromAPage(
	servesFetches: PageFetchServing,
	context: RunContext,
	payloadHex: string,
): Promise<Publisher> {
	const run = { context, payloadHex };
	const publisher = await startPublisher("own-browser", run, {
		servesWhenAsked: "at-once",
		servesFetches,
		...(servesFetches === "serve-unsent"
			? { firstGroup: FIRST_GROUP_AFTER_UNSENT }
			: {}),
	});
	const feeder = await startSubscriber("own-rust", "feeder:own-rust", run, {});
	await withTimeout(
		publisher.serving(),
		"the publishing page to accept the SUBSCRIBE the relay sends it",
	);
	for (const group of await publisher.sendBatch(PAGE_GROUPS_BEFORE)) {
		await feeder.object(group);
	}
	return publisher;
}
