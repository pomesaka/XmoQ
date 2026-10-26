// Interop driver for the runs in which a client leaves: a publisher that is
// killed or closes right after its last Object, a subscriber that leaves while
// others stay, a subscriber that takes nothing, and two publishers of one
// track. The last kind is here although nobody has to leave in it: it needs
// the same parts, and what it adds is what happens when one of the two goes.
// Every run goes through
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
//   same-group-twice        --publisher <client> --publisher <client> --subscriber <client>...
//                           [--first-sends N]
//     Two publishers send the same Group, one after the other; the
//     subscribers get it once. With --first-sends N the first sends N Groups
//     before the second sends Group 0, a source that is far behind.
//   second-source           --publisher <client> --publisher <client> --subscriber <client>...
//                           [--first done|kill|kill-with-open-stream|stays] [--second-joins before|after]
//                           [--first-sends N] [--second-resends]
//     The first publisher sends and goes, or stays; the second sends the next
//     Group, which the subscribers get. With --second-resends the second
//     first sends again every Group the first has sent.
//   ask-all                 --ask <way> [--publisher <client>] --subscriber <own client> --subscriber <own client>...
//     Which publishers the relay sends its own SUBSCRIBE. two-announcers: two
//     publishers announce the namespace and send no PUBLISH, own-rust ones
//     unless --publisher names the two (own clients).
//     announcer-and-publisher: --publisher sends PUBLISH, and an own-rust
//     publisher announces before the subscribers come. late-announcer: it
//     announces after they came. refusing-announcer: an own-browser page
//     announces and refuses what it is asked; it is not asked again until it
//     announces anew.
//   subgroup-priorities     --priorities <first>,<second> --publisher <own client> [--publisher <own client>] --subscriber <own client>...
//     A Subgroup comes twice, with these Publisher Priorities (0 to 255, or
//     default for a header without one). With two publishers the second
//     sends the Group the first has sent; with one, own-browser, it sends
//     the Group again itself. Where the priorities differ the track ends as
//     malformed; where they are the same the second stream is only stopped.
//   end-of-track            --publisher own-rust --subscriber <own client>...
//     The publisher sends a Group, then an Object with the status
//     END_OF_TRACK, then another Group: past the end of the track (§2.4.2,
//     the fifth example). The relay ends the track as malformed: PUBLISH_DONE
//     MALFORMED_TRACK to the subscribers, the PUBLISH cancelled with 0x12.
//   subgroup-again          --again longer|shorter --publisher own-rust --subscriber <own client>...
//     The publisher sends a Group, then the next Group on a stream that ends
//     with a FIN, then the same Subgroup again on a second stream that ends
//     with a FIN, with one Object more (Objects 0, then 0 and 1) or one fewer
//     (0 and 1, then 0): an Object past the final Object of the Subgroup, or
//     two FINs after different final Objects (§2.4.2, the second and the
//     third example). The relay reads the second stream for the check alone
//     and ends the track as malformed: PUBLISH_DONE MALFORMED_TRACK to the
//     subscribers, the PUBLISH cancelled with 0x12. Written before the first
//     run (2026-10-25).
//   publish-anew            --publisher own-rust --subscriber <client>... [--same-alias]
//     The publisher sends a Group, says PUBLISH_DONE and publishes the track
//     anew on the same session, under the next Track Alias or the same, and
//     sends another Group. The subscribers stay and get both.
//   publish-while-asked     --publisher own-rust --publisher own-rust --subscriber <client>...
//     The first publisher announces the namespace and holds back its answer
//     to the relay's SUBSCRIBE; the second, which is connected already, sends
//     PUBLISH for the track then, which answers the subscribers. Both then
//     send a Group. The driver tells each publisher when to go on.
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
	type PublisherPlan,
	type PublisherPriority,
	type Subscriber,
	type SubscriberPlan,
	type SubscriptionOutcome,
	startPublisher,
	startRefusingAnnouncer,
	startSubscriber,
} from "./clients.ts";
import {
	type Client,
	captureGroup,
	DISCONNECT_TIMEOUT_MS,
	type Fence,
	isBrowserClient,
	isOwnClient,
	isRustClient,
	log,
	parseClient,
	runCleanups,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";

/** PUBLISH_DONE Status Codes (§10.11). */
const TRACK_ENDED = 0x2;
const TOO_FAR_BEHIND = 0x5;
/** Also the Stream Reset Error Code and the STOP_SENDING code of the same name. */
const MALFORMED_TRACK = 0x12;

/**
 * The streams a subscription may have that are not written out, for the run
 * with a stopped subscriber (`--subscription-send-streams` of the relay).
 *
 * WHY not the relay's 1,024: a stopped process answers nothing, and its
 * connection ends at the relay's idle timeout, 6 seconds unless told
 * (`STOPPED_IDLE_TIMEOUT_MS`); with 1,024 the
 * publishing page of CI had opened 834 streams to it in 6 seconds (one Object a
 * stream, about 140 a second), and the subscriber was never cut off for
 * being behind. Locally it was cut off after 1,089 Objects, 4.3 seconds
 * after the stop. WHY 256: the number the relay had before for a session,
 * which the run was measured with, and reached in about 2.3 seconds at the
 * rate of CI. The page that does not read keeps answering, so it is run
 * with the relay's own number.
 */
const STOPPED_SUBSCRIPTION_STREAMS = 256;
/**
 * The relay's idle timeout for the run with a stopped subscriber
 * (`--idle-timeout-ms`).
 *
 * WHY longer than the relay's 6 seconds: the run has to cut the subscriber
 * off, send the next Group to the others and look at the relay's log before
 * it lets the subscriber go on, all while it is stopped (about 1.4 s
 * here); on a machine busy enough for that to pass 6 seconds, the relay
 * would close the session for idleness, and the run would fail as though
 * the relay had dropped it. The stopped client does not end its own session
 * for idleness: stopped for 15 seconds, it read the PUBLISH_DONE when it went
 * on and closed with NO_ERROR
 * (docs/experiments/2026-10-26-what-the-interop-runs-wait-for-and-leave-behind.md).
 */
const STOPPED_IDLE_TIMEOUT_MS = 30_000;
/**
 * Objects the run with a stopped subscriber sends at most before it gives
 * up: the streams above and the 65 the own Rust client lets the relay open,
 * which in 6 rounds had it cut off after 321 Objects each time, 0.52 to
 * 0.67 seconds after the first it did not take; with the 192 of margin the
 * page's number below has. At the rate of CI the 321 take about 2.3
 * seconds, and 513 about 3.7, both within the idle timeout.
 *
 * WHY so close to what was measured: as for the page, a relay that held
 * more than it says would still cut the subscriber off, only later, and has
 * to fail here.
 */
const SLOW_SUBSCRIBER_MAX_OBJECTS = 513;
/**
 * The same for a page that does not read. The page's library takes 64
 * streams at once (`maxPeerUniStreams`) and Chromium holds some more; in 6
 * rounds the publisher had sent 1,473 or 1,537 Objects when the relay cut the
 * page off, with 1,024 streams a subscription (577 or 641 in 60 rounds with
 * 256 streams a session, before). The number follows from counts and not
 * from time.
 *
 * WHY so close to what was measured: a library that takes a few hundred
 * streams more would still be cut off, only later, and has to fail here.
 */
const UNREAD_PAGE_MAX_OBJECTS = 1729;
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
// The runs with two publishers wait for two more: STOP_DATA, for a stream of
// a publisher that the relay stopped, and SEND … PUBLISH_DONE towards a
// subscriber, which has to be absent while a source is left and is seen once
// for every own subscriber when the track has ended.
//
// The relay's lines are counted, and found absent, only before a line of the
// relay itself, the fence (`RelayLog.linesBefore` in harness.ts, which has
// why). The type of the relay has no way to count the lines "so far". A
// scenario names the fence by what makes the relay print it: a session that
// ended (`leaveInOrder`), a stream it stopped (`expectStreamsStopped`), a
// client that came, a Group it sent on, or, for what holds of the whole
// scenario, the first session of the run that follows it
// (`relayServesTheTrackAgain`).
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
/** The relay stopped a subgroup stream of the session (STOP_SENDING); capture group 1 is the code. */
const dataStreamStopped = (session: number): RegExp =>
	new RegExp(`^STOP_DATA session=${session} stream=\\d+ code=(\\S+)$`);
/** The relay sent PUBLISH_DONE to the session. */
const publishDoneSent = (session: number): RegExp =>
	new RegExp(`^SEND session=${session} stream=\\d+ PUBLISH_DONE `);
/** The relay sent SUBSCRIBE_OK to the session. */
const subscribeOkSent = (session: number): RegExp =>
	new RegExp(`^SEND session=${session} stream=\\d+ SUBSCRIBE_OK`);
/** The relay opened a request of its own towards the session: the SUBSCRIBE it sends a publisher that announced the namespace. */
const requestOpenedTowards = (session: number): RegExp =>
	new RegExp(`^OPEN_REQUEST session=${session} `);
/** The relay ended a request of its own towards the session. */
const ownRequestEnded = (session: number): RegExp =>
	new RegExp(`^(?:FINISH|CANCEL)_OWN_REQUEST session=${session} `);
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
	"same-group-twice",
	"second-source",
	"publish-while-asked",
	"publish-anew",
	"ask-all",
	"subgroup-priorities",
	"end-of-track",
	"subgroup-again",
] as const;
type Scenario = (typeof SCENARIOS)[number];
type OptionOfTwoSources =
	| "first"
	| "first-sends"
	| "second-resends"
	| "second-joins";

function isScenario(value: string): value is Scenario {
	return SCENARIOS.some((scenario) => scenario === value);
}

