// Interop driver for the runs in which a client leaves: a publisher that is
// killed or closes right after its last Object, a subscriber that leaves while
// others stay, and a subscriber that takes nothing. Every run goes through
// the own relay and ends by moving one more Object over the same track with
// fresh clients, which the relay refuses if it still holds the old publisher.
// `matrix.ts` runs the scenarios.
//
// Usage: bun disconnect.ts --scenario <scenario> --publisher <client> [--port N] [--verbose]
//   publisher-killed        --subscriber <client>...
//     The publisher is killed after its Objects arrived; an own-browser
//     publisher has a subgroup stream open at that moment.
//   last-object-then-close  --subscriber <client>... [--with-publish-done]
//     The publisher sends one Object and closes the session at once.
//   subscriber-leaves       --leaver <client>:<cancel|close|kill>... --stayer <client>...
//     The publisher and the stayers go on after the leavers left.
//   slow-subscriber         --subscriber <client>... [--slow stopped-own-rust|unread-own-browser]
//     A subscriber that takes nothing is cut off; the others get every Object.
//     stopped-own-rust: an own-rust client whose process is stopped.
//     unread-own-browser: an own-browser page that subscribes and never reads.
//   <client>: own-browser | own-rust | moqtail-browser | moqtail-rust
//
// What each scenario has to show is listed in
// docs/experiments/2026-10-10-disconnects-through-own-relay.md.

import { randomBytes } from "node:crypto";
import { parseArgs } from "node:util";
import {
	type ClientRun,
	isLeaveWay,
	LAST_OBJECT_WITH_PUBLISH_DONE,
	type LeaveWay,
	type Publisher,
	type Subscriber,
	type SubscriberPlan,
	type SubscriptionOutcome,
	startPublisher,
	startSubscriber,
} from "./clients.ts";
import {
	type Client,
	captureGroup,
	DISCONNECT_TIMEOUT_MS,
	isOwnClient,
	isRustClient,
	log,
	parseClient,
	runCleanups,
	startRun,
	stopEverythingOnExit,
} from "./harness.ts";

/** PUBLISH_DONE Status Codes (§10.11). */
const TRACK_ENDED = 0x2;
const TOO_FAR_BEHIND = 0x5;

/**
 * Objects the slow-subscriber run sends at most before it gives up. The
 * relay cuts a subscriber off at 256 streams it cannot write out (provisional,
 * `SendLimits` in crates/moqt-io), on top of the streams it could still open.
 */
const SLOW_SUBSCRIBER_MAX_OBJECTS = 2000;
/**
 * The same for a page that does not read. The page's library takes 64
 * streams at once (`maxPeerUniStreams`) and Chromium holds some more; in 60
 * rounds the publisher had sent 577 or 641 Objects when the relay cut the
 * page off, alone or beside other runs. The number follows from counts and
 * not from time.
 *
 * WHY so close to what was measured: a library that takes a few hundred
 * streams more would still be cut off, only later, and has to fail here.
 */
const UNREAD_PAGE_MAX_OBJECTS = 833;
/** Objects it sends before it looks whether the relay has cut the subscriber off. */
const SLOW_SUBSCRIBER_BATCH = 64;

/** 2^62 - 1, the Stream Count of a publisher that cannot give the exact one (§10.11). */
const STREAM_COUNT_UNKNOWN = "4611686018427387903";

/**
 * How long the relay may take to notice a publisher, and a subscriber, that
 * was killed. A killed process says nothing, so the relay notices by its
 * idle timeout of 6 seconds (provisional). The timer starts anew when the
 * relay sends after a silence, which for a publisher is its next keep-alive,
 * up to 3 seconds later, and for a subscriber the Object it is sent right
 * after the kill. Measured in 40 rounds: 9.0 seconds for the publisher, 6.0
 * to 6.1 for the subscriber.
 *
 * WHY 1.5 seconds above what was measured, and not more: with the idle
 * timeout of 9 seconds the relay had before, the two took 12.0 (11.8 once)
 * and 9.0 seconds, and a relay that falls back to that has to fail here. The
 * 1.5 seconds are for the time this driver takes to see the outcome and for
 * the runs that go on beside. No run was made on a CI runner. Provisional.
 */
const PUBLISHER_KILL_NOTICED_WITHIN_MS = 10_500;
const SUBSCRIBER_KILL_NOTICED_WITHIN_MS = 7_500;

