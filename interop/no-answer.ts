// A request the peer does not answer: the own clients give it up after the
// time its answer gets, and their session stays open.
//
// Usage: bun no-answer.ts --subscriber own-browser|own-rust [--port N]
//
// Through the own relay. An own-rust publisher announces the namespace and
// holds back its answer to the SUBSCRIBE the relay sends it, for as long as
// the run lasts. The relay waits 5 seconds for a publisher and would then
// answer the subscriber with REQUEST_ERROR `TIMEOUT`; the subscriber is
// given 1.5 seconds for an answer here, so that it gives up first.
//
// WHY 1.5 seconds and not the 10 the clients have by themselves: with 10 the
// relay answers first, and nothing is left unanswered. The time is counted
// by the same code whatever it is.
//
// What a run has to show (the row "A request the peer does not answer" of
// docs/moqt-implementation-notes.md):
//   - The subscriber gives its SUBSCRIBE up between 1.5 and 4 seconds
//     after it sent it, and says that no answer came in time: the page's
//     `subscribe()` fails with `RequestAbortedError` of `no-answer-in-time`,
//     the Rust client prints `ANSWER_TIMED_OUT request=subscribe
//     after_ms=1500`.
//   - Its request stream is reset with CANCELLED (0x1): the relay prints
//     `RESET_RECEIVED` with `code=0x1` for a stream of that session.
//   - The relay did not answer the SUBSCRIBE, and closed no session.
//   - **The session stays open** (the page): its `closed` has not settled,
//     and a PUBLISH_NAMESPACE and a PUBLISH it sends afterwards on the same
//     session are answered. The Rust client is a program that runs one mode
//     and leaves: it closes its session itself with NO_ERROR (0x0), which
//     the relay prints.

