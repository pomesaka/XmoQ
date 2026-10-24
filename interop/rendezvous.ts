// A SUBSCRIBE that comes before its publisher: with RENDEZVOUS_TIMEOUT the
// relay holds it (§10.2.6 of draft-ietf-moq-transport-18).
//
// Usage: bun rendezvous.ts --case <case> --subscriber own-browser|own-rust [--relay own] [--port N]
//
// Through the own relay. (The row "A SUBSCRIBE for a track nobody
// publishes" of docs/moqt-implementation-notes.md.)
//
//   With --through-edge (publisher-comes alone): the subscriber connects to
//   a second own relay, on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET,
//   whose upstream is the relay of the run (`--upstream`); the publisher
//   stays with the relay of the run. The relay below holds the SUBSCRIBE and
//   asks the one above with a RENDEZVOUS_TIMEOUT of its own, which the one
//   above holds as the lines say (its session 0 is the relay below), and the
//   rest is as without it.
//
//   publisher-comes   The subscriber sends its SUBSCRIBE with a
//                     RENDEZVOUS_TIMEOUT of 20 seconds while nobody
//                     publishes the track. The relay holds it
//                     (`HOLD_REQUEST session=0 stream=S SUBSCRIBE`) and
//                     answers nothing. Two seconds later an own-rust
//                     publisher sends PUBLISH for the track and one Object:
//                     the relay answers SUBSCRIBE_OK, and the subscriber has
//                     the Object.
//   nobody-comes      The same SUBSCRIBE with 1.5 seconds, and no publisher.
//                     The relay answers REQUEST_ERROR with TIMEOUT (0x2)
//                     between 1.5 and 4 seconds after the SUBSCRIBE, and
//                     not before, with a Retry Interval of 1 (ask again at
//                     once), which the page's library hands on as a wait
//                     of 0 ms. The session is not closed over it.
//   another-publisher-comes
//                     As publisher-returns, but the first publisher has
//                     sent three Objects (Groups 0, 1 and 2) before its
//                     PUBLISH_DONE, and the PUBLISH that follows, half a
//                     second later and so while the relay still waits for
//                     late streams, is of another own-rust process, which
//                     numbers its Groups from the time (`--first-group
//                     now`; transport §2.4 has a publisher that resumes
//                     "set the initial Group ID to a unique value
//                     guaranteed to be larger than all previously used
//                     groups"). The held subscriber gets SUBSCRIBE_OK and
//                     the first Object the new publisher sends. (A
//                     publisher that began at Group 0 again would have its
//                     first three Groups left out for that subscriber.)
//   page-publisher-returns
//                     An own page publishes a broadcast (catalog, video,
//                     sound) and another plays its video as the demo's
//                     `/watch?wait=1` does: on one session, with a
//                     RENDEZVOUS_TIMEOUT of 20 seconds on the SUBSCRIBE for
//                     the catalog. The publishing page ends its tracks with
//                     PUBLISH_DONE and stays connected. The viewer is told
//                     that what it played ended, asks again on its session
//                     a second later, and is held: it is never told "not
//                     yet", and the relay sends no DOES_NOT_EXIST while no
//                     page publishes. Three
//                     seconds after the end a third page publishes the
//                     broadcast anew, and the viewer draws 30 frames more,
//                     on the session it had.
//   viewer-before-the-broadcast
//                     The terminal viewer (`xmoq-watch
//                     --rendezvous-timeout-ms 20000`) comes first. The
//                     relay holds its SUBSCRIBE for the catalog and answers
//                     it nothing. Two seconds later an own page begins to
//                     publish a catalog, video and sound: the viewer finds
//                     the tracks and plays them (100 frames or more drawn,
//                     100 packets of sound or more) and ends of itself.
//   not-asked         The SUBSCRIBE without the parameter: DOES_NOT_EXIST
//                     (0x10) at once, as before, with a Retry Interval of
//                     1,001 (ask again after a second).
//   publisher-returns An own-rust publisher ends its track with
//                     PUBLISH_DONE and a Stream Count it does not know, so
//                     that the relay waits two seconds for late streams.
//                     In that time the subscriber sends its SUBSCRIBE with
//                     20 seconds. The relay holds it and answers nothing,
//                     neither while it waits for the streams nor when the
//                     track is over. Four seconds after the PUBLISH_DONE
//                     the publisher sends PUBLISH anew and one Object: the
//                     relay answers SUBSCRIBE_OK, and the subscriber has
//                     the Object.