// Lines of the own relay (crates/relay/src/hub.rs). They are waited for where
// no client can tell that the relay has acted: a killed client says nothing,
// and a stopped one reads nothing. Where a scenario expects a SESSION_CLOSED
// or an OPEN_DATA line to be absent, the run that follows every scenario
// waits for a line of the same form, so that a change of the form is noticed.
// Two kinds of lines have no such check in every run:
// - CLOSE_SESSION: no scenario makes the relay print it, so a change of its
//   form would go unnoticed and the check for its absence would pass.
// - STOP_SENDING_RECEIVED: it is waited for only where moqtail-browser
//   cancels. In the other runs a change of its form would go unnoticed.
//
// Which session a client has is taken from the order in which the clients of
// a run connect, one after the other; the relay prints no address to tell
// them by.
const sessionOpened = (session: number): RegExp =>
	new RegExp(`^SESSION_OPENED session=${session}$`);
const sessionClosed = (session: number): RegExp =>
	new RegExp(`^SESSION_CLOSED session=${session}$`);
/** The relay ended a request of the session on its side, as it does when the peer cancels. */
const requestEnded = (session: number): RegExp =>
	new RegExp(`^(?:FINISH|RESET)_REQUEST session=${session} stream=\\d+`);
/**
 * The relay got STOP_SENDING on a request stream of the session; capture
 * group 1 is the code, or "none".
 *
 * The relay prints this only where it sees the STOP_SENDING before the reset
 * of the other direction, which an own client that cancels sends as well
 * (§3.3.2). For the own page it saw the reset first, and printed nothing, in
 * 7 of 60 rounds. moqtail-browser sends a FIN in that direction and no
 * reset, and the line was there in every round.
 */
const stopSendingReceived = (session: number): RegExp =>
	new RegExp(
		`^STOP_SENDING_RECEIVED session=${session} stream=\\d+ code=(\\S+)$`,
	);
/** The relay reset a stream towards a subscriber; capture group 1 is the code. */
const streamReset = /^RESET_DATA out=\d+ code=(\S+)$/;
/** The relay opened a subgroup stream of the Group towards the session. */
const streamOpened = (session: number, group: number): RegExp =>
	new RegExp(`^OPEN_DATA session=${session} out=\\d+ .* group=${group}$`);
/** The relay closed a session itself, which no scenario here gives it a reason for. */
const SESSION_CLOSED_BY_RELAY = /^CLOSE_SESSION /;
const cutOffSent = (session: number): RegExp =>
	new RegExp(
		`^SEND session=${session} stream=\\d+ PUBLISH_DONE status_code=0x5 `,
	);

const SCENARIOS = [
	"publisher-killed",
	"last-object-then-close",
	"subscriber-leaves",
	"slow-subscriber",
] as const;
type Scenario = (typeof SCENARIOS)[number];

function isScenario(value: string): value is Scenario {
	return SCENARIOS.some((scenario) => scenario === value);
}