const { values: options } = parseArgs({
	options: {
		scenario: { type: "string", default: "" },
		// Once per publisher of the track; own-rust alone when left out.
		publisher: { type: "string", multiple: true, default: [] },
		subscriber: { type: "string", multiple: true, default: [] },
		leaver: { type: "string", multiple: true, default: [] },
		stayer: { type: "string", multiple: true, default: [] },
		"with-publish-done": { type: "boolean", default: false },
		slow: { type: "string", default: "stopped-own-rust" },
		// The four below are for the scenarios with two publishers. They have
		// no default here, so that one that was given where it means nothing
		// can be told and refused.
		first: { type: "string" },
		"first-sends": { type: "string" },
		"second-resends": { type: "boolean" },
		// publish-anew: the new PUBLISH has the Track Alias of the first.
		"same-alias": { type: "boolean", default: false },
		// ask-all: which publishers there are and when they announce.
		ask: { type: "string" },
		// subgroup-priorities: the Publisher Priority of the first and of the
		// second stream of the Subgroup, as "0,1" or "128,default".
		priorities: { type: "string" },
		"second-joins": { type: "string" },
		// subgroup-again: whether the second stream has one Object more or
		// one fewer than the first.
		again: { type: "string" },
		port: { type: "string", default: "24340" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

function hexOf(payload: string): string {
	return Buffer.from(payload).toString("hex");
}

/** A run and the number the relay gave the session of each of its clients. */
interface Run extends ClientRun {
	/** The relay numbers its sessions from 0 in the order it accepts them. */
	sessions: number;
	sessionOf: Map<Publisher | Subscriber, number>;
}

/** The publishers `--publisher` names, in the order given. */
function publisherKinds(): Client[] {
	const given = options.publisher.length > 0 ? options.publisher : ["own-rust"];
	return given.map((value) => parseClient("publisher", value));
}

/** The publisher of a scenario that has one. */
function onePublisherKind(): Client {
	const [kind, ...more] = publisherKinds();
	if (kind === undefined || more.length > 0) {
		throw new Error(
			`--scenario ${options.scenario} has one publisher, and --publisher was given ${options.publisher.length} times`,
		);
	}
	return kind;
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

async function joinAsPublisher(
	kind: Client,
	run: Run,
	plan?: PublisherPlan,
): Promise<Publisher> {
	const publisher = await startPublisher(kind, run, plan);
	run.sessionOf.set(publisher, await sessionOfNewClient(run));
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

function sessionOf(client: Publisher | Subscriber, run: Run): number {
	const session = run.sessionOf.get(client);
	if (session === undefined) {
		throw new Error(`${client.label} did not join through this run`);
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

/**
 * Every subscriber has the Object of the Group, with the payload `from`
 * sends: a publisher, or the run where its publishers all send the same.
 */
async function allReceive(
	subscribers: Subscriber[],
	group: number,
	from: { payloadHex: string },
): Promise<void> {
	await Promise.all(
		subscribers.map(async (subscriber) => {
			const received = await subscriber.object(group);
			if (received !== from.payloadHex) {
				throw new Error(
					`${subscriber.label} received ${received} in Group ${group}, not the sent payload ${from.payloadHex}`,
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
		 * The Stream Count of the PUBLISH_DONE, where it is not the number of
		 * `groups`: with several publishers, the streams the relay opened
		 * towards the subscriber are not one for each Group that was sent.
		 */
		streamCount?: number;
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
	const streamCount = expected.streamCount ?? expected.groups?.length;
	if (
		streamCount !== undefined &&
		outcome.streamCount !== String(streamCount)
	) {
		fail(
			`PUBLISH_DONE has Stream Count ${outcome.streamCount}, not ${streamCount}`,
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
 * Moves one Object over the same track with a fresh publisher and two fresh
 * subscribers, and has the track end with that publisher.
 *
 * WHY the track has to end: a relay that still held a source of the scenario
 * would take the fresh publisher as one more source of the track, and the
 * Object would pass all the same. What such a relay cannot do is end the
 * track when the fresh publisher leaves, since for it a source is left. So
 * the subscriber that stays has to see PUBLISH_DONE.
 *
 * A scenario calls this itself, as its last step and only when every
 * publisher it started has left: one that is still there would make the
 * fresh publisher a second one of the track.
 *
 * It is also where the lines that the scenarios expect to be absent are seen
 * once: a stream opened towards a session, and a session dropped.
 *
 * Returns the fence for what a scenario counts in the relay's log when it is
 * over: the line for the first session of this step. WHY that line: the
 * step starts when every client of the scenario has reported its end, and a
 * client reports what the relay sent it after the relay printed that it
 * sent it. So every line of the scenario stands before this one, and none
 * of this step does.
 */
/** The Group of the Object that is moved after a scenario; the scenarios stay far below it. */
const AFTER_GROUP = 1_000_000;

async function relayServesTheTrackAgain(run: Run): Promise<Fence> {
	const relay = run.context.relay;
	// WHY a label and a payload of its own: the log tells this publisher from
	// the one of the scenario, and an Object of the scenario that came late
	// would not pass for this one.
	// WHY a Group no scenario reaches: a relay that still held a source of
	// the scenario could also remember which Groups it took from it, and
	// would stop a Group 0 sent again. The step would then fail for the
	// Object and not show whether the track ends.
	const publisher = await joinAsPublisher("own-rust", run, {
		label: "pub-after:own-rust",
		firstGroup: AFTER_GROUP,
		payloadHex: hexOf(`after ${run.payloadHex}`),
	});
	const subscriber = await joinAsSubscriber(
		"own-rust",
		"sub-after:own-rust",
		run,
		{
			leavesAfterObjects: 1,
		},
	);
	const staying = await joinAsSubscriber("own-rust", "sub-end:own-rust", run);
	const session = sessionOf(subscriber, run);
	const group = await publisher.send();
	await allReceive([subscriber, staying], group, publisher);
	await relay.waitForLine(streamOpened(session, group));
	await subscriber.leave("close");
	await relay.waitForLine(sessionClosed(session));
	await expectOwnPublishDone(publisher, await publisher.end());
	expectPublishDone(staying, await staying.outcome(), {
		statusCode: TRACK_ENDED,
		groups: [group],
		abortedStreams: 0,
	});
	// §3.3: a control stream is not closed while the session lasts. The relay
	// closes the session of a client that stops its control stream, which
	// own-rust did before it closed its session, on `wtransport`.
	// The subscriber that stays leaves when its subscription has ended, and
	// the relay prints the end of a session when it has printed everything
	// else that end made it do.
	const closedByRelay = await relay.linesBefore(
		{ line: sessionClosed(sessionOf(staying, run)) },
		SESSION_CLOSED_BY_RELAY,
		DISCONNECT_TIMEOUT_MS,
	);
	if (closedByRelay.length > 0) {
		throw new Error(
			`the relay closed a session itself: ${closedByRelay.map((match) => match.input).join("; ")}`,
		);
	}
	return { line: sessionOpened(sessionOf(publisher, run)) };
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
	const kind = onePublisherKind();
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
	const kind = onePublisherKind();
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
	if (!publishDone) return closedWithoutPublishDone(run, subscribers, group);

	await expectRelayFinished(publisher);
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

/**
 * The end of `last-object-then-close` for a publisher that sent no
 * PUBLISH_DONE: its last Object arrives at every subscriber or at none, and
 * the track ends either way.
 *
 * WHY the Object may be missing: nothing tells such a publisher that the
 * relay has read its last stream. When the session ends, a stream that has
 * arrived and was not read is dropped (docs/protocols.md, on what the
 * WebTransport library cost), and so is the rest of one whose header was
 * read. Both were seen: the first in 1 of 200 runs on a loaded machine, the
 * second in CI. PUBLISH_DONE and the wait for the relay's FIN are what a
 * publisher has against it, and the run with --with-publish-done holds that
 * the Object arrives then.
 *
 * WHY not a wait before the close, which would make the Object arrive: the
 * run would then show a delivery that nothing promises.
 */
async function closedWithoutPublishDone(
	run: Run,
	subscribers: Subscriber[],
	group: number,
): Promise<ScenarioResult> {
	// The subscriptions end first: where the Object was lost, a subscriber
	// that is waited on for it would never have it.
	const ended = await outcomes(subscribers);
	const took = ended.filter(({ outcome }) => outcome.groups.includes(group));
	if (took.length !== 0 && took.length !== ended.length) {
		throw new Error(
			`the last Object reached ${took.map(({ subscriber }) => subscriber.label).join(", ")} and not the other own subscribers, of ${ended.length}`,
		);
	}
	const delivered = took.length > 0;
	if (delivered) await allReceive(subscribers, group, run);
	for (const { subscriber, outcome } of ended) {
		if (delivered) {
			expectPublishDone(subscriber, outcome, {
				statusCode: TRACK_ENDED,
				groups: [group],
				abortedStreams: 0,
			});
			continue;
		}
		// The relay had opened a stream towards the subscriber, when it had
		// read the header of the publisher's, or it had not: one stream that
		// ends with the relay's reset, or none.
		if (outcome.streamCount !== "0" && outcome.streamCount !== "1") {
			throw new Error(
				`${subscriber.label}: PUBLISH_DONE has Stream Count ${outcome.streamCount} for a track whose one Object was lost, not 0 or 1: ${outcome.reported}`,
			);
		}
		const opened = Number(outcome.streamCount);
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			streamCount: opened,
			abortedStreams: opened,
		});
		if ((await subscriber.payloads(group)).length !== 0) {
			throw new Error(
				`${subscriber.label} has an Object of Group ${group} that its subscription did not count`,
			);
		}
	}
	const over = await relayServesTheTrackAgain(run);
	await expectPublishDoneLines(ownOnly(subscribers), run, over);

	return {
		summary: delivered
			? "every subscriber received the last Object"
			: "the last Object was lost with the publisher's session, and every own subscriber saw the track end",
		subscriptions: subscriptionsOf(ended),
		measured: { "last Object delivered": delivered ? 1 : 0 },
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
	const publisher = await joinAsPublisher(onePublisherKind(), run);
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
	// The relay's own order of the two lines: the stream of the Group towards
	// a stayer, and the end of the killed session.
	const [stayer] = stayers;
	if (stayer === undefined) throw new Error("the scenario has no stayer");
	const servedAfterLeaving: Fence = {
		line: streamOpened(sessionOf(stayer, run), afterLeaving),
	};
	for (const { subscriber, way } of leavers) {
		if (
			way === "kill" &&
			isRustClient(subscriber.kind) &&
			(
				await relay.linesBefore(
					servedAfterLeaving,
					sessionClosed(sessionOf(subscriber, run)),
				)
			).length > 0
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
	const over = await relayServesTheTrackAgain(run);

	// §11.4.1: ending the request stream ends the subscription, not the
	// session. So the relay opened no stream towards the one that cancelled
	// for what was sent after, and still serves its session.
	const sentAfterLeaving = groups.filter(
		(group) => !beforeLeaving.includes(group),
	);
	for (const subscriber of cancelled) {
		const session = sessionOf(subscriber, run);
		const opened: number[] = [];
		for (const group of sentAfterLeaving) {
			const streams = await relay.linesBefore(
				over,
				streamOpened(session, group),
			);
			if (streams.length > 0) opened.push(group);
		}
		if (opened.length > 0) {
			throw new Error(
				`the relay went on sending to ${subscriber.label} after it cancelled: Groups [${opened}]`,
			);
		}
		if ((await relay.linesBefore(over, sessionClosed(session))).length > 0) {
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
		const codes = (
			await relay.linesBefore(over, stopSendingReceived(session))
		).map((match) => captureGroup(match, 1));
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
	const publisher = await joinAsPublisher(onePublisherKind(), run);
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

	const slowSession = sessionOf(slow, run);
	// Only the subscription was ended: the relay had not dropped the session
	// when it sent the Group after the cut-off on to a reader. The slow one
	// is still stopped, or a page that does not read, and cannot have left
	// by itself. That the relay closes no session itself, at any time, is
	// seen in `relayServesTheTrackAgain`.
	const [reader] = readers;
	if (reader === undefined) throw new Error("the scenario has no reader");
	const dropped = await relay.linesBefore(
		{ line: streamOpened(sessionOf(reader, run), last) },
		sessionClosed(slowSession),
	);
	if (dropped.length > 0) {
		throw new Error(
			`the relay dropped the session of ${slow.label}, where it had to end its subscription only (${Math.round(performance.now() - stopped)} ms after it stopped reading, and cut off ${msUntilCutOff} ms after)`,
		);
	}
	if (!unread) await slow.setStopped(false);
	// The page ends the subscription only when the library's wait for late
	// streams is over, as the Stream Count says nothing:
	// DEFAULT_PUBLISH_DONE_TIMEOUT_MS in
	// packages/moqt/src/webtransport/connection.ts, 5 seconds.
	const cutOff = await slow.outcome();
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
	const over = await relayServesTheTrackAgain(run);
	// §10.11: no stream is opened for a subscription after its PUBLISH_DONE.
	// `last` was sent after the cut-off, and the readers have it.
	const late = await relay.linesBefore(over, streamOpened(slowSession, last));
	if (late.length > 0) {
		throw new Error(
			`the relay opened a stream of Group ${last} towards ${slow.label} after it had ended its subscription`,
		);
	}
	// §3.3.3: the streams the relay cut short say why. Read from the relay's
	// log: they are the streams it could not open, so the subscriber never
	// saw them reset.
	const cutShort = (await relay.linesBefore(over, streamReset)).map((match) =>
		captureGroup(match, 1),
	);
	if (cutShort.length === 0 || cutShort.some((code) => code !== "0x5")) {
		throw new Error(
			`the relay reset ${cutShort.length} streams when it cut ${slow.label} off, with the codes [${[...new Set(cutShort)]}], not TOO_FAR_BEHIND alone`,
		);
	}

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

/**
 * The line `expectStillSubscribed` looks for was printed for every
 * subscriber once the track had ended. Without this, a change of the line's
 * form would make that check pass for the wrong reason.
 */
async function expectPublishDoneLines(
	subscribers: Subscriber[],
	run: Run,
	over: Fence,
): Promise<void> {
	for (const subscriber of subscribers) {
		const session = sessionOf(subscriber, run);
		const sent = await run.context.relay.linesBefore(
			over,
			publishDoneSent(session),
		);
		if (sent.length !== 1) {
			throw new Error(
				`the relay printed ${sent.length} PUBLISH_DONE towards ${subscriber.label} (session ${session}) for the track that ended, not one`,
			);
		}
	}
}

/** The two publishers of a scenario with two sources of one track. */
function twoPublisherKinds(): [Client, Client] {
	const kinds = publisherKinds();
	const [first, second] = kinds;
	if (first === undefined || second === undefined || kinds.length !== 2) {
		throw new Error(
			`--scenario ${options.scenario} has two publishers, and --publisher was given ${options.publisher.length} times`,
		);
	}
	// WHY: the second publisher is the one whose stream the relay stops, or
	// the one that sends a Group after Group 0. The moqtail clients end with
	// an error when their stream is stopped before they have written it out
	// (moqtail 0.15.0), and send Group 0 only.
	if (!isOwnClient(second)) {
		throw new Error(
			`--scenario ${options.scenario} needs an own client as its second publisher, not ${second}`,
		);
	}
	return [first, second];
}

/** `--first-sends`: how many Groups the first publisher sends before the second sends. */
function firstSends(): number {
	const given = options["first-sends"] ?? "1";
	const count = Number(given);
	if (!Number.isInteger(count) || count < 1) {
		throw new Error(`--first-sends "${given}" is not a whole number from 1`);
	}
	return count;
}

/** Fails for an option that was given and means nothing in the scenario. */
function refuseOptions(
	scenario: Scenario,
	names: readonly OptionOfTwoSources[],
): void {
	for (const name of names) {
		const value = options[name];
		if (value !== undefined && value !== false) {
			throw new Error(`--${name} means nothing in --scenario ${scenario}`);
		}
	}
}

/**
 * The subscriptions go on while the track has a source left
 * (docs/moqt-implementation-notes.md, "A source of a track that goes while
 * another is left"): the relay has sent none of the subscribers PUBLISH_DONE.
 * The line is seen in every run, when the last source has gone.
 *
 * `by` is the relay's line for what `when` names.
 */
async function expectStillSubscribed(
	subscribers: Subscriber[],
	run: Run,
	when: string,
	by: Fence,
): Promise<void> {
	for (const subscriber of subscribers) {
		const sent = await run.context.relay.linesBefore(
			by,
			publishDoneSent(sessionOf(subscriber, run)),
			DISCONNECT_TIMEOUT_MS,
		);
		if (sent.length > 0) {
			throw new Error(
				`the relay ended the subscription of ${subscriber.label} ${when}, while the track had another source: ${sent[0]?.[0]}`,
			);
		}
	}
}

/**
 * A publisher leaves in order, and the relay has dealt with it when this
 * returns. What is returned is the fence for what the relay did until then:
 * the end of the publisher's session.
 *
 * WHY its session is waited for: the relay prints `SESSION_CLOSED` when it
 * has printed everything else the end of that session made it do (`feed` in
 * crates/relay/src/hub.rs). A check for a line the relay must not print is
 * sound only after that; a client's own word that it has left comes earlier.
 * WHY a page is closed for it: `end()` of the own page ends its Track and
 * keeps its session, so there would be nothing to wait for.
 */
async function leaveInOrder(publisher: Publisher, run: Run): Promise<Fence> {
	await expectOwnPublishDone(publisher, await publisher.end());
	if (isBrowserClient(publisher.kind)) await publisher.kill();
	const gone = sessionClosed(sessionOf(publisher, run));
	await run.context.relay.waitForLine(gone, DISCONNECT_TIMEOUT_MS);
	return { line: gone };
}

/**
 * The relay has stopped `count` subgroup streams of the publisher's session,
 * each with `code`, CANCELLED where it is left out: what it does with a
 * stream for a Subgroup it took from another source. That it stopped no
 * more than these is for `expectStreamsStoppedInAll`, when the scenario is
 * over.
 *
 * Returns the fence for what the relay did with those streams: the line for
 * the last of them. The relay decides on the header of a stream whether it
 * stops it or sends it on, so a stream it had sent on stands before.
 */
async function expectStreamsStopped(
	publisher: Publisher,
	count: number,
	run: Run,
	code = "0x1",
): Promise<Fence> {
	const line = dataStreamStopped(sessionOf(publisher, run));
	const codes = (await run.context.relay.waitForLines(line, count)).map(
		(match) => captureGroup(match, 1),
	);
	if (codes.some((each) => each !== code)) {
		throw new Error(
			`the relay stopped streams of ${publisher.label} with the codes [${codes}], not ${count} times ${code}`,
		);
	}
	return { line, nth: count };
}

/** The relay stopped `count` subgroup streams of the publisher's session in the whole scenario, and no other. */
async function expectStreamsStoppedInAll(
	publisher: Publisher,
	count: number,
	run: Run,
	over: Fence,
): Promise<void> {
	const stopped = await run.context.relay.linesBefore(
		over,
		dataStreamStopped(sessionOf(publisher, run)),
	);
	if (stopped.length !== count) {
		throw new Error(
			`the relay stopped ${stopped.length} streams of ${publisher.label} in all, not ${count}: ${stopped.map((match) => match[0]).join("; ")}`,
		);
	}
}

/**
 * The relay opened one stream of each Group towards each subscriber, and no
 * second one, before `by`.
 */
async function expectOneStreamEach(
	subscribers: Subscriber[],
	groups: number[],
	run: Run,
	by: Fence,
): Promise<void> {
	for (const subscriber of subscribers) {
		for (const group of groups) {
			const opened = await run.context.relay.linesBefore(
				by,
				streamOpened(sessionOf(subscriber, run), group),
			);
			if (opened.length !== 1) {
				throw new Error(
					`the relay opened ${opened.length} streams of Group ${group} towards ${subscriber.label}, not 1`,
				);
			}
		}
	}
}

/**
 * Every own subscriber got one Object for each of the Groups, with the
 * payload of `from`: what a subscriber itself shows of a Subgroup that came
 * to it twice. Called when the subscriptions have ended, so that nothing is
 * still on its way.
 *
 * WHY NOT `Subscriber.object`: it gives the first Object of a Group and says
 * nothing of a second one.
 */
async function expectOneObjectEach(
	subscribers: Subscriber[],
	groups: number[],
	from: { label: string; payloadHex: string },
): Promise<void> {
	for (const subscriber of ownOnly(subscribers)) {
		for (const group of groups) {
			const payloads = await subscriber.payloads(group);
			if (payloads.length !== 1 || payloads[0] !== from.payloadHex) {
				throw new Error(
					`${subscriber.label} got ${payloads.length} Objects for Group ${group}, not the one of ${from.label}: [${payloads.map((payload) => (payload === from.payloadHex ? `that of ${from.label}` : payload.slice(0, 32))).join(", ")}]`,
				);
			}
		}
	}
}

/**
 * Two publishers send the same Subgroup of one track, one after the other.
 * The notes, "A stream for a Subgroup from another source than the one it
 * was taken from": the relay takes the Subgroup from the source that sent it
 * first, stops the other's stream and sends nothing of it downstream.
 */
async function sameGroupTwice(run: Run): Promise<ScenarioResult> {
	refuseOptions("same-group-twice", [
		"first",
		"second-joins",
		"second-resends",
	]);
	const [firstKind, secondKind] = twoPublisherKinds();
	const count = firstSends();
	// WHY payloads of their own where the first is an own client: what a
	// subscriber holds for the Group then shows which source it came from. A
	// moqtail client sends the payload of the run and nothing else, so beside
	// it the two send the same, as two sources of a Subgroup do.
	const apart = isOwnClient(firstKind);
	const first = await joinAsPublisher(firstKind, run, {
		label: `pub1:${firstKind}`,
		...(apart ? { payloadHex: hexOf(`first ${run.payloadHex}`) } : {}),
	});
	// "PUBLISH for a track that already has a source": accepted.
	const second = await joinAsPublisher(secondKind, run, {
		label: `pub2:${secondKind}`,
		...(apart ? { payloadHex: hexOf(`second ${run.payloadHex}`) } : {}),
	});
	const subscribers = await joinAsSubscribers(
		options.subscriber.map((value) => parseClient("subscriber", value)),
		"sub",
		run,
	);

	const group = await first.send();
	await allReceive(subscribers, group, first);
	const groups = [group];
	if (count > 1) {
		groups.push(...(await first.sendBatch(count - 1)));
		await allReceive(ownOnly(subscribers), count - 1, first);
	}
	// WHY the second only now: which source is first is then not a race.
	const again = await second.send();
	if (again !== group) {
		throw new Error(
			`${second.label} sent Group ${again}, where the scenario needs Group ${group} a second time`,
		);
	}
	const stopped = await expectStreamsStopped(second, 1, run);
	await expectOneStreamEach(subscribers, [group], run, stopped);

	// The source the Subgroups came from leaves first; the track stays for the other.
	const firstGone = await leaveInOrder(first, run);
	await expectStillSubscribed(
		subscribers,
		run,
		`when ${first.label} had left`,
		firstGone,
	);
	// The one stream the second publisher counts in its PUBLISH_DONE is the
	// one the relay stopped. A stream it stopped has ended for the relay, so
	// an own publisher sees the relay finish the request stream all the same
	// (checked in `expectOwnPublishDone`).
	await expectOwnPublishDone(second, await second.end());
	const ended = await outcomes(subscribers);
	await expectOneObjectEach(subscribers, groups, first);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups,
			abortedStreams: 0,
		});
	}
	const over = await relayServesTheTrackAgain(run);
	await expectPublishDoneLines(ownOnly(subscribers), run, over);
	await expectStreamsStoppedInAll(second, 1, run, over);

	return {
		summary: `the relay took ${count === 1 ? `Group ${group}` : `Groups ${group} to ${count - 1}`} from ${first.label} and stopped the stream of ${second.label} for Group ${group}; every subscriber got it once`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

const FIRST_SOURCE_WAYS = [
	"done",
	"kill",
	"kill-with-open-stream",
	"stays",
] as const;

/**
 * A track with two sources, of which the first sends, and then the second
 * sends the next Group: after the first left (`--first done`, `kill`,
 * `kill-with-open-stream`) or beside it (`stays`). With `--second-joins
 * after`, the second publishes when the subscribers already have an Object.
 *
 * The notes, "A source of a track that goes while another is left": the track
 * goes on, nothing is said to the subscribers, and a stream that was open for
 * a Subgroup of the source that went is reset.
 */
async function secondSource(run: Run): Promise<ScenarioResult> {
	const [firstKind, secondKind] = twoPublisherKinds();
	const given = options.first ?? "done";
	const way = FIRST_SOURCE_WAYS.find((value) => value === given);
	if (way === undefined) {
		throw new Error(
			`--first "${given}" is not one of ${FIRST_SOURCE_WAYS.join(", ")}`,
		);
	}
	const joins = options["second-joins"] ?? "before";
	if (joins !== "before" && joins !== "after") {
		throw new Error(`--second-joins "${joins}" is neither before nor after`);
	}
	const late = joins === "after";
	const sends = firstSends();
	const resends = options["second-resends"] === true;
	// WHY done alone: what the second sends again has to be stopped because
	// the first delivered it whole, and a first that was killed, or left a
	// stream open, makes that a matter of which of its streams had ended.
	if (resends && way !== "done") {
		throw new Error(`--second-resends goes with --first done, not ${way}`);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	// WHY: the moqtail clients take one Object and leave; here a subscriber
	// has to stay for the Group of the second publisher.
	const passing = subscriberKinds.find((kind) => !isOwnClient(kind));
	if (passing !== undefined) {
		throw new Error(
			`--scenario second-source needs own subscribers, which stay for more than one Object, not ${passing}`,
		);
	}
	const relay = run.context.relay;
	// WHY payloads of their own: a subscriber then shows which source a Group
	// came from.
	const first = await joinAsPublisher(firstKind, run, {
		label: `pub1:${firstKind}`,
		payloadHex: hexOf(`first ${run.payloadHex}`),
	});
	// The first source sends `--first-sends` Groups, and one more on a stream
	// it leaves open where that is asked for. The second starts after those,
	// or, with --second-resends, at Group 0: a source that is behind and
	// sends again what the first has sent, before it sends the next Group.
	const firstGroups = sends + (way === "kill-with-open-stream" ? 1 : 0);
	const joinSecond = (): Promise<Publisher> =>
		joinAsPublisher(secondKind, run, {
			label: `pub2:${secondKind}`,
			firstGroup: resends ? 0 : firstGroups,
			payloadHex: hexOf(`second ${run.payloadHex}`),
		});
	let second = late ? undefined : await joinSecond();
	const subscribers = await joinAsSubscribers(subscriberKinds, "sub", run);

	const fromFirst = [await first.send()];
	await allReceive(subscribers, 0, first);
	if (sends > 1) {
		fromFirst.push(...(await first.sendBatch(sends - 1)));
		await allReceive(subscribers, sends - 1, first);
	}
	if (way === "kill-with-open-stream") {
		const open = await first.sendUnfinished();
		fromFirst.push(open);
		await allReceive(subscribers, open, first);
	}
	// "PUBLISH for a track that already has a source": accepted "whether or
	// not the track has subscribers".
	second ??= await joinSecond();

	const measured: Record<string, number> = {};
	// The relay's line for the end of the first publisher's session, once
	// it has gone.
	let firstGone: Fence | undefined;
	switch (way) {
		case "done":
			firstGone = await leaveInOrder(first, run);
			break;
		case "kill":
		case "kill-with-open-stream": {
			const killed = performance.now();
			await first.kill();
			firstGone = { line: sessionClosed(sessionOf(first, run)) };
			await relay.waitForLine(firstGone.line, DISCONNECT_TIMEOUT_MS);
			const ms = Math.round(performance.now() - killed);
			measured[
				`ms from killing ${first.label} to the relay dropping its session`
			] = ms;
			// A killed process is noticed by the idle timeout; a closed page says so itself.
			if (
				!isBrowserClient(firstKind) &&
				ms > PUBLISHER_KILL_NOTICED_WITHIN_MS
			) {
				throw new Error(
					`the relay dropped the session of ${first.label} ${seconds(ms)} after it was killed, later than ${seconds(PUBLISHER_KILL_NOTICED_WITHIN_MS)}`,
				);
			}
			break;
		}
		case "stays":
			break;
	}
	if (firstGone !== undefined) {
		await expectStillSubscribed(
			subscribers,
			run,
			`when ${first.label} was gone`,
			firstGone,
		);
	}

	let stoppedInAll = 0;
	if (resends) {
		// "A stream for a Subgroup from another source…": stopped, also after
		// the first source's stream ended with a FIN and after that source has
		// gone, and nothing of it goes downstream. That holds for the Groups
		// the relay no longer remembers a source for as well, on a track that
		// had two sources at once.
		const again = await second.sendBatch(firstGroups);
		stoppedInAll = again.length;
		const stopped = await expectStreamsStopped(second, again.length, run);
		await expectOneStreamEach(subscribers, again, run, stopped);
	}
	const next = await second.send();
	await allReceive(subscribers, next, second);

	if (way === "stays") {
		await expectStillSubscribed(
			subscribers,
			run,
			`when ${first.label} had left`,
			await leaveInOrder(first, run),
		);
	}
	await expectOwnPublishDone(second, await second.end());
	const ended = await outcomes(subscribers);
	// What a subscriber holds, Group by Group: one Object each, the first
	// publisher's for what that one sent, also where the second sent it again.
	await expectOneObjectEach(subscribers, fromFirst, first);
	await expectOneObjectEach(subscribers, [next], second);
	const groups = [...fromFirst, next];
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups,
			// "Stream Count in the PUBLISH_DONE the relay sends": the streams it
			// opened for the subscription, one a Group here, the reset one
			// among them.
			streamCount: groups.length,
			abortedStreams: first.streamsLeftOpen(),
		});
	}
	const over = await relayServesTheTrackAgain(run);
	await expectPublishDoneLines(ownOnly(subscribers), run, over);
	await expectStreamsStoppedInAll(second, stoppedInAll, run, over);

	return {
		summary: `${first.label} ${way === "stays" ? "stayed" : `went (${way})`} and the subscribers went on to Group ${next} of ${second.label}${late ? ", which joined after them" : ""}`,
		subscriptions: subscriptionsOf(ended),
		measured,
	};
}

/**
 * A publisher sends PUBLISH for a track while the relay waits for another
 * publisher's answer to its SUBSCRIBE for that track.
 *
 * The notes, "PUBLISH for a track the relay knows already": "A track the
 * relay has asked a publisher for and has no answer about: the PUBLISH gives
 * it its source, the subscribers that waited get SUBSCRIBE_OK with what the
 * PUBLISH brought, and what the relay asked stays asked." And "The
 * publisher's answer to the relay's SUBSCRIBE", when the track got a source
 * meanwhile: "SUBSCRIBE_OK: the publisher is a further source".
 */
async function publishWhileAsked(run: Run): Promise<ScenarioResult> {
	const [askedKind, otherKind] = twoPublisherKinds();
	if (askedKind !== "own-rust") {
		throw new Error(
			`--scenario publish-while-asked needs own-rust as its first publisher, the one that announces and is asked, not ${askedKind}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((kind) => !isOwnClient(kind));
	if (passing !== undefined) {
		throw new Error(
			`--scenario publish-while-asked needs own subscribers, which stay for more than one Object, not ${passing}`,
		);
	}
	if (otherKind !== "own-rust") {
		throw new Error(
			`--scenario publish-while-asked needs own-rust as its second publisher, which holds its PUBLISH back until it is told, not ${otherKind}`,
		);
	}
	const relay = run.context.relay;
	// WHY both publishers are there before a subscriber is, each held back
	// by the driver: the relay waits 5 seconds (provisional) for the answer
	// to its SUBSCRIBE, and the PUBLISH has to come in that time. With a
	// publisher that started when the relay had asked, and an answer that
	// came 3 seconds late by itself, the start of a process and its
	// connection had to fit into the 3 seconds, and did not on a loaded
	// machine (6 of 24 rounds with four matrices at once, and once in CI).
	// The subscribers are connected before any of them subscribes, too
	// (`subscribesWhen`): what has to fit into the relay's 5 seconds is
	// messages between peers that are connected.
	//
	// It announces the namespace and waits to be asked. Its Groups start
	// after the one the other publisher sends, as a source that goes on with
	// the track's Group IDs does.
	const asked = await joinAsPublisher(askedKind, run, {
		label: `asked:${askedKind}`,
		firstGroup: 1,
		payloadHex: hexOf(`asked ${run.payloadHex}`),
		servesWhenAsked: "on-line",
	});
	// It announces the namespace as well, as every own-rust publisher does,
	// so the relay asks it too; its PUBLISH then makes the relay's SUBSCRIBE
	// to it give way (the notes, "A PUBLISH for a track the relay has asked
	// a publisher for and has no answer about").
	const other = await joinAsPublisher(otherKind, run, {
		label: `pub:${otherKind}`,
		payloadHex: hexOf(`other ${run.payloadHex}`),
		publishesOnLine: true,
	});
	const askedSession = sessionOf(asked, run);
	const otherSession = sessionOf(other, run);

	// The subscribers subscribe and are not answered: "Subscribers while the
	// publisher has not answered the relay's SUBSCRIBE: They wait, and no
	// SUBSCRIBE_OK is sent."
	let subscribeNow = (): void => {};
	const subscribesWhen = new Promise<void>((resolve) => {
		subscribeNow = resolve;
	});
	const waiting: { session: number; subscriber: Promise<Subscriber> }[] = [];
	for (const [index, kind] of subscriberKinds.entries()) {
		const subscriber = startSubscriber(kind, `sub${index + 1}:${kind}`, run, {
			subscribesWhen,
		});
		// Observed here already, so that a failure while it waits is not unhandled.
		subscriber.catch(() => {});
		waiting.push({ session: await sessionOfNewClient(run), subscriber });
	}
	subscribeNow();
	// The relay's SUBSCRIBE has come to the publisher, which holds its
	// answer back. The first subscriber's SUBSCRIBE is what made the relay
	// ask. Of a later subscriber the relay prints nothing while it waits, so
	// that its SUBSCRIBE was read before the PUBLISH is not certain; what is
	// checked for each is that it was not answered before the PUBLISH.
	await relay.waitForLine(requestOpenedTowards(askedSession));
	await asked.asked();

	/**
	 * Runs a step that fails when the relay has given up on the publisher
	 * that was asked, and says so then, in place of what the step failed
	 * with.
	 *
	 * WHY this is said apart from a fault of the relay: it is a matter of
	 * time, the relay's 5 seconds, and the run then shows nothing.
	 */
	const unlessTheRelayGaveUp = async <T>(
		step: Promise<T>,
		shows: string,
	): Promise<T> => {
		try {
			return await step;
		} catch (error) {
			// The relay's line and the client's failure reach the driver in
			// no order, so the line is given a moment.
			const gaveUp = await relay
				.waitForLine(ownRequestEnded(askedSession), 2_000)
				.catch(() => undefined);
			if (gaveUp === undefined) throw error;
			throw new Error(
				`the relay had ended its SUBSCRIBE to ${asked.label} (${gaveUp[0]}), for which it waits 5 seconds, so the run does not show that ${shows}: ${String(error)}`,
			);
		}
	};
	const answersWhileWaiting =
		"a PUBLISH answers subscribers while the relay waits";

	await other.publish();
	// The PUBLISH gave the track its source: the subscribers are answered.
	const subscribers: Subscriber[] = [];
	for (const { session, subscriber } of waiting) {
		const answered = await unlessTheRelayGaveUp(
			withTimeout(
				subscriber,
				`the subscriber of session ${session} to be answered after the PUBLISH`,
			),
			answersWhileWaiting,
		);
		run.sessionOf.set(answered, session);
		subscribers.push(answered);
	}
	// The relay's PUBLISH_OK: the second REQUEST_OK it sends the publisher,
	// after the one for its namespace. It is sent when the PUBLISH was dealt
	// with.
	const publishOk: Fence = {
		line: new RegExp(`^SEND session=${otherSession} stream=\\d+ REQUEST_OK`),
		nth: 2,
	};
	const answerOfAsked = new RegExp(
		`^RECV session=${askedSession} own=\\d+ SUBSCRIBE_OK`,
	);
	// The relay had not given up on the publisher that was asked, and had no
	// answer from it, when it dealt with the PUBLISH: its own request to
	// that session was open, so the PUBLISH came "while the relay waits".
	const givenUp = await relay.linesBefore(
		publishOk,
		ownRequestEnded(askedSession),
	);
	if (givenUp.length > 0) {
		throw new Error(
			`the relay had ended its SUBSCRIBE to ${asked.label} (${givenUp[0]?.[0]}), for which it waits 5 seconds, before the PUBLISH of ${other.label} was dealt with, so the run does not show that ${answersWhileWaiting}`,
		);
	}
	// The publisher holds its answer back until it is told below, so an
	// answer before the PUBLISH is a fault of the client or of this driver.
	const answeredByAsked = await relay.linesBefore(publishOk, answerOfAsked);
	if (answeredByAsked.length > 0) {
		throw new Error(
			`${asked.label} answered the relay's SUBSCRIBE before it was told to: ${answeredByAsked[0]?.[0]}`,
		);
	}
	// Nobody was answered before the PUBLISH was dealt with: with the
	// publisher that was asked silent until then, no publisher had the
	// track. And each was answered before that publisher answered.
	for (const { session } of waiting) {
		const early = await relay.linesBefore(publishOk, subscribeOkSent(session));
		if (early.length > 0) {
			throw new Error(
				`the relay answered the subscriber of session ${session} before any publisher had the track: ${early[0]?.[0]}`,
			);
		}
	}
	const first = await other.send();
	await allReceive(subscribers, first, other);

	// "What the relay asked stays asked": the answer makes the publisher a
	// further source, and its Group reaches the subscribers.
	//
	// The relay may have given up on the publisher by now: the client then
	// fails to send its SUBSCRIBE_OK, and ends.
	await unlessTheRelayGaveUp(asked.serve(), "what the relay asked stays asked");
	// The relay prints the answer in every run: a change of the line's form
	// would otherwise make the check above pass for the wrong reason.
	await relay.waitForLine(answerOfAsked);
	const second = await asked.send();
	await allReceive(subscribers, second, asked);

	await expectStillSubscribed(
		subscribers,
		run,
		`when ${asked.label} had left`,
		await leaveInOrder(asked, run),
	);
	await expectOwnPublishDone(other, await other.end());
	const ended = await outcomes(subscribers);
	await expectOneObjectEach(subscribers, [first], other);
	await expectOneObjectEach(subscribers, [second], asked);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups: [first, second],
			abortedStreams: 0,
		});
	}
	await expectPublishDoneLines(
		subscribers,
		run,
		await relayServesTheTrackAgain(run),
	);

	return {
		summary: `the subscribers were answered by the PUBLISH of ${other.label} while ${asked.label} had not answered, and got Group ${first} from the one and Group ${second} from the other`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

function parsePriority(text: string): PublisherPriority {
	if (text === "default") return text;
	const value = Number(text);
	if (!/^\d+$/.test(text) || value > 255) {
		throw new Error(`"${text}" is no Publisher Priority: 0 to 255, or default`);
	}
	return value;
}

/** The priority a header with this setting means: 128 where it is left out and the publisher gave no default (§11.4.2). */
const priorityMeant = (priority: PublisherPriority): number =>
	priority === "default" ? 128 : priority;

/** The relay reset or stopped a request stream of the session; capture group 1 is the code. */
const requestCancelled = (session: number): RegExp =>
	new RegExp(
		`^(?:RESET|STOP)_REQUEST session=${session} stream=\\d+ code=(\\S+)$`,
	);

/**
 * A Subgroup comes to the relay a second time, with the same Publisher
 * Priority or another: from a second publisher, or, with one own-browser
 * publisher, from the same one.
 *
 * The notes, "A Subgroup that comes again with another Publisher Priority
 * (relay)": "Malformed Track: every subscription ends with PUBLISH_DONE
 * `MALFORMED_TRACK`, and every source is cancelled [...] The stream that
 * brought the other priority is stopped with `MALFORMED_TRACK` and its source
 * is the one that gets that code. Which of the two is wrong the relay cannot
 * know; the one that came first is taken as given." "A Subgroup that two
 * sources send with the same priority, one in the header and one by its
 * default, is no difference". And "A malformed track that has several
 * sources": "the one that sent what broke the object model with
 * `MALFORMED_TRACK`, the others with `CANCELLED`".
 */
async function subgroupPriorities(run: Run): Promise<ScenarioResult> {
	const [firstText, secondText, ...more] = (options.priorities ?? "").split(
		",",
	);
	if (firstText === undefined || secondText === undefined || more.length > 0) {
		throw new Error(
			`--priorities "${options.priorities}" is not <first>,<second>, each 0 to 255 or default`,
		);
	}
	const priorities = [parsePriority(firstText), parsePriority(secondText)];
	const [firstPriority, secondPriority] = priorities;
	if (firstPriority === undefined || secondPriority === undefined) {
		throw new Error("two priorities are needed");
	}
	const differ = priorityMeant(firstPriority) !== priorityMeant(secondPriority);
	const kinds = publisherKinds();
	const [firstKind, secondKind] = kinds;
	if (firstKind === undefined || kinds.length > 2) {
		throw new Error("--scenario subgroup-priorities has one or two publishers");
	}
	if (secondKind === undefined && firstKind !== "own-browser") {
		throw new Error(
			`one publisher that sends a Subgroup twice has to be own-browser, the one client that can, not ${firstKind}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((kind) => !isOwnClient(kind));
	if (passing !== undefined) {
		throw new Error(
			`--scenario subgroup-priorities needs own subscribers, which say how their subscription ended, not ${passing}`,
		);
	}
	const relay = run.context.relay;
	const first = await joinAsPublisher(firstKind, run, {
		label: `pub1:${firstKind}`,
		payloadHex: hexOf(`first ${run.payloadHex}`),
		publisherPriority: firstPriority,
	});
	const second =
		secondKind === undefined
			? undefined
			: await joinAsPublisher(secondKind, run, {
					label: `pub2:${secondKind}`,
					payloadHex: hexOf(`second ${run.payloadHex}`),
					publisherPriority: secondPriority,
				});
	const subscribers = await joinAsSubscribers(subscriberKinds, "sub", run);

	// WHY the first stream is delivered before the second is sent: "the one
	// that came first is taken as given", and which came first is then not a
	// race. The stream has ended with a FIN by then; the clients finish
	// every stream they send here, so a Subgroup that comes again while its
	// first stream is open is not run.
	const group = await first.send();
	await allReceive(subscribers, group, first);
	// The one whose stream comes second, and that stream's session.
	const again = second ?? first;
	// WHY `sendAgain` for a page: the relay stops this stream, and may do so
	// before the page has written its Object (`Publisher.sendAgain`). An own
	// Rust client reports a stopped stream as sent.
	if (second === undefined) {
		await first.sendAgain(group, secondPriority);
	} else if (second.kind === "own-browser") {
		await second.sendAgain(group, secondPriority);
	} else {
		const sent = await second.send();
		if (sent !== group) {
			throw new Error(
				`${second.label} sent Group ${sent}, where the scenario needs Group ${group} a second time`,
			);
		}
	}
	// What the relay does with the second stream: one stop, with the code.
	// That it is the only one is seen when the scenario is over.
	const stopped = await expectStreamsStopped(
		again,
		1,
		run,
		`0x${(differ ? MALFORMED_TRACK : 0x1).toString(16)}`,
	);

	if (!differ) {
		// "no difference": the track goes on, and ends when its publishers leave.
		if (second === undefined) {
			throw new Error("the same priority twice is run with two publishers");
		}
		await expectOneStreamEach(subscribers, [group], run, stopped);
		await expectStillSubscribed(
			subscribers,
			run,
			`when ${first.label} had left`,
			await leaveInOrder(first, run),
		);
		await expectOwnPublishDone(second, await second.end());
		const ended = await outcomes(subscribers);
		await expectOneObjectEach(subscribers, [group], first);
		for (const { subscriber, outcome } of ended) {
			expectPublishDone(subscriber, outcome, {
				statusCode: TRACK_ENDED,
				groups: [group],
				abortedStreams: 0,
			});
		}
		const over = await relayServesTheTrackAgain(run);
		await expectPublishDoneLines(subscribers, run, over);
		await expectStreamsStoppedInAll(again, 1, run, over);
		return {
			summary: `the relay took Group ${group} from ${first.label} with priority ${firstText} and stopped the stream of ${second.label} with ${secondText} as one more of the same Subgroup; the track went on`,
			subscriptions: subscriptionsOf(ended),
			measured: {},
		};
	}

	// "every subscription ends with PUBLISH_DONE MALFORMED_TRACK". The Group
	// they have is the one that came first, whole.
	const ended = await outcomes(subscribers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: MALFORMED_TRACK,
			groups: [group],
			abortedStreams: 0,
		});
	}
	await expectOneObjectEach(subscribers, [group], first);
	// "every source is cancelled": the one whose stream brought the other
	// priority with MALFORMED_TRACK, another with CANCELLED.
	const sources = [
		{ publisher: again, code: "0x12" },
		...(second === undefined ? [] : [{ publisher: first, code: "0x1" }]),
	];
	for (const { publisher } of sources) {
		await relay.waitForLine(requestCancelled(sessionOf(publisher, run)));
	}
	// The publishers are still there: an own-rust one closes its session
	// when its PUBLISH is ended, a page is closed. The end of a session is
	// the fence for every line about its requests.
	for (const { publisher, code } of sources) {
		if (isBrowserClient(publisher.kind)) await publisher.kill();
		const session = sessionOf(publisher, run);
		const codes = (
			await relay.linesBefore(
				{ line: sessionClosed(session) },
				requestCancelled(session),
				DISCONNECT_TIMEOUT_MS,
			)
		).map((match) => captureGroup(match, 1));
		if (codes.length === 0 || codes.some((each) => each !== code)) {
			throw new Error(
				`the relay cancelled the PUBLISH of ${publisher.label} with [${codes}], not with ${code}`,
			);
		}
		if (publisher.kind === "own-rust") {
			const cancelled = await publisher.publishCancelledWith();
			if (cancelled !== code) {
				throw new Error(
					`${publisher.label} saw its PUBLISH ended with ${cancelled}, not ${code}`,
				);
			}
		}
	}
	const over = await relayServesTheTrackAgain(run);
	await expectPublishDoneLines(subscribers, run, over);
	await expectStreamsStoppedInAll(again, 1, run, over);

	return {
		summary: `the relay ended the track as malformed when ${again.label} sent Group ${group} again with priority ${secondText} after ${firstText}: PUBLISH_DONE 0x12 to the subscribers, 0x12 to ${again.label}${second === undefined ? "" : `, 0x1 to ${first.label}`}`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

/**
 * An Object past the end of the track (§2.4.2: "An Object is received whose
 * Group and Object ID are larger than the final Object in the Track. The
 * final Object in a Track is the Object with Status END_OF_TRACK"): the relay
 * ends the track as malformed, as for a Subgroup that comes again with
 * another priority (`subgroupPriorities`).
 */
async function endOfTrack(run: Run): Promise<ScenarioResult> {
	const kind = onePublisherKind();
	if (kind !== "own-rust") {
		throw new Error(
			`--scenario end-of-track needs own-rust, the one client that sends END_OF_TRACK, not ${kind}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((each) => !isOwnClient(each));
	if (passing !== undefined) {
		throw new Error(
			`--scenario end-of-track needs own subscribers, which say how their subscription ended, not ${passing}`,
		);
	}
	const publisher = await joinAsPublisher(kind, run, {
		label: `pub:${kind}`,
		payloadHex: run.payloadHex,
	});
	if (publisher.sendEndOfTrack === undefined) {
		throw new Error(`${publisher.label} cannot send END_OF_TRACK`);
	}
	const subscribers = await joinAsSubscribers(subscriberKinds, "sub", run);
	const group = await publisher.send();
	await allReceive(subscribers, group, publisher);
	const end = await publisher.sendEndOfTrack();
	const past = await publisher.send();
	const ended = await outcomes(subscribers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, { statusCode: MALFORMED_TRACK });
	}
	const session = sessionOf(publisher, run);
	const code = captureGroup(
		await run.context.relay.waitForLine(requestCancelled(session)),
		1,
	);
	if (code !== "0x12") {
		throw new Error(
			`the relay cancelled the PUBLISH of ${publisher.label} with ${code}, not 0x12`,
		);
	}
	const cancelled = await publisher.publishCancelledWith();
	return {
		summary: `the relay ended the track as malformed when ${publisher.label} sent Group ${past} after END_OF_TRACK in Group ${end}: PUBLISH_DONE 0x12 to the subscribers, the PUBLISH cancelled with ${code} (${publisher.label} saw ${cancelled})`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

/**
 * A Subgroup on a second stream after it was delivered whole, with one
 * Object more or one fewer (§2.4.2, the second and the third example): the
 * relay ends the track as malformed, as for an Object past the end of the
 * track (`endOfTrack`).
 */
async function subgroupAgain(run: Run): Promise<ScenarioResult> {
	const again = options.again;
	if (again !== "longer" && again !== "shorter") {
		throw new Error(
			`--scenario subgroup-again needs --again longer or shorter, not ${again}`,
		);
	}
	const kind = onePublisherKind();
	if (kind !== "own-rust") {
		throw new Error(
			`--scenario subgroup-again needs own-rust, the one client that sends a Subgroup again, not ${kind}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((each) => !isOwnClient(each));
	if (passing !== undefined) {
		throw new Error(
			`--scenario subgroup-again needs own subscribers, which say how their subscription ended, not ${passing}`,
		);
	}
	const publisher = await joinAsPublisher(kind, run, {
		label: `pub:${kind}`,
		payloadHex: run.payloadHex,
	});
	if (publisher.sendSubgroupAgain === undefined) {
		throw new Error(`${publisher.label} cannot send a Subgroup again`);
	}
	const subscribers = await joinAsSubscribers(subscriberKinds, "sub", run);
	const group = await publisher.send();
	await allReceive(subscribers, group, publisher);
	const twice = await publisher.sendSubgroupAgain(again === "longer", (group) =>
		deliveredWhole(group, subscribers.length, run),
	);
	const ended = await outcomes(subscribers);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, { statusCode: MALFORMED_TRACK });
	}
	const session = sessionOf(publisher, run);
	const code = captureGroup(
		await run.context.relay.waitForLine(requestCancelled(session)),
		1,
	);
	if (code !== "0x12") {
		throw new Error(
			`the relay cancelled the PUBLISH of ${publisher.label} with ${code}, not 0x12`,
		);
	}
	const cancelled = await publisher.publishCancelledWith();
	const what =
		again === "longer"
			? "an Object past the final one on its second stream"
			: "its second stream ended with a FIN after another final Object";
	return {
		summary: `the relay ended the track as malformed when ${publisher.label} sent Group ${twice} again after it was delivered whole and ${what}: PUBLISH_DONE 0x12 to the subscribers, the PUBLISH cancelled with ${code} (${publisher.label} saw ${cancelled})`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

/**
 * Resolves once the relay has finished its stream of the Group towards each
 * of `subscribers` sessions: the first stream of `subgroupAgain` was taken
 * whole and passed on whole.
 *
 * WHY the second stream waits for this, and not only for the FIN of the
 * first that the publisher wrote: QUIC keeps no order between two streams,
 * so the second, opened after that FIN was written, can still reach the
 * relay first. The relay then holds it as the Subgroup's stream and ends
 * the first (§11.4), and what the run is about, a Subgroup delivered whole
 * and sent again, did not happen. In a run of CI (#339, 2026-10-25) the
 * short stream came first: the relay reset its stream towards the
 * subscriber with 0x1, stopped the publisher's with 0x1, and passed on the
 * long one to its end.
 */
async function deliveredWhole(
	group: number,
	subscribers: number,
	run: Run,
): Promise<void> {
	const opened = await run.context.relay.waitForLines(
		new RegExp(`^OPEN_DATA session=\\d+ out=(\\d+) .* group=${group}$`),
		subscribers,
	);
	for (const line of opened) {
		const out = captureGroup(line, 1);
		await run.context.relay.waitForLines(
			new RegExp(`^FINISH_DATA out=${out}$`),
			1,
		);
	}
}

const ASK_WAYS = [
	"two-announcers",
	"announcer-and-publisher",
	"late-announcer",
	"refusing-announcer",
] as const;

/** Resolves once the relay has printed `count` lines that match. */
async function relayPrinted(
	pattern: RegExp,
	count: number,
	run: Run,
): Promise<void> {
	await run.context.relay.waitForLines(pattern, count);
}

/**
 * Throws unless the relay had asked each session as often as given, with a
 * SUBSCRIBE of its own, before `by`: the relay's line for what `when` names.
 */
async function expectAsks(
	expected: { who: string; session: number; asks: number }[],
	run: Run,
	when: string,
	by: Fence,
): Promise<void> {
	for (const { who, session, asks } of expected) {
		const seen = (
			await run.context.relay.linesBefore(
				by,
				requestOpenedTowards(session),
				DISCONNECT_TIMEOUT_MS,
			)
		).length;
		if (seen !== asks) {
			throw new Error(
				`the relay had sent ${who} (session ${session}) ${seen} SUBSCRIBE(s) ${when}, not ${asks}`,
			);
		}
	}
}

/**
 * The relay asks every publisher that announced a matching namespace for a
 * track, and none that is a source of it already.
 *
 * The notes, "A SUBSCRIBE for a track that has no source, and which
 * publishers the relay asks": "The relay sends its own SUBSCRIBE to every
 * publisher whose announced namespace matches [...] They are asked in the
 * order they announced". "The same is done for a track that has a source,
 * each time a subscriber comes, after its SUBSCRIBE_OK: the publishers of
 * the namespace that are not a source of the track are asked, and one that
 * accepts is one more source." "Not asked are a session that is a source of
 * the track, one that is asked for it and has not answered, and one that was
 * asked and did not become a source: it refused [...] A publisher is asked
 * again once it announces a namespace of the track anew". "When a
 * PUBLISH_NAMESPACE arrives, the relay answers it and then does the same for
 * every track in that namespace that has a subscriber".
 */
async function askAll(run: Run): Promise<ScenarioResult> {
	const way = ASK_WAYS.find((value) => value === options.ask);
	if (way === undefined) {
		throw new Error(
			`--ask "${options.ask}" is not one of ${ASK_WAYS.join(", ")}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((kind) => !isOwnClient(kind));
	if (passing !== undefined || subscriberKinds.length < 2) {
		throw new Error(
			"--scenario ask-all needs two or more own subscribers: they stay for more than one Object, and the second shows what a later subscriber makes the relay ask",
		);
	}
	const relay = run.context.relay;
	// A publisher that only announces: it serves the track when it is asked.
	const announcer = (
		name: string,
		firstGroup: number,
		kind: Client = "own-rust",
	): Promise<Publisher> => {
		if (!isOwnClient(kind)) {
			throw new Error(
				`--scenario ask-all needs own clients as its announcers, which serve a track when they are asked, not ${kind}`,
			);
		}
		const label = `${name}:${kind}`;
		return joinAsPublisher(kind, run, {
			label,
			firstGroup,
			payloadHex: hexOf(`${label} ${run.payloadHex}`),
			servesWhenAsked: "at-once",
		});
	};
	// The publisher with PUBLISH, of the ways that have one.
	const withPublish = (): Promise<Publisher> => {
		const kind = onePublisherKind();
		return joinAsPublisher(kind, run, {
			label: `pub:${kind}`,
			payloadHex: hexOf(`publish ${run.payloadHex}`),
		});
	};
	// The publisher serves the track, and the relay's line for the SUBSCRIBE
	// it sent has been read.
	//
	// WHY the relay's line is waited for as well: the publisher's `SERVING`
	// says the SUBSCRIBE came, but the two processes' lines reach the driver
	// in no order, and a count of the relay's lines taken on `SERVING` alone
	// can miss the one that is on its way (seen in CI). The relay prints the
	// line when it sends the SUBSCRIBE, so it is certain to come.
	const asked = async (publisher: Publisher): Promise<void> => {
		await publisher.serving();
		await relayPrinted(requestOpenedTowards(sessionOf(publisher, run)), 1, run);
	};
	const join = (index: number): Promise<Subscriber> => {
		const kind = subscriberKinds[index];
		if (kind === undefined) throw new Error(`no subscriber ${index + 1}`);
		return joinAsSubscriber(kind, `sub${index + 1}:${kind}`, run);
	};
	const joinRest = async (from: number): Promise<Subscriber[]> => {
		const rest: Subscriber[] = [];
		for (let index = from; index < subscriberKinds.length; index++) {
			rest.push(await join(index));
		}
		return rest;
	};

	// The relay's line for a client that came: what it had done before, it
	// had done without that client.
	const cameAt = (client: Publisher | Subscriber): Fence => ({
		line: sessionOpened(sessionOf(client, run)),
	});
	// The relay's line for a Group it sends on to the subscriber that came
	// last: every subscriber had come by then, and been dealt with.
	const servedAt = (all: Subscriber[], group: number): Fence => {
		const last = all.at(-1);
		if (last === undefined) throw new Error("the scenario has no subscriber");
		return { line: streamOpened(sessionOf(last, run), group) };
	};

	if (way === "refusing-announcer") {
		// The page announced before the publisher came, so it is asked first
		// where it is asked at all.
		const refusing = await startRefusingAnnouncer("announcer:own-browser", run);
		const refusingSession = await sessionOfNewClient(run);
		const page = { who: refusing.label, session: refusingSession };
		const publisher = await withPublish();
		const first = await join(0);
		await expectAsks(
			[{ ...page, asks: 0 }],
			run,
			"before a subscriber came",
			cameAt(first),
		);
		// "after its SUBSCRIBE_OK": the first subscriber is answered by the
		// source there is, and the page is asked for it.
		await relayPrinted(requestOpenedTowards(refusingSession), 1, run);
		// The relay has dealt with the refusal when it ended its request.
		await relayPrinted(ownRequestEnded(refusingSession), 1, run);
		const rest = await joinRest(1);
		const group = await publisher.send();
		await allReceive([first, ...rest], group, publisher);
		// "one that was asked and did not become a source: it refused" is not
		// asked for the subscribers that came after.
		await expectAsks(
			[{ ...page, asks: 1 }],
			run,
			"when every subscriber had come",
			servedAt([first, ...rest], group),
		);
		// "A publisher is asked again once it announces a namespace of the
		// track anew".
		await refusing.announce();
		await relayPrinted(requestOpenedTowards(refusingSession), 2, run);
		// The relay has dealt with the second refusal when it ended its
		// second request.
		const refusedAgain: Fence = {
			line: ownRequestEnded(refusingSession),
			nth: 2,
		};
		await expectAsks(
			[{ ...page, asks: 2 }],
			run,
			"after it announced anew",
			refusedAgain,
		);
		await expectStillSubscribed(
			[first, ...rest],
			run,
			"after a publisher refused",
			refusedAgain,
		);

		// The end of the publisher's session comes after every line of what
		// went before.
		await expectAsks(
			[{ ...page, asks: 2 }],
			run,
			"when the publisher had left",
			await leaveInOrder(publisher, run),
		);
		const ended = await outcomes([first, ...rest]);
		for (const { subscriber, outcome } of ended) {
			expectPublishDone(subscriber, outcome, {
				statusCode: TRACK_ENDED,
				groups: [group],
				abortedStreams: 0,
			});
		}
		return {
			summary: `the relay asked ${refusing.label} once for two subscribers after it refused, and again when it announced anew; the subscribers got Group ${group} from ${publisher.label}`,
			subscriptions: subscriptionsOf(ended),
			measured: {},
		};
	}

	let first: Publisher;
	let second: Publisher;
	let subscribers: Subscriber[];
	// What the relay has asked in all when the run is over.
	let asksInAll: { who: string; session: number; asks: number }[];
	// "They are asked in the order they announced": the sessions of every
	// OPEN_REQUEST line, where the way has more than one.
	let askedInOrder: number[] | undefined;
	if (way === "two-announcers") {
		// --publisher twice names the two; own-rust both where it is left out.
		const [firstKind, secondKind] =
			options.publisher.length === 0
				? (["own-rust", "own-rust"] satisfies Client[])
				: twoPublisherKinds();
		first = await announcer("announcer1", 0, firstKind);
		second = await announcer("announcer2", 1, secondKind);
		const sessions = [first, second].map((each) => ({
			who: each.label,
			session: sessionOf(each, run),
		}));
		// The first subscriber is answered by whichever answers first.
		subscribers = [await join(0)];
		await expectAsks(
			sessions.map((each) => ({ ...each, asks: 0 })),
			run,
			"before a subscriber came",
			cameAt(subscribers[0] ?? first),
		);
		await asked(first);
		await asked(second);
		askedInOrder = sessions.map(({ session }) => session);
		// Both are sources now: a subscriber that comes makes nobody be asked.
		subscribers.push(...(await joinRest(1)));
		asksInAll = sessions.map((each) => ({ ...each, asks: 1 }));
	} else {
		first = await withPublish();
		const publishing = { who: first.label, session: sessionOf(first, run) };
		if (way === "announcer-and-publisher") {
			second = await announcer("announcer", 1);
			const announced = { who: second.label, session: sessionOf(second, run) };
			subscribers = [await join(0)];
			await expectAsks(
				[{ ...announced, asks: 0 }],
				run,
				"before a subscriber came",
				cameAt(subscribers[0] ?? second),
			);
			await asked(second);
			subscribers.push(...(await joinRest(1)));
			asksInAll = [
				{ ...announced, asks: 1 },
				{ ...publishing, asks: 0 },
			];
		} else {
			subscribers = await joinRest(0);
			second = await announcer("announcer", 1);
			await expectAsks(
				[{ ...publishing, asks: 0 }],
				run,
				"before the announcer came",
				cameAt(second),
			);
			const announced = { who: second.label, session: sessionOf(second, run) };
			// "the relay answers it and then does the same": the SUBSCRIBE
			// comes without a subscriber coming.
			await asked(second);
			asksInAll = [
				{ ...announced, asks: 1 },
				{ ...publishing, asks: 0 },
			];
		}
	}

	const group0 = await first.send();
	await allReceive(subscribers, group0, first);
	// Every subscriber had come, and the announcer where it came last, when
	// the relay sent this Group on.
	const served = servedAt(subscribers, group0);
	await expectAsks(
		asksInAll,
		run,
		"when every client had come and a Group was sent on",
		served,
	);
	if (askedInOrder !== undefined) {
		const order = (
			await relay.linesBefore(served, /^OPEN_REQUEST session=(\d+) /)
		).map((match) => Number(captureGroup(match, 1)));
		if (order.join(",") !== askedInOrder.join(",")) {
			throw new Error(
				`the relay asked the sessions ${order.join(", ")}, not ${askedInOrder.join(", ")} in the order they announced`,
			);
		}
	}
	const group1 = await second.send();
	await allReceive(subscribers, group1, second);

	await expectStillSubscribed(
		subscribers,
		run,
		`when ${first.label} had left`,
		await leaveInOrder(first, run),
	);
	// The end of the last publisher's session comes after every line of
	// what went before.
	await expectAsks(
		asksInAll,
		run,
		"when both publishers had left",
		await leaveInOrder(second, run),
	);
	const ended = await outcomes(subscribers);
	await expectOneObjectEach(subscribers, [group0], first);
	await expectOneObjectEach(subscribers, [group1], second);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups: [group0, group1],
			abortedStreams: 0,
		});
	}
	await expectPublishDoneLines(
		subscribers,
		run,
		await relayServesTheTrackAgain(run),
	);

	return {
		summary: `the relay asked ${second.label}${way === "two-announcers" ? ` and ${first.label}` : ""} once each, and the subscribers got Group ${group0} from ${first.label} and Group ${group1} from ${second.label}`,
		subscriptions: subscriptionsOf(ended),
		measured: {},
	};
}

/**
 * How long the relay waits for the streams a PUBLISH_DONE counted before it
 * gives the source up (`drain_timeout` in crates/relay/src/hub.rs,
 * provisional). The bounds below are this value with room on either side.
 */
const RELAY_DRAIN_TIMEOUT_MS = 2_000;

/**
 * A publisher says PUBLISH_DONE and publishes its track anew on the same
 * session, while the relay still waits for streams the first subscription
 * counted.
 *
 * The notes, "PUBLISH for a track the relay knows already": "Once the session
 * has said PUBLISH_DONE, its PUBLISH for the track is accepted, also while
 * the relay still waits for the streams the first subscription counted.
 * Under another Track Alias the two are two sources of the track, told apart
 * by the alias: the first drains as it would have [...] Under the same Track
 * Alias the alias is the new subscription's and the first one is over at
 * once: the relay stops waiting for what the first counted and resets its
 * PUBLISH stream (`CANCELLED`)". And: "If a counted stream is still under
 * way, the PUBLISH is accepted for the track as it is, and the subscribers
 * stay."
 */
async function publishAnew(run: Run): Promise<ScenarioResult> {
	const kind = onePublisherKind();
	if (kind !== "own-rust") {
		throw new Error(
			`--scenario publish-anew needs own-rust as its publisher, the one client that publishes a track anew, not ${kind}`,
		);
	}
	const subscriberKinds = options.subscriber.map((value) =>
		parseClient("subscriber", value),
	);
	const passing = subscriberKinds.find((each) => !isOwnClient(each));
	if (passing !== undefined) {
		throw new Error(
			`--scenario publish-anew needs own subscribers, which stay for more than one Object, not ${passing}`,
		);
	}
	const sameAlias = options["same-alias"];
	const publisher = await joinAsPublisher(kind, run);
	const subscribers = await joinAsSubscribers(subscriberKinds, "sub", run);

	const first = await publisher.send();
	await allReceive(subscribers, first, run);

	// The client's first PUBLISH has Track Alias 1 (crates/moqt-client).
	const firstAlias = 1;
	// WHY the relay's line is waited for between the PUBLISH_DONE and the
	// PUBLISH: a relay that reads the PUBLISH first refuses it, as §5.1 has
	// it do (`publishAnew` in clients.ts). `DRAIN_TIMER` is printed when the
	// relay has read the PUBLISH_DONE and waits for the streams it counted,
	// which is the state this scenario is about.
	//
	// What is left to time: the relay waits 2 seconds, and the PUBLISH has
	// to be read within them. One that comes later finds the track over; the
	// run fails then, and says that this is what happened.
	const anew = await publisher.publishAnew(sameAlias, () =>
		relayPrinted(
			new RegExp(
				`^DRAIN_TIMER session=${sessionOf(publisher, run)} track_alias=${firstAlias}$`,
			),
			1,
			run,
		),
	);
	const expectedAlias = sameAlias ? firstAlias : firstAlias + 1;
	if (anew.trackAlias !== expectedAlias) {
		throw new Error(
			`${publisher.label} published anew under Track Alias ${anew.trackAlias}, not ${expectedAlias}`,
		);
	}
	// "The subscribers stay": the track did not end between the two.
	//
	// WHY a message of its own: this is where a run ends whose PUBLISH came
	// after the relay's wait, and that is a matter of time and not of what
	// the relay does with a PUBLISH that comes in time. A relay that does not
	// wait at all ends here too; the time tells the two apart.
	//
	// The fence: the relay's answer to the new PUBLISH, the first REQUEST_OK
	// it sends the publisher after its DRAIN_TIMER line. That none comes
	// later is seen below, before the Group that is sent next.
	const publisherSession = sessionOf(publisher, run);
	const requestOk = new RegExp(
		`^SEND session=${publisherSession} stream=\\d+ REQUEST_OK`,
	);
	const okBefore = await run.context.relay.linesBefore(
		{
			line: new RegExp(
				`^DRAIN_TIMER session=${publisherSession} track_alias=${firstAlias}$`,
			),
		},
		requestOk,
	);
	const answered: Fence = { line: requestOk, nth: okBefore.length + 1 };
	const gone: typeof subscribers = [];
	for (const subscriber of subscribers) {
		const done = await run.context.relay.linesBefore(
			answered,
			publishDoneSent(sessionOf(subscriber, run)),
		);
		if (done.length > 0) gone.push(subscriber);
	}
	if (gone.length > 0) {
		throw new Error(
			`the track was over when the relay read the new PUBLISH of ${publisher.label}: it had sent PUBLISH_DONE to ${gone.map(({ label }) => label).join(", ")}. The PUBLISH was established ${seconds(anew.publishedAfterMs)} after this driver read the relay's DRAIN_TIMER line, and the relay waits ${seconds(RELAY_DRAIN_TIMEOUT_MS)} from that line. The run did not reach the state it is about`,
		);
	}
	// The first PUBLISH stream cannot get a FIN: the streams its PUBLISH_DONE
	// counted never all end. Under the same alias the relay resets it at
	// once; under another the first source drains until the relay's wait for
	// it runs out.
	if (!anew.peerEnd.startsWith("reset")) {
		throw new Error(
			`${publisher.label}: the relay ended the first PUBLISH stream by "${anew.peerEnd}", not by a reset`,
		);
	}
	if (sameAlias && anew.peerEnd !== `reset, code ${RESET_BY_RELAY}`) {
		throw new Error(
			`${publisher.label}: the relay reset the first PUBLISH stream with "${anew.peerEnd}", not with CANCELLED (${RESET_BY_RELAY})`,
		);
	}
	// WHY the relay's own lines, and not the time this driver read them at:
	// the relay's wait runs by its clock, and the driver reads a line later
	// by as much as it is busy. The relay writes one stream of lines, so
	// their order is the order it acted in. Under the same alias it resets
	// the first PUBLISH stream before its wait runs out (the wait is not
	// stopped, and runs out all the same); under another it resets the
	// stream when the wait has run out, after the line that says so.
	const drainEnded = new RegExp(
		`^DRAIN_ENDED session=${publisherSession} after_ms=(\\d+) by=(\\w+)$`,
	);
	const [drainLine, ranMs = "", by = ""] =
		await run.context.relay.waitForLine(drainEnded);
	const ran = Number(ranMs);
	const resetBefore = await run.context.relay.linesBefore(
		{ line: drainEnded },
		new RegExp(`^RESET_REQUEST session=${publisherSession} `),
	);
	const atOnce = resetBefore.length > 0;
	const ranOut = by === "timeout" && ran >= RELAY_DRAIN_TIMEOUT_MS;
	if (sameAlias ? !atOnce : atOnce || !ranOut) {
		throw new Error(
			`${publisher.label}: ${sameAlias ? "under the same alias the relay was to reset the first PUBLISH stream before its wait for the first source ran out" : `under another alias the first source was to drain for ${seconds(RELAY_DRAIN_TIMEOUT_MS)}, and its stream to be reset after that`}: ${drainLine}, ${resetBefore.length} resets of the session before it`,
		);
	}

	const second = await publisher.send();
	await allReceive(subscribers, second, run);
	const [anySubscriber] = subscribers;
	if (anySubscriber === undefined) {
		throw new Error("the scenario has no subscriber");
	}
	await expectStillSubscribed(
		subscribers,
		run,
		"when the relay sent on the Group of the new PUBLISH",
		{ line: streamOpened(sessionOf(anySubscriber, run), second) },
	);

	await expectOwnPublishDone(publisher, await publisher.end());
	const ended = await outcomes(subscribers);
	await expectOneObjectEach(subscribers, [first, second], publisher);
	for (const { subscriber, outcome } of ended) {
		expectPublishDone(subscriber, outcome, {
			statusCode: TRACK_ENDED,
			groups: [first, second],
			abortedStreams: 0,
		});
	}
	await expectPublishDoneLines(
		subscribers,
		run,
		await relayServesTheTrackAgain(run),
	);

	return {
		summary: `the subscribers stayed through a new PUBLISH under ${sameAlias ? "the same" : "another"} Track Alias and got Group ${first} before it and Group ${second} after; the relay ended the first PUBLISH stream by "${anew.peerEnd}" after ${seconds(anew.peerEndAfterMs)}, and its wait for the first source ended by ${by} after ${ran} ms`,
		subscriptions: subscriptionsOf(ended),
		measured: {
			"ms from the relay reading PUBLISH_DONE to the new PUBLISH":
				anew.publishedAfterMs,
			"ms until the first PUBLISH stream ended": anew.peerEndAfterMs,
			"ms the relay waited for the first source": ran,
		},
	};
}

const RUNNERS: Record<Scenario, (run: Run) => Promise<ScenarioResult>> = {
	"publisher-killed": publisherKilled,
	"last-object-then-close": lastObjectThenClose,
	"subscriber-leaves": subscriberLeaves,
	"slow-subscriber": slowSubscriber,
	"same-group-twice": sameGroupTwice,
	"second-source": secondSource,
	"publish-while-asked": publishWhileAsked,
	"publish-anew": publishAnew,
	"ask-all": askAll,
	"subgroup-priorities": subgroupPriorities,
	"end-of-track": endOfTrack,
	"subgroup-again": subgroupAgain,
};

async function main(): Promise<void> {
	const scenario = options.scenario;
	if (!isScenario(scenario)) {
		throw new Error(
			`--scenario "${scenario}" is not one of ${SCENARIOS.join(", ")}`,
		);
	}
	if (scenario !== "same-group-twice" && scenario !== "second-source") {
		// `publish-while-asked` has two publishers and takes none of these.
		refuseOptions(scenario, [
			"first",
			"first-sends",
			"second-resends",
			"second-joins",
		]);
	}
	const subscribers = [
		...options.subscriber,
		...options.stayer,
		...options.leaver.map((value) => parseLeaver(value).kind),
	].map((value) => parseClient("subscriber", value));
	const publishers = publisherKinds();

	let result: ScenarioResult;
	let relayVersion: string;
	let browserVersion: string | null;
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: [
				...publishers,
				...subscribers,
				...(options.slow === "unread-own-browser"
					? ["own-browser" as const]
					: []),
				// The run that follows every scenario uses the own Rust client.
				"own-rust",
			],
			verbose: options.verbose,
			...(options.slow === "stopped-own-rust" && scenario === "slow-subscriber"
				? {
						relayArgs: [
							"--subscription-send-streams",
							String(STOPPED_SUBSCRIPTION_STREAMS),
							"--idle-timeout-ms",
							String(STOPPED_IDLE_TIMEOUT_MS),
						],
					}
				: {}),
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
		// WHY one name where there is one publisher: the records and whoever
		// reads this line have `publisher` as a client's name.
		`RESULT ${JSON.stringify({ scenario, publisher: publishers.length === 1 ? publishers[0] : publishers, ...result, relayVersion, browserVersion })}`,
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