import { parseArgs } from "node:util";
import {
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	NAMESPACE,
	openConnectedPage,
	type RunContext,
	runCleanups,
	spawnRustClient,
	spawnTerminalViewer,
	startRun,
	startSecondOwnRelay,
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
		case: { type: "string", default: "" },
		subscriber: { type: "string", default: "own-rust" },
		port: { type: "string", default: "24398" },
		relay: { type: "string", default: "own" },
		"through-edge": { type: "boolean", default: false },
		verbose: { type: "boolean", default: false },
	},
});

/**
 * The relay below the run's with `--through-edge`, where the subscriber
 * connects. Set once by `main`, before a case runs.
 */
let edgeUrl: string | undefined;

stopEverythingOnExit();

const PAYLOAD_HEX = "72656e64657a766f7573";
const wait = (ms: number): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, ms));

/** A subscriber that has sent its SUBSCRIBE: how that ended, and the payload of the Object it got. */
interface Asked {
	/** "ok", or what the subscriber said of the refusal. */
	answer: Promise<string>;
	object(): Promise<string>;
}

async function subscribe(
	context: RunContext,
	subscriber: string,
	rendezvousMs: number | undefined,
): Promise<Asked> {
	if (edgeUrl !== undefined) {
		context = { ...context, relayUrl: edgeUrl };
	}
	if (subscriber === "own-browser") {
		const page = await openConnectedPage(
			context,
			"sub:own-browser",
			"own-browser",
		);
		const answer = page
			.evaluate(
				([namespace, track, rendezvous]) =>
					window.xmoq.subscribe(namespace, track, rendezvous),
				[NAMESPACE, TRACK, rendezvousMs] satisfies [
					string,
					string,
					number | undefined,
				],
			)
			.then(
				() => "ok",
				(error: unknown) => String(error),
			);
		return {
			answer,
			object: async () =>
				(await page.evaluate(() => window.xmoq.firstObject())).payloadHex,
		};
	}
	if (subscriber !== "own-rust") {
		throw new Error(
			`--subscriber is own-browser or own-rust, not ${subscriber}`,
		);
	}
	const rust = spawnRustClient(
		context,
		"sub:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		[
			...(rendezvousMs === undefined
				? []
				: ["--rendezvous-timeout-ms", String(rendezvousMs)]),
			"--timeout-secs",
			"40",
		],
	);
	const answer = Promise.race([
		rust.waitForLine(/^SUBSCRIBED /, 40_000).then(() => "ok"),
		rust.exited.then(
			() =>
				rust.matches(/^.*(?:rejected|refused|error_code).*$/i)[0]?.[0] ??
				"it left",
		),
	]);
	return {
		answer,
		object: async () => {
			const [, payload] = await rust.waitForLine(
				/^OBJECT .*payload_hex=([0-9a-f]*)$/,
			);
			return payload ?? "";
		},
	};
}

/** The second session of the run is the subscriber's where a publisher is there first; here the subscriber is the first. */
const HELD = /^HOLD_REQUEST session=0 stream=(\d+) SUBSCRIBE$/;
const answered = (stream: string, session = 0): RegExp =>
	new RegExp(
		`^SEND session=${session} stream=${stream} (SUBSCRIBE_OK|REQUEST_ERROR.*)$`,
	);