const { values: options } = parseArgs({
	options: {
		scenario: { type: "string", default: "" },
		publisher: { type: "string", default: "own-rust" },
		subscriber: { type: "string", multiple: true, default: [] },
		leaver: { type: "string", multiple: true, default: [] },
		stayer: { type: "string", multiple: true, default: [] },
		"with-publish-done": { type: "boolean", default: false },
		slow: { type: "string", default: "stopped-own-rust" },
		port: { type: "string", default: "44340" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

function hexOf(payload: string): string {
	return Buffer.from(payload).toString("hex");
}

/** A run and the number the relay gave the session of each of its subscribers. */
interface Run extends ClientRun {
	/** The relay numbers its sessions from 0 in the order it accepts them. */
	sessions: number;
	sessionOf: Map<Subscriber, number>;
}

/**
 * The number the relay gave the session of the client that just connected.
 * The clients of a run connect one after the other, so it is the next one.
 */
async function sessionOfNewClient(run: Run): Promise<number> {
	const session = run.sessions;
	run.sessions += 1;
	await run.context.relay.waitForLine(sessionOpened(session));
	return session;
}

async function joinAsPublisher(kind: Client, run: Run): Promise<Publisher> {
	const publisher = await startPublisher(kind, run);
	await sessionOfNewClient(run);
	return publisher;
}

async function joinAsSubscriber(
	kind: Client,
	label: string,
	run: Run,
	plan: SubscriberPlan = {},
): Promise<Subscriber> {
	const subscriber = await startSubscriber(kind, label, run, plan);
	run.sessionOf.set(subscriber, await sessionOfNewClient(run));
	return subscriber;
}

function sessionOf(subscriber: Subscriber, run: Run): number {
	const session = run.sessionOf.get(subscriber);
	if (session === undefined) {
		throw new Error(`${subscriber.label} did not join through this run`);
	}
	return session;
}

/** Starts one subscriber per kind, one after the other, labelled by position. */
async function joinAsSubscribers(
	kinds: Client[],
	role: string,
	run: Run,
): Promise<Subscriber[]> {
	const started: Subscriber[] = [];
	for (const [index, kind] of kinds.entries()) {
		started.push(
			await joinAsSubscriber(kind, `${role}${index + 1}:${kind}`, run),
		);
	}
	return started;
}

const ownOnly = (subscribers: Subscriber[]): Subscriber[] =>
	subscribers.filter((subscriber) => isOwnClient(subscriber.kind));

/** Every subscriber has to receive the Object of the Group with the payload of the run. */
async function allReceive(
	subscribers: Subscriber[],
	group: number,
	run: Run,
): Promise<void> {
	await Promise.all(
		subscribers.map(async (subscriber) => {
			const received = await subscriber.object(group);
			if (received !== run.payloadHex) {
				throw new Error(
					`${subscriber.label} received ${received} in Group ${group}, not the sent payload`,
				);
			}
		}),
	);
}

/**
 * Whether the client took exactly these Groups. The order is not compared:
 * each Group has a stream of its own, and streams are not ordered against
 * each other.
 */
function tookGroups(outcome: SubscriptionOutcome, groups: number[]): boolean {
	const sorted = (values: number[]): string =>
		[...values].sort((a, b) => a - b).join(",");
	return sorted(outcome.groups) === sorted(groups);
}

/**
 * The Stream Reset Error Code the relay gives a stream that its publisher
 * left unfinished (§11.4.3: such a stream ends with a reset): CANCELLED, the
 * code of docs/moqt-implementation-notes.md for a subscription that ended.
 *
 * The relay gives CANCELLED where the publisher's session ended. Where the
 * publisher resets the stream itself, the relay hands on the publisher's
 * code instead. A browser that reset its streams before it closed the
 * session of a page that is closed would therefore fail these runs. In 20
 * rounds the relay saw the session end before any of the 111 streams of a
 * closed page (own-browser and moqtail-browser) was reset.
 */
const RESET_BY_RELAY = "0x1";

/**
 * What an own subscriber has to report of a subscription that PUBLISH_DONE
 * ended: the Status Code, the Groups it took, and, where the client can tell,
 * how its subgroup streams ended.
 */
function expectPublishDone(
	subscriber: Subscriber,
	outcome: SubscriptionOutcome,
	expected: {
		statusCode: number;
		/** Absent where the Groups taken depend on timing. */
		groups?: number[];
		/**
		 * How many streams have to end without a FIN. Absent where the number
		 * depends on timing.
		 */
		abortedStreams?: number;
	},
): void {
	const fail = (what: string): never => {
		throw new Error(`${subscriber.label}: ${what}: ${outcome.reported}`);
	};
	if (outcome.type !== "publish-done") {
		fail(`the subscription ended as "${outcome.type}", not by PUBLISH_DONE`);
	}
	if (outcome.statusCode !== expected.statusCode) {
		fail(
			`PUBLISH_DONE has Status Code ${outcome.statusCode}, not ${expected.statusCode}`,
		);
	}
	if (expected.groups !== undefined && !tookGroups(outcome, expected.groups)) {
		fail(`it took the Groups [${outcome.groups}], not [${expected.groups}]`);
	}
	// §10.11: the Stream Count is the number of streams opened for the
	// subscription. Every Group here has one stream of its own.
	if (
		expected.groups !== undefined &&
		outcome.streamCount !== String(expected.groups.length)
	) {
		fail(
			`PUBLISH_DONE has Stream Count ${outcome.streamCount}, not ${expected.groups.length}`,
		);
	}
	// §10.11: PUBLISH_DONE comes after every stream is closed, so with an
	// exact count the client has seen them all end and does not wait.
	if (expected.groups !== undefined && outcome.allStreamsEnded === false) {
		fail("it gave up waiting for streams that PUBLISH_DONE counted");
	}
	if (
		outcome.abortedStreams !== null &&
		expected.abortedStreams !== undefined &&
		(outcome.abortedStreams.length !== expected.abortedStreams ||
			outcome.abortedStreams.some((code) => code !== RESET_BY_RELAY))
	) {
		fail(
			`streams ended without a FIN with the codes [${outcome.abortedStreams}], not ${expected.abortedStreams} of ${RESET_BY_RELAY}`,
		);
	}
}

/**
 * The outcome of every own subscriber, with how long each took to learn of
 * it. Each gets the time it takes the relay to notice a publisher that said
 * nothing.
 */
async function outcomes(
	subscribers: Subscriber[],
): Promise<
	{ subscriber: Subscriber; outcome: SubscriptionOutcome; ms: number }[]
> {
	const started = performance.now();
	return Promise.all(
		ownOnly(subscribers).map(async (subscriber) => {
			const outcome = await subscriber.outcome(DISCONNECT_TIMEOUT_MS);
			return {
				subscriber,
				outcome,
				ms: Math.round(performance.now() - started),
			};
		}),
	);
}

/**
 * Moves one Object over the same track with a fresh publisher and subscriber.
 * The relay refuses a PUBLISH for a track that has a publisher, so this fails
 * if it still holds the one that left.
 *
 * It is also where the lines that the scenarios expect to be absent are seen
 * once: a stream opened towards a session, and a session dropped.
 */
async function relayServesTheTrackAgain(run: Run): Promise<void> {
	const relay = run.context.relay;
	const publisher = await joinAsPublisher("own-rust", run);
	const subscriber = await joinAsSubscriber(
		"own-rust",
		"sub-after:own-rust",
		run,
		{
			leavesAfterObjects: 1,
		},
	);
	const session = sessionOf(subscriber, run);
	const group = await publisher.send();
	await allReceive([subscriber], group, run);
	await relay.waitForLine(streamOpened(session, group));
	await subscriber.leave("close");
	await relay.waitForLine(sessionClosed(session));
	await publisher.end();
	// §3.3: a control stream is not closed while the session lasts. The relay
	// closes the session of a client that stops its control stream, which
	// own-rust did before it closed its session, on `wtransport`.
	const closedByRelay = relay.matches(SESSION_CLOSED_BY_RELAY);
	if (closedByRelay.length > 0) {
		throw new Error(
			`the relay closed a session itself: ${closedByRelay.map((match) => match.input).join("; ")}`,
		);
	}
}

/** An own-browser publisher has to see its subscription end by its own PUBLISH_DONE. */
async function expectOwnPublishDone(
	publisher: Publisher,
	reported: string,
): Promise<void> {
	if (publisher.kind === "own-browser" && reported !== '{"type":"done"}') {
		throw new Error(
			`${publisher.label}: its subscription ended as ${reported}, not by its own PUBLISH_DONE`,
		);
	}
	await expectRelayFinished(publisher);
}

/**
 * The relay finished its direction of the PUBLISH request stream after the
 * client's PUBLISH_DONE, which an XmoQ relay does once the streams the
 * Stream Count named have ended there (docs/moqt-implementation-notes.md,
 * "A publisher that ended its PUBLISH with PUBLISH_DONE"). Both own clients
 * wait for it before they close the session.
 */
async function expectRelayFinished(publisher: Publisher): Promise<void> {
	if (!isOwnClient(publisher.kind)) return;
	const how = await publisher.peerEndAfterPublishDone();
	if (how !== "FIN") {
		throw new Error(
			`${publisher.label} sent PUBLISH_DONE and saw the relay end its direction of the request stream as ${how ?? "nothing"}, not with a FIN`,
		);
	}
}

/** What a scenario reports: a line for the table of the matrix, and the details. */
interface ScenarioResult {
	summary: string;
	/** How each own subscription ended, by the label of its client. */
	subscriptions: Record<string, SubscriptionOutcome>;
	measured: Record<string, number>;
}

function subscriptionsOf(
	ended: { subscriber: Subscriber; outcome: SubscriptionOutcome }[],
): Record<string, SubscriptionOutcome> {
	return Object.fromEntries(
		ended.map(({ subscriber, outcome }) => [subscriber.label, outcome]),
	);
}

function seconds(ms: number): string {
	return `${(ms / 1000).toFixed(1)}s`;
}

async function publisherKilled(run: Run): Promise<ScenarioResult> {
	const kind = parseClient("publisher", options.publisher);
	const publisher = await joinAsPublisher(kind, run);
	const subscribers = await joinAsSubscribers(
		options.subscriber.map((value) => parseClient("subscriber", value)),
		"sub",
		run,
	);
	const groups = [await publisher.send()];
	await allReceive(subscribers, 0, run);
	// A stream that is open when the publisher goes: the relay has to end it
	// with a reset, and the Object on it still arrives.
	if (kind === "own-browser") {
		const open = await publisher.sendUnfinished();
		groups.push(open);
		await allReceive(ownOnly(subscribers), open, run);
	}

	await publisher.kill();
	const ended = await outcomes(subscribers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups,
			abortedStreams: publisher.streamsLeftOpen(),
		});
	}
	await relayServesTheTrackAgain(run);

	const slowest = Math.max(...ended.map(({ ms }) => ms));
	if (slowest > PUBLISHER_KILL_NOTICED_WITHIN_MS) {
		throw new Error(
			`the subscribers saw PUBLISH_DONE ${seconds(slowest)} after the publisher was killed, later than ${seconds(PUBLISHER_KILL_NOTICED_WITHIN_MS)}`,
		);
	}
	return {
		summary: `the subscribers saw PUBLISH_DONE ${seconds(slowest)} after the kill`,
		subscriptions: subscriptionsOf(ended),
		measured: Object.fromEntries(
			ended.map(({ subscriber, ms }) => [
				`${subscriber.label} ms from kill to end`,
				ms,
			]),
		),
	};
}

async function lastObjectThenClose(run: Run): Promise<ScenarioResult> {
	const kind = parseClient("publisher", options.publisher);
	const publishDone = options["with-publish-done"];
	if (!LAST_OBJECT_WITH_PUBLISH_DONE[kind].includes(publishDone)) {
		throw new Error(
			`${kind} cannot close after its last Object ${publishDone ? "with" : "without"} PUBLISH_DONE`,
		);
	}
	const publisher = await joinAsPublisher(kind, run);
	const subscribers = await joinAsSubscribers(
		options.subscriber.map((value) => parseClient("subscriber", value)),
		"sub",
		run,
	);

	const group = await publisher.sendLastAndClose(publishDone);
	if (publishDone) await expectRelayFinished(publisher);
	await allReceive(subscribers, group, run);
	const ended = await outcomes(subscribers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups: [group],
			abortedStreams: 0,
		});
	}
	await relayServesTheTrackAgain(run);

	return {
		summary: "every subscriber received the last Object",
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

function parseLeaver(value: string): { kind: Client; way: LeaveWay } {
	const [kind = "", way = ""] = value.split(":");
	if (!isLeaveWay(way)) {
		throw new Error(`--leaver "${value}" is not <client>:<cancel|close|kill>`);
	}
	return { kind: parseClient("leaver", kind), way };
}

async function subscriberLeaves(run: Run): Promise<ScenarioResult> {
	const publisher = await joinAsPublisher(
		parseClient("publisher", options.publisher),
		run,
	);
	const leavers: { subscriber: Subscriber; way: LeaveWay }[] = [];
	for (const [index, value] of options.leaver.entries()) {
		const { kind, way } = parseLeaver(value);
		const subscriber = await joinAsSubscriber(
			kind,
			`leaver${index + 1}:${kind}`,
			run,
			// The own Rust client leaves in order by having all it asked for.
			kind === "own-rust" && way === "close" ? { leavesAfterObjects: 1 } : {},
		);
		leavers.push({ subscriber, way });
	}
	const stayers = await joinAsSubscribers(
		options.stayer.map((value) => parseClient("stayer", value)),
		"stayer",
		run,
	);
	if (leavers.length === 0 || stayers.length === 0) {
		throw new Error("subscriber-leaves needs a --leaver and a --stayer");
	}
	// moqtail-rust closes its session by itself once it has an Object, so
	// after one it is not there to be killed.
	const leftAlready = leavers.find(
		({ subscriber, way }) =>
			publisher.sendsSeveral &&
			subscriber.kind === "moqtail-rust" &&
			way === "kill",
	);
	if (leftAlready !== undefined) {
		throw new Error(
			`${leftAlready.subscriber.label} cannot be killed after the first Object of ${publisher.kind}: it has left by then`,
		);
	}

	const everyone = [...leavers.map(({ subscriber }) => subscriber), ...stayers];
	const groups: number[] = [];
	// With a publisher that sends one Object only, the leavers go before it.
	if (publisher.sendsSeveral) {
		groups.push(await publisher.send());
		await allReceive(everyone, 0, run);
	}
	const beforeLeaving = [...groups];

	const relay = run.context.relay;
	const left = performance.now();
	await Promise.all(
		leavers.map(({ subscriber, way }) => subscriber.leave(way)),
	);
	// A cancel is over when the relay has ended the request on its side. What
	// is sent from here on must not go to the one that cancelled.
	const cancelled = leavers
		.filter(({ way }) => way === "cancel")
		.map(({ subscriber }) => subscriber);
	for (const subscriber of cancelled) {
		await relay.waitForLine(requestEnded(sessionOf(subscriber, run)));
	}
	// At once: the relay does not know yet that a killed process is gone, and
	// must not wait for it.
	const afterLeaving = await publisher.send();
	groups.push(afterLeaving);
	await allReceive(stayers, afterLeaving, run);
	for (const { subscriber, way } of leavers) {
		if (
			way === "kill" &&
			isRustClient(subscriber.kind) &&
			relay.matches(sessionClosed(sessionOf(subscriber, run))).length > 0
		) {
			throw new Error(
				`the relay had dropped the session of ${subscriber.label} before the stayers received Group ${afterLeaving}, so the run does not show that it did not wait for it`,
			);
		}
	}

	const measured: Record<string, number> = {};
	if (publisher.sendsSeveral) {
		// Once more when the relay has dropped the sessions that ended.
		await Promise.all(
			leavers
				.filter(({ way }) => way !== "cancel")
				.map(async ({ subscriber, way }) => {
					await relay.waitForLine(
						sessionClosed(sessionOf(subscriber, run)),
						DISCONNECT_TIMEOUT_MS,
					);
					const ms = Math.round(performance.now() - left);
					measured[
						`${subscriber.label} ms from ${way} to the relay dropping the session`
					] = ms;
					if (way === "kill" && ms > SUBSCRIBER_KILL_NOTICED_WITHIN_MS) {
						throw new Error(
							`the relay dropped the session of ${subscriber.label} ${seconds(ms)} after it was killed, later than ${seconds(SUBSCRIBER_KILL_NOTICED_WITHIN_MS)}`,
						);
					}
				}),
		);
		const afterDropped = await publisher.send();
		groups.push(afterDropped);
		await allReceive(stayers, afterDropped, run);
	}

	await expectOwnPublishDone(publisher, await publisher.end());
	const ended = await outcomes(stayers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups,
			abortedStreams: publisher.streamsLeftOpen(),
		});
	}

	// The own page still exists after cancel and close, and tells how its
	// subscription ended and what it took.
	const leaverEnds: { subscriber: Subscriber; outcome: SubscriptionOutcome }[] =
		[];
	for (const { subscriber, way } of leavers) {
		if (subscriber.kind !== "own-browser" || way === "kill") continue;
		const outcome = await subscriber.outcome();
		const expected = way === "cancel" ? "cancelled" : "session-closed";
		if (outcome.type !== expected) {
			throw new Error(
				`${subscriber.label}: the subscription ended as "${outcome.type}", not "${expected}": ${outcome.reported}`,
			);
		}
		// Nothing sent after it left may have reached it.
		if (!tookGroups(outcome, beforeLeaving)) {
			throw new Error(
				`${subscriber.label}: it took the Groups [${outcome.groups}], not [${beforeLeaving}]: ${outcome.reported}`,
			);
		}
		leaverEnds.push({ subscriber, outcome });
	}
	await relayServesTheTrackAgain(run);

	// §11.4.1: ending the request stream ends the subscription, not the
	// session. So the relay opened no stream towards the one that cancelled
	// for what was sent after, and still serves its session.
	// WHY only now: every line the relay printed during the scenario has been
	// read by the time the run that follows it is over.
	const sentAfterLeaving = groups.filter(
		(group) => !beforeLeaving.includes(group),
	);
	for (const subscriber of cancelled) {
		const session = sessionOf(subscriber, run);
		const opened = sentAfterLeaving.filter(
			(group) => relay.matches(streamOpened(session, group)).length > 0,
		);
		if (opened.length > 0) {
			throw new Error(
				`the relay went on sending to ${subscriber.label} after it cancelled: Groups [${opened}]`,
			);
		}
		if (relay.matches(sessionClosed(session)).length > 0) {
			throw new Error(
				`the relay dropped the session of ${subscriber.label}, which cancelled its subscription only`,
			);
		}
		// §3.3.3: a relevant code. The own page cancels with a
		// `WebTransportError` that carries CANCELLED (0x1), and the browser has
		// to send that code and no other; the relay does not always print what
		// it got (see `stopSendingReceived`). moqtail-browser cancels its reader
		// without a reason, which arrives as 0x0, and its line is always there:
		// that is the one place where a change of the line's form is noticed.
		const codes = relay
			.matches(stopSendingReceived(session))
			.map((match) => captureGroup(match, 1));
		const expectedCode = subscriber.kind === "own-browser" ? "0x1" : "0x0";
		if (
			codes.some((code) => code !== expectedCode) ||
			(subscriber.kind === "moqtail-browser" && codes.length === 0)
		) {
			throw new Error(
				`${subscriber.label} cancelled, and the relay printed STOP_SENDING for its request stream with the codes [${codes}], not ${expectedCode}`,
			);
		}
		measured[`${subscriber.label} STOP_SENDING seen by the relay`] =
			codes.length;
	}

	return {
		summary: `the stayers received ${groups.length - beforeLeaving.length} Objects sent after the leavers left`,
		subscriptions: subscriptionsOf([...ended, ...leaverEnds]),
		measured,
	};
}