import { parseArgs } from "node:util";
import {
	log,
	NAMESPACE,
	openConnectedPage,
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
		subscriber: { type: "string", default: "own-browser" },
		port: { type: "string", default: "24396" },
		// The matrix gives every driver a relay; this one is through the own.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const ANSWER_WAIT_MS = 1500;
/** Before the relay gives the publisher up itself, after 5 seconds. */
const GIVEN_UP_BEFORE_MS = 4000;

/** The second session the relay opened: the publisher's is the first. */
async function subscribersSession(context: RunContext): Promise<string> {
	const opened = await context.relay.waitForLines(
		/^SESSION_OPENED session=(\d+)$/,
		2,
	);
	const session = opened[1]?.[1];
	if (session === undefined) throw new Error("the relay opened no session");
	return session;
}

/** The relay's lines about the subscriber's SUBSCRIBE: reset with CANCELLED, and not answered. */
async function expectCancelledAtTheRelay(
	context: RunContext,
	session: string,
): Promise<void> {
	const gone = new RegExp(
		`^RESET_RECEIVED session=${session} stream=\\d+ code=0x1$`,
	);
	const unwanted = await context.relay.linesBefore(
		{ line: gone },
		new RegExp(
			`^(?:SEND session=${session} stream=\\d+ (?:SUBSCRIBE_OK|REQUEST_ERROR)|CLOSE_SESSION )`,
		),
	);
	if (unwanted.length > 0) {
		throw new Error(
			`the relay was to answer nothing and close no session before the SUBSCRIBE was given up: ${unwanted.map((match) => match[0]).join(" | ")}`,
		);
	}
}

async function withAPage(context: RunContext): Promise<string> {
	const page = await openConnectedPage(
		context,
		"sub:own-browser",
		"own-browser",
		undefined,
		ANSWER_WAIT_MS,
	);
	const session = await subscribersSession(context);
	const began = Date.now();
	const failed = await withTimeout(
		page
			.evaluate(
				([namespace, track]) => window.xmoq.subscribe(namespace, track),
				[NAMESPACE, TRACK] satisfies [string, string],
			)
			.then(
				(requestId) => `it was answered: request ${requestId}`,
				(error: unknown) => String(error),
			),
		"the page to give its SUBSCRIBE up",
	);
	const after = Date.now() - began;
	if (
		!failed.includes(
			"RequestAbortedError: the request ended without a response: no-answer-in-time",
		)
	) {
		throw new Error(
			`the page's subscribe() was to fail for no answer in time: ${failed}`,
		);
	}
	if (after < ANSWER_WAIT_MS || after > GIVEN_UP_BEFORE_MS) {
		throw new Error(
			`the page was to give up between ${ANSWER_WAIT_MS} and ${GIVEN_UP_BEFORE_MS} ms, not after ${after}`,
		);
	}
	await expectCancelledAtTheRelay(context, session);
	// The session goes on: it is open, and its next requests are answered.
	const state = await page.evaluate(() => window.xmoqOwn.state());
	if (!state.includes('"session":null')) {
		throw new Error(`the page's session was to stay open: ${state}`);
	}
	await withTimeout(
		page.evaluate(
			([namespace, track]) => window.xmoq.publishTrack(namespace, track),
			["xmoq/no-answer/after", TRACK] satisfies [string, string],
		),
		"the page's requests after the one it gave up to be answered",
	);
	return `own-browser gave its SUBSCRIBE up ${after} ms after it was sent (its wait: ${ANSWER_WAIT_MS} ms), the relay saw the request stream reset with CANCELLED, and a PUBLISH_NAMESPACE and a PUBLISH on the same session were answered afterwards`;
}

async function withARustClient(context: RunContext): Promise<string> {
	const began = Date.now();
	const client = spawnRustClient(
		context,
		"sub:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		["--answer-wait-ms", String(ANSWER_WAIT_MS)],
	);
	const session = await subscribersSession(context);
	await client.waitForLine(
		new RegExp(
			`^ANSWER_TIMED_OUT request=subscribe after_ms=${ANSWER_WAIT_MS}$`,
		),
	);
	const after = Date.now() - began;
	if (after < ANSWER_WAIT_MS || after > GIVEN_UP_BEFORE_MS) {
		throw new Error(
			`own-rust was to give up between ${ANSWER_WAIT_MS} and ${GIVEN_UP_BEFORE_MS} ms, not after ${after}`,
		);
	}
	await expectCancelledAtTheRelay(context, session);
	const code = await withTimeout(client.exited, "own-rust to leave");
	if (code === 0) throw new Error("own-rust was to fail, and left with 0");
	// It closes its session itself, with NO_ERROR: no protocol error ends it.
	await context.relay.waitForLine(
		new RegExp(
			`^CONNECTION_ENDED session=${session} ClosedByPeer \\{ code: 0,`,
		),
	);
	return `own-rust gave its SUBSCRIBE up ${after} ms after it started (its wait: ${ANSWER_WAIT_MS} ms), the relay saw the request stream reset with CANCELLED, and the client closed its session with NO_ERROR`;
}

async function main(): Promise<void> {
	const subscriber = options.subscriber;
	if (subscriber !== "own-browser" && subscriber !== "own-rust") {
		throw new Error(
			`--subscriber is own-browser or own-rust, not ${subscriber}`,
		);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients:
				subscriber === "own-browser"
					? ["own-browser", "own-rust"]
					: ["own-rust"],
			verbose: options.verbose,
		});
		// It announces the namespace and never answers what it is asked.
		const publisher = spawnRustClient(
			context,
			"pub:own-rust",
			"own-rust",
			"publish-namespace",
			TRACK,
			["--payload-hex", "00", "--serve-on-line", "--timeout-secs", "60"],
		);
		await publisher.waitForLine(/^NAMESPACE_PUBLISHED/);
		summary =
			subscriber === "own-browser"
				? await withAPage(context)
				: await withARustClient(context);
		// The relay asked the publisher, which is what was not answered.
		await publisher.waitForLine(/^ASKED request_id=\d+$/);
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ subscriber })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED no answer: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