async function publisherComes(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const asked = await subscribe(context, subscriber, 20_000);
	const [, stream = ""] = await context.relay.waitForLine(HELD);
	await wait(2000);
	if (context.relay.hasPrinted(answered(stream))) {
		throw new Error(
			"the relay was to answer nothing while nobody published the track",
		);
	}
	const publisher = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish",
		TRACK,
		["--payload-hex", PAYLOAD_HEX, "--timeout-secs", "40"],
	);
	await publisher.waitForLine(/^PUBLISHED /);
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be answered",
	);
	if (answer !== "ok") {
		throw new Error(
			`the held SUBSCRIBE was to be accepted when the publisher came: ${answer}`,
		);
	}
	await context.relay.waitForLine(
		new RegExp(`^SEND session=0 stream=${stream} SUBSCRIBE_OK`),
	);
	publisher.child.stdin.write("send\n");
	const payload = await withTimeout(
		asked.object(),
		"the Object of the publisher that came later",
	);
	if (payload !== PAYLOAD_HEX) {
		throw new Error(
			`the subscriber was to get the publisher's Object, not ${payload}`,
		);
	}
	return `${subscriber} subscribed 2 s before anybody published the track, with a RENDEZVOUS_TIMEOUT of 20 s: the relay held the SUBSCRIBE, answered SUBSCRIBE_OK when own-rust sent its PUBLISH, and the subscriber got the Object`;
}

async function publisherReturns(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const publisher = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish",
		TRACK,
		["--payload-hex", PAYLOAD_HEX, "--timeout-secs", "40"],
	);
	await publisher.waitForLine(/^PUBLISHED /);
	publisher.child.stdin.write("publish-done\n");
	await publisher.waitForLine(/^PUBLISH_DONE_SENT$/);
	const ended = Date.now();
	// The publisher's is the first session of the run.
	const asked = await subscribe(context, subscriber, 20_000);
	const [, stream = ""] = await context.relay.waitForLine(
		/^HOLD_REQUEST session=1 stream=(\d+) SUBSCRIBE$/,
	);
	const heldAfter = Date.now() - ended;
	if (heldAfter >= 2000) {
		throw new Error(
			`the SUBSCRIBE was to reach the relay while it waited for the track's late streams, not ${heldAfter} ms after the PUBLISH_DONE`,
		);
	}
	await wait(4000 - heldAfter);
	if (context.relay.hasPrinted(answered(stream, 1))) {
		throw new Error(
			"the relay was to answer nothing while the track's publisher was away",
		);
	}
	publisher.child.stdin.write("publish-anew\n");
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be answered",
	);
	if (answer !== "ok") {
		throw new Error(
			`the held SUBSCRIBE was to be accepted when the publisher returned: ${answer}`,
		);
	}
	publisher.child.stdin.write("send\n");
	const payload = await withTimeout(
		asked.object(),
		"the Object of the publisher that returned",
	);
	if (payload !== PAYLOAD_HEX) {
		throw new Error(
			`the subscriber was to get the publisher's Object, not ${payload}`,
		);
	}
	return `${subscriber} subscribed ${heldAfter} ms after own-rust ended the track with PUBLISH_DONE, with a RENDEZVOUS_TIMEOUT of 20 s: the relay held the SUBSCRIBE through its wait for late streams and after the track was over, answered SUBSCRIBE_OK when own-rust sent PUBLISH anew 4 s after the PUBLISH_DONE, and the subscriber got the Object`;
}