async function slowSubscriber(run: Run): Promise<ScenarioResult> {
	const publisher = await joinAsPublisher(
		parseClient("publisher", options.publisher),
		run,
	);
	if (!publisher.sendsSeveral) {
		throw new Error(
			`${publisher.kind} sends one Object, too few to fall behind`,
		);
	}
	// Two ways not to take what is sent. A stopped process answers nothing
	// at all; a page that does not read leaves the streams with its browser,
	// and its session goes on answering.
	if (
		options.slow !== "stopped-own-rust" &&
		options.slow !== "unread-own-browser"
	) {
		throw new Error(
			`--slow "${options.slow}" is neither stopped-own-rust nor unread-own-browser`,
		);
	}
	const unread = options.slow === "unread-own-browser";
	const slow = unread
		? await joinAsSubscriber("own-browser", "slow:own-browser", run, {
				withoutReading: true,
			})
		: await joinAsSubscriber("own-rust", "slow:own-rust", run);
	const readers = await joinAsSubscribers(
		options.subscriber.map((value) => parseClient("subscriber", value)),
		"sub",
		run,
	);
	const relay = run.context.relay;

	const groups = [await publisher.send()];
	if (unread) {
		await allReceive(readers, 0, run);
	} else {
		// It is a working subscriber until it is stopped.
		await allReceive([slow, ...readers], 0, run);
		await slow.setStopped(true);
	}
	const stopped = performance.now();

	// Sends until the relay has cut the subscriber off.
	let cut: "cut" | Error | undefined;
	const watching = relay
		.waitForLine(cutOffSent(sessionOf(slow, run)), DISCONNECT_TIMEOUT_MS)
		.then(
			() => {
				cut = "cut";
			},
			(error: unknown) => {
				cut = error instanceof Error ? error : new Error(String(error));
			},
		);
	while (cut === undefined) {
		if (
			groups.length >=
			(unread ? UNREAD_PAGE_MAX_OBJECTS : SLOW_SUBSCRIBER_MAX_OBJECTS)
		) {
			throw new Error(
				`${slow.label} was not cut off after ${groups.length} Objects it did not take`,
			);
		}
		groups.push(...(await publisher.sendBatch(SLOW_SUBSCRIBER_BATCH)));
		// Lets the relay's line in, which arrives between two batches.
		await Promise.race([
			watching,
			new Promise((resolve) => setImmediate(resolve)),
		]);
	}
	if (cut !== "cut") throw cut;
	const sentUntilCutOff = groups.length;
	const msUntilCutOff = Math.round(performance.now() - stopped);

	// The others are not held back, before the cut-off or after it.
	const last = await publisher.send();
	groups.push(last);
	await allReceive(readers, last, run);

	// §10.11: no stream is opened for a subscription after its PUBLISH_DONE.
	// `last` was sent after the cut-off, and the readers have it.
	const slowSession = sessionOf(slow, run);
	if (relay.matches(streamOpened(slowSession, last)).length > 0) {
		throw new Error(
			`the relay opened a stream of Group ${last} towards ${slow.label} after it had ended its subscription`,
		);
	}
	// §3.3.3: the streams the relay cut short say why. Read from the relay's
	// log: they are the streams it could not open, so the subscriber never
	// saw them reset.
	const cutShort = relay
		.matches(streamReset)
		.map((match) => captureGroup(match, 1));
	if (cutShort.length === 0 || cutShort.some((code) => code !== "0x5")) {
		throw new Error(
			`the relay reset ${cutShort.length} streams when it cut ${slow.label} off, with the codes [${[...new Set(cutShort)]}], not TOO_FAR_BEHIND alone`,
		);
	}
	if (!unread) await slow.setStopped(false);
	// The page ends the subscription only when the library's wait for late
	// streams is over, as the Stream Count says nothing:
	// DEFAULT_PUBLISH_DONE_TIMEOUT_MS in
	// packages/moqt/src/webtransport/connection.ts, 5 seconds.
	const cutOff = await slow.outcome();
	// Only the subscription was ended: the relay still serves the session,
	// also after that wait.
	if (relay.matches(sessionClosed(slowSession)).length > 0) {
		throw new Error(
			`the relay dropped the session of ${slow.label}, where it had to end its subscription only`,
		);
	}
	expectPublishDone(slow, cutOff, { statusCode: TOO_FAR_BEHIND });
	// §10.11: a publisher that cannot give the exact number of streams gives
	// 2^62 - 1. The relay could not open every stream it had for this
	// subscriber, which is the case its notes name for that value.
	if (cutOff.streamCount !== STREAM_COUNT_UNKNOWN) {
		throw new Error(
			`${slow.label}: PUBLISH_DONE has Stream Count ${cutOff.streamCount}, not 2^62 - 1: ${cutOff.reported}`,
		);
	}
	// Not compared: the streams the subscriber sees reset. In every run so
	// far it saw none, because the streams the relay reset were the ones it
	// could not open while the subscriber gave it no more stream credit.

	await expectOwnPublishDone(publisher, await publisher.end());
	const ended = await outcomes(readers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups,
			abortedStreams: 0,
		});
	}
	await relayServesTheTrackAgain(run);

	return {
		summary: `the ${unread ? "page that did not read" : "stopped subscriber"} was cut off with TOO_FAR_BEHIND after ${sentUntilCutOff} Objects; the others received all ${groups.length}`,
		subscriptions: subscriptionsOf([
			...ended,
			{ subscriber: slow, outcome: cutOff },
		]),
		measured: {
			"Objects sent until the relay cut the subscriber off": sentUntilCutOff,
			"ms from the first Object it did not take to the cut-off": msUntilCutOff,
		},
	};
}

const RUNNERS: Record<Scenario, (run: Run) => Promise<ScenarioResult>> = {
	"publisher-killed": publisherKilled,
	"last-object-then-close": lastObjectThenClose,
	"subscriber-leaves": subscriberLeaves,
	"slow-subscriber": slowSubscriber,
};

async function main(): Promise<void> {
	const scenario = options.scenario;
	if (!isScenario(scenario)) {
		throw new Error(
			`--scenario "${scenario}" is not one of ${SCENARIOS.join(", ")}`,
		);
	}
	const subscribers = [
		...options.subscriber,
		...options.stayer,
		...options.leaver.map((value) => parseLeaver(value).kind),
	].map((value) => parseClient("subscriber", value));
	const publisher = parseClient("publisher", options.publisher);

	let result: ScenarioResult;
	let relayVersion: string;
	let browserVersion: string | null;
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: [
				publisher,
				...subscribers,
				...(options.slow === "unread-own-browser"
					? ["own-browser" as const]
					: []),
				// The run that follows every scenario uses the own Rust client.
				"own-rust",
			],
			verbose: options.verbose,
		});
		relayVersion = context.relayVersion;
		browserVersion = context.pages?.browser.version() ?? null;
		result = await RUNNERS[scenario]({
			context,
			payloadHex: hexOf(
				`xmoq-disconnect ${new Date().toISOString()} ${randomBytes(8).toString("hex")}`,
			),
			sessions: 0,
			sessionOf: new Map(),
		});
	} finally {
		await runCleanups();
	}

	log(
		"driver",
		`RESULT ${JSON.stringify({ scenario, publisher, ...result, relayVersion, browserVersion })}`,
	);
	log("driver", `SUMMARY ${result.summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