async function nobodyComes(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const began = Date.now();
	const asked = await subscribe(context, subscriber, 1500);
	const [, stream = ""] = await context.relay.waitForLine(HELD);
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be refused",
	);
	const after = Date.now() - began;
	const [, sent = ""] = await context.relay.waitForLine(answered(stream));
	if (!/^REQUEST_ERROR error_code=0x2\b/.test(sent)) {
		throw new Error(
			`the relay was to answer REQUEST_ERROR with TIMEOUT (0x2), not ${sent}`,
		);
	}
	// TIMEOUT is to be asked again at once: a Retry Interval of 1, which the
	// page's library hands on as a wait of 0 ms.
	const atOnce =
		subscriber === "own-browser"
			? /\(retry: after 0 ms\)/
			: /retry_interval=1\b/;
	if (!/^REQUEST_ERROR error_code=0x2\b/.test(sent) || !atOnce.test(answer)) {
		throw new Error(
			`the subscriber was to be told that it may ask again at once: ${answer}`,
		);
	}
	if (answer === "ok" || after < 1500 || after > 5000) {
		throw new Error(
			`the subscriber was to be refused between 1.5 and 5 s, not "${answer}" after ${after} ms`,
		);
	}
	if (context.relay.hasPrinted(/^CLOSE_SESSION /)) {
		throw new Error("the relay was to close no session over it");
	}
	return `${subscriber} subscribed to a track nobody publishes with a RENDEZVOUS_TIMEOUT of 1.5 s: the relay held the SUBSCRIBE and refused it with TIMEOUT (0x2) ${after} ms after the subscriber began: ${answer}`;
}

async function notAsked(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const asked = await subscribe(context, subscriber, undefined);
	const answer = await withTimeout(asked.answer, "the SUBSCRIBE to be refused");
	const [, sent = ""] = await context.relay.waitForLine(
		/^SEND session=0 stream=\d+ (SUBSCRIBE_OK|REQUEST_ERROR.*)$/,
	);
	if (!/^REQUEST_ERROR error_code=0x10\b/.test(sent) || answer === "ok") {
		throw new Error(
			`the relay was to answer DOES_NOT_EXIST (0x10) at once, not ${sent}`,
		);
	}
	if (context.relay.hasPrinted(/^HOLD_REQUEST /)) {
		throw new Error("the relay was to hold nothing");
	}
	// To be asked again after a second: a Retry Interval of 1,001.
	if (!/retry_interval=1001\b/.test(answer)) {
		throw new Error(
			`the subscriber was to be told to ask again after a second: ${answer}`,
		);
	}
	return `${subscriber} subscribed to a track nobody publishes without RENDEZVOUS_TIMEOUT, and was refused with DOES_NOT_EXIST (0x10) at once: ${answer}`;
}

/**
 * The terminal viewer comes before the broadcast: it is held on the
 * SUBSCRIBE for the catalog, and plays when a page begins to publish.
 */
async function viewerBeforeTheBroadcast(context: RunContext): Promise<string> {
	const namespace = "xmoq/rendezvous";
	const seconds = 14;
	const viewer = spawnTerminalViewer(context, "viewer:xmoq-watch", [
		"--namespace",
		namespace,
		"--rendezvous-timeout-ms",
		"20000",
		"--seconds",
		String(seconds),
		"--sound",
		"off",
		"--out",
		"/dev/null",
		"--medium",
		"direct",
		"--fit",
		"none",
	]);
	await context.relay.waitForLine(HELD);
	await wait(2000);
	if (context.relay.hasPrinted(/^SEND session=0 stream=\d+ /)) {
		throw new Error(
			"the relay was to answer the viewer nothing before the broadcast",
		);
	}
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(([name]) => window.xmoqOwn.publishBroadcast(name), [
			namespace,
		] satisfies [string]),
		"the page to publish its broadcast",
	);
	const [line] = await viewer.waitForLine(/^AV_PLAYED .*$/, 40_000);
	const field = (name: string): number =>
		Number(new RegExp(` ${name}=([0-9.]+)`).exec(line)?.[1] ?? Number.NaN);
	const drawn = field("video_drawn");
	const packets = field("audio_packets");
	if (!(drawn >= 100) || !(packets >= 100)) {
		throw new Error(
			`the viewer was to play the broadcast that came after it: ${drawn} frames drawn, ${packets} packets of sound. ${line}`,
		);
	}
	return `xmoq-watch came 2 s before the broadcast, with a RENDEZVOUS_TIMEOUT of 20 s on its SUBSCRIBEs: the relay held the one for the catalog, and when a page began to publish the viewer found the tracks and played them (${drawn} frames drawn, ${packets} packets of sound)`;
}

async function anotherPublisherComes(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const first = spawnRustClient(
		context,
		"pub1:own-rust",
		"own-rust",
		"publish",
		TRACK,
		["--payload-hex", "01", "--timeout-secs", "40"],
	);
	await first.waitForLine(/^PUBLISHED /);
	// Groups 0, 1 and 2.
	for (let sent = 0; sent < 3; sent++) {
		first.child.stdin.write("send\n");
		await wait(100);
	}
	first.child.stdin.write("publish-done\n");
	await first.waitForLine(/^PUBLISH_DONE_SENT$/);
	const ended = Date.now();
	const asked = await subscribe(context, subscriber, 20_000);
	await context.relay.waitForLine(
		/^HOLD_REQUEST session=1 stream=(\d+) SUBSCRIBE$/,
	);
	const second = spawnRustClient(
		context,
		"pub2:own-rust",
		"own-rust",
		"publish",
		TRACK,
		[
			"--payload-hex",
			PAYLOAD_HEX,
			"--first-group",
			"now",
			"--timeout-secs",
			"40",
		],
	);
	await second.waitForLine(/^PUBLISHED /);
	const after = Date.now() - ended;
	if (after >= 2000) {
		throw new Error(
			`the second PUBLISH was to come while the relay waited for the track's late streams, not ${after} ms after the PUBLISH_DONE`,
		);
	}
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be answered",
	);
	if (answer !== "ok") {
		throw new Error(
			`the held SUBSCRIBE was to be accepted when another publisher came: ${answer}`,
		);
	}
	second.child.stdin.write("send\n");
	const payload = await withTimeout(
		asked.object(),
		"the first Object of the publisher that came",
	);
	if (payload !== PAYLOAD_HEX) {
		throw new Error(
			`the subscriber was to get the new publisher's first Object, not ${payload}`,
		);
	}
	return `${subscriber} subscribed after own-rust had sent Groups 0 to 2 and ended the track with PUBLISH_DONE, with a RENDEZVOUS_TIMEOUT of 20 s: the relay held the SUBSCRIBE, another own-rust process that numbers its Groups from the time sent PUBLISH ${after} ms after the PUBLISH_DONE, and the subscriber got SUBSCRIBE_OK and the first Object of the new publisher`;
}

/**
 * A page that publishes ends its broadcast with PUBLISH_DONE, and another
 * page publishes it anew: the viewer waits on its session.
 */
async function pagePublisherReturns(context: RunContext): Promise<string> {
	const namespace = "xmoq/returns";
	const first = await openConnectedPage(context, "publisher 1", "own-browser");
	await withTimeout(
		first.evaluate(([name]) => window.xmoqOwn.publishBroadcast(name), [
			namespace,
		] satisfies [string]),
		"the first page to publish its broadcast",
	);
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	await viewer.evaluate(
		([url, hash, name]) =>
			window.xmoqOwn.keepBroadcastPlayed(url, hash, name, 20_000),
		[context.relayUrl, context.certSha256Hex, namespace] satisfies [
			string,
			string,
			string,
		],
	);
	const kept = () => viewer.evaluate(() => window.xmoqOwn.keptReport());
	const until = async (
		what: string,
		reached: (report: Awaited<ReturnType<typeof kept>>) => boolean,
	): Promise<Awaited<ReturnType<typeof kept>>> => {
		const began = Date.now();
		for (;;) {
			const report = await kept();
			if (reached(report)) return report;
			if (Date.now() - began > 20_000) {
				throw new Error(
					`timed out waiting for ${what}: ${JSON.stringify(report.playing)}, ${report.shown.length} frames`,
				);
			}
			await wait(100);
		}
	};
	const before = await until(
		"the viewer to draw 30 frames of the first broadcast",
		(report) => report.shown.length >= 30,
	);
	// WHY counted between two lines of the relay: while a broadcast plays,
	// the relay may answer a Joining FETCH of the viewer with
	// DOES_NOT_EXIST (the page that publishes serves no FETCH for its video
	// track), which is not what this run is about. The first page's is the
	// relay's first session, and the page that publishes anew its fourth.
	const refusedBy = async (fence: RegExp): Promise<number> =>
		(
			await context.relay.linesBefore(
				{ line: fence },
				/^SEND session=\d+ stream=\d+ REQUEST_ERROR error_code=0x10\b/,
			)
		).length;
	await withTimeout(
		first.evaluate(() => window.xmoqOwn.endBroadcast()),
		"the first page to end its broadcast",
	);
	await until("the viewer to be told that what it played ended", (report) =>
		report.playing.includes("playing-ended"),
	);
	// It asks again a second after that, and is held.
	await wait(3000);
	const held = await kept();
	const second = await openConnectedPage(context, "publisher 2", "own-browser");
	await withTimeout(
		second.evaluate(([name]) => window.xmoqOwn.publishBroadcast(name), [
			namespace,
		] satisfies [string]),
		"the second page to publish the broadcast",
	);
	const after = await until(
		"the viewer to draw 30 frames of the second broadcast",
		(report) => report.shown.length >= held.shown.length + 30,
	);
	const notYet = after.playing.filter((state) => state.startsWith("not-yet"));
	if (notYet.length > 0) {
		throw new Error(
			`the viewer was to be held, and not told "not yet": ${JSON.stringify(after.playing)}`,
		);
	}
	const whenItEnded = await refusedBy(/^RECV session=0 own=\d+ PUBLISH_DONE /);
	const whenItWasBack = await refusedBy(
		/^OPEN_REQUEST session=3 own=\d+ SUBSCRIBE$/,
	);
	if (whenItEnded !== whenItWasBack) {
		throw new Error(
			`the relay was to send nobody DOES_NOT_EXIST while no page published: ${whenItWasBack - whenItEnded} sent`,
		);
	}
	const sessions = after.playing.filter((state) => /^playing \d+$/.test(state));
	if (sessions.join(",") !== "playing 1,playing 1") {
		throw new Error(
			`the viewer was to play twice on its first session: ${JSON.stringify(after.playing)}`,
		);
	}
	const drained = context.relay.hasPrinted(/^DRAIN_TIMER /);
	return `a page ended its broadcast with PUBLISH_DONE and stayed connected; the viewer, which waits as /watch?wait=1 does, was told that what it played ended (${before.shown.length} frames drawn by then), asked again on its session and was held, and drew ${after.shown.length - held.shown.length} frames more when another page published the broadcast 3 s later: one session, never told "not yet", no DOES_NOT_EXIST from the relay between the end and the new broadcast (the relay ${drained ? "waited" : "did not wait"} for late streams of the tracks that ended); its states: ${after.playing.join(", ")}`;
}

const CASES: Record<
	string,
	(context: RunContext, subscriber: string) => Promise<string>
> = {
	"publisher-comes": publisherComes,
	"nobody-comes": nobodyComes,
	"not-asked": notAsked,
	"publisher-returns": publisherReturns,
	"another-publisher-comes": anotherPublisherComes,
	"viewer-before-the-broadcast": (context) => viewerBeforeTheBroadcast(context),
	"page-publisher-returns": (context) => pagePublisherReturns(context),
};

async function main(): Promise<void> {
	const run = CASES[options.case];
	if (run === undefined) throw new Error(`unknown --case ${options.case}`);
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients:
				options.subscriber === "own-browser" ||
				options.case === "page-publisher-returns" ||
				options.case === "viewer-before-the-broadcast"
					? ["own-browser", "own-rust"]
					: ["own-rust"],
			verbose: options.verbose,
		});
		if (options["through-edge"]) {
			if (options.case !== "publisher-comes") {
				throw new Error(
					`--through-edge is for publisher-comes, not ${options.case}`,
				);
			}
			const edge = await startSecondOwnRelay(
				String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
				context,
			);
			edgeUrl = edge.url;
		}
		summary = await run(context, options.subscriber);
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ case: options.case, subscriber: options.subscriber })}`,
	);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED rendezvous ${options.case}: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
