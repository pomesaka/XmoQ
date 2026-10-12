// The own clients against a peer that stays quiet: `xmoq-quiet-peer`
// (`crates/quiet-peer`), a server that takes their session and sends no
// SETUP, or answers no request. No relay is in between, and what the client
// did is read from what the peer printed: what reached another endpoint.
//
// Usage: bun quiet-peer.ts --case <case> --client own-browser|own-rust|xmoq-watch|quiet-peer [--port N]
//
// The clients' waits are set to 1.5 seconds; by themselves they are 10.
//
//   no-setup          The peer sends no SETUP. The client closes the session
//                     with CONTROL_MESSAGE_TIMEOUT: the peer prints
//                     `SESSION_ENDED ... ClosedByPeer { code: 17,` (0x11),
//                     between 1.5 and 5 seconds after the client began. All
//                     three clients. (The rows "A peer that sends no SETUP"
//                     and, for the two below, "A request the peer does not
//                     answer" of docs/moqt-implementation-notes.md.)
//   no-publish-answer The peer answers a PUBLISH_NAMESPACE and no PUBLISH.
//                     The client ends its PUBLISH with PUBLISH_DONE and a
//                     FIN: the peer prints, for the stream of the PUBLISH,
//                     `PUBLISH_DONE status_code=0x0 stream_count=0
//                     reason="no answer in time"` and then `REQUEST_CLOSED
//                     ... how=FIN`, and no session has ended before that.
//                     own-browser and own-rust; the terminal viewer
//                     publishes nothing.
//   no-fetch-answer   The peer answers nothing. The client gives its FETCH
//                     up: the peer prints `REQUEST ... FETCH` and then
//                     `REQUEST_CLOSED ... how=reset, code 0x1` for that
//                     stream. own-browser (`ended` of the FETCH has
//                     `no-answer-in-time`) and own-rust (`ANSWER_TIMED_OUT
//                     request=fetch#0 after_ms=1500`, and it leaves with a
//                     failure). The terminal viewer sends a FETCH for its
//                     catalog alone, which has 5 seconds as a whole.
//
//   no-stream         The peer sends its SETUP and allows the client no
//                     bidirectional stream. The client's SUBSCRIBE waits
//                     for a stream and fails as not sent: the page's
//                     `subscribe()` with `RequestAbortedError` of
//                     `not-sent-in-time`, its session still open; own-rust
//                     with `NOT_SENT request=subscribe after_ms=1500`; the
//                     terminal viewer with "the relay allowed no stream".
//                     The peer has printed no `REQUEST` line: nothing was
//                     sent. All three clients. (The row "A request that
//                     waits for a stream".)
//   no-subgroup-stream
//                     The peer answers a PUBLISH_NAMESPACE and a PUBLISH and
//                     allows the client no unidirectional stream but its
//                     control stream. The Object the client is to send has
//                     no stream: it is not sent, the client says so, and
//                     the session goes on. own-browser: `sendObject` fails
//                     with `StreamNotOpenedError`, and its session is open
//                     afterwards. own-rust is told to send six Objects: the
//                     peer has room for the first one or two streams and
//                     takes none of them, so the later Objects are not
//                     sent (`NOT_SENT group=G object=0`, one at least), and
//                     its PUBLISH_DONE has the Stream Count of the streams
//                     it did open. (The row "A subgroup stream the peer
//                     allows no room for".)
//   relay-no-setup    The other way round: the own relay, told to give a
//                     client 1.5 seconds for its SETUP
//                     (`--setup-timeout-ms`; 10 by itself), and
//                     `xmoq-quiet-peer` as a client that connects and
//                     sends none (`--client quiet-peer`). The relay closes
//                     the session with CONTROL_MESSAGE_TIMEOUT: it prints
//                     `CLOSE_SESSION session=0 code=0x11 reason="no SETUP
//                     in time"`, and the client `SESSION_ENDED ClosedByPeer
//                     { code: 17, reason: "no SETUP in time" }`, between
//                     1.5 and 5 seconds after the client began.
//
//   relay-idle-session
//                     The own relay, told to let a session hold nothing
//                     for 1.5 seconds (`--idle-session-ms`; 30 by itself),
//                     and `xmoq-quiet-peer` as a client that sends its
//                     SETUP and nothing after it. The relay closes the
//                     session with NO_ERROR: it prints `CLOSE_SESSION
//                     session=0 code=0x0 reason="the session held
//                     nothing"`, and the client `SESSION_ENDED ClosedByPeer
//                     { code: 0, reason: "the session held nothing" }`,
//                     between 1.5 and 5 seconds after the client began.
//                     Beside it an own-rust publisher that announced its
//                     namespace and has no subscriber: it holds something,
//                     and is still there when the idle one has gone.
//
// Not seen here: the STOP_SENDING for the peer's direction of a request,
// which the peer does not watch; the run with `no answer:` in its name has
// it from the relay's log, for a SUBSCRIBE.

import { parseArgs } from "node:util";
import {
	log,
	NAMESPACE,
	openPage,
	type RunContext,
	runCleanups,
	spawnQuietClient,
	spawnRustClient,
	spawnTerminalViewer,
	startQuietPeer,
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
		case: { type: "string", default: "" },
		client: { type: "string", default: "own-rust" },
		port: { type: "string", default: "24397" },
		// The matrix gives every driver a relay; this one has none.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const WAIT_MS = 1500;
const BEFORE_MS = 5000;
const CLIENTS = ["own-browser", "own-rust", "xmoq-watch"] as const;
type QuietClient = (typeof CLIENTS)[number];

function isQuietClient(value: string): value is QuietClient {
	return CLIENTS.some((client) => client === value);
}

/** The page of the run, connected to the peer with the short waits. */
async function connectedPage(context: RunContext): Promise<{
	connect: Promise<string>;
	evaluate: Awaited<ReturnType<typeof openPage>>["evaluate"];
}> {
	const page = await openPage(context, "client:own-browser", "own-browser");
	const connect = page
		.evaluate(
			([url, hash, wait]) =>
				window.xmoq.connect(url, hash, undefined, wait, wait, wait),
			[context.relayUrl, context.certSha256Hex, WAIT_MS] satisfies [
				string,
				string,
				number,
			],
		)
		.then(
			() => "connected",
			(error: unknown) => String(error),
		);
	return { connect, evaluate: page.evaluate.bind(page) };
}

const VIEWER_ARGS = [
	"--namespace",
	NAMESPACE,
	"--video",
	TRACK,
	"--audio",
	"sound",
	"--sound",
	"off",
	"--medium",
	"direct",
	"--out",
	"/dev/null",
];

async function noSetup(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	const began = Date.now();
	let said: string;
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			said = await withTimeout(page.connect, "the page to give up");
			if (!said.includes("the peer sent no SETUP in time")) {
				throw new Error(
					`the page's connect() was to fail for no SETUP: ${said}`,
				);
			}
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
				["--setup-wait-ms", String(WAIT_MS)],
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			if (code === 0) throw new Error("own-rust was to fail, and left with 0");
			said = `it left with ${code}`;
			break;
		}
		case "xmoq-watch": {
			const viewer = spawnTerminalViewer(context, "client:xmoq-watch", [
				...VIEWER_ARGS,
				"--setup-wait-ms",
				String(WAIT_MS),
			]);
			const [line] = await viewer.waitForLine(
				new RegExp(`the relay sent no SETUP within ${WAIT_MS} ms`),
			);
			said = line;
			break;
		}
	}
	// 17 is 0x11, CONTROL_MESSAGE_TIMEOUT.
	await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 17,/,
	);
	const after = Date.now() - began;
	if (after < WAIT_MS || after > BEFORE_MS) {
		throw new Error(
			`${client} was to close between ${WAIT_MS} and ${BEFORE_MS} ms, not after ${after}`,
		);
	}
	return `${client} closed its session with CONTROL_MESSAGE_TIMEOUT (0x11), which the peer received, ${after} ms after it began (its wait: ${WAIT_MS} ms): ${said}`;
}

async function noPublishAnswer(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said: string;
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			await withTimeout(page.connect, "the page to connect");
			said = await withTimeout(
				page
					.evaluate(
						([namespace, track]) => window.xmoq.publishTrack(namespace, track),
						[NAMESPACE, TRACK] satisfies [string, string],
					)
					.then(
						() => "it was answered",
						(error: unknown) => String(error),
					),
				"the page to give its PUBLISH up",
			);
			if (!said.includes("no-answer-in-time")) {
				throw new Error(
					`the page's publish() was to fail for no answer in time: ${said}`,
				);
			}
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"publish",
				TRACK,
				["--payload-hex", "00", "--answer-wait-ms", String(WAIT_MS)],
			);
			const [line] = await rust.waitForLine(
				new RegExp(`^ANSWER_TIMED_OUT request=publish after_ms=${WAIT_MS}$`),
			);
			said = line;
			break;
		}
		case "xmoq-watch":
			throw new Error("the terminal viewer publishes nothing");
	}
	const done =
		/^REQUEST session=0 stream=(\d+) PUBLISH_DONE status_code=0x0 stream_count=0 reason="no answer in time"$/;
	const [, stream] = await context.relay.waitForLine(done);
	await context.relay.waitForLine(
		new RegExp(`^REQUEST_CLOSED session=0 stream=${stream} how=FIN$`),
	);
	const ended = await context.relay.linesBefore(
		{ line: done },
		/^SESSION_ENDED /,
	);
	if (ended.length > 0) {
		throw new Error("the session was to stay open until the PUBLISH_DONE");
	}
	return `${client} ended a PUBLISH nobody answered with PUBLISH_DONE (INTERNAL_ERROR, Stream Count 0, "no answer in time") and a FIN, which the peer read from the wire: ${said}`;
}

async function noFetchAnswer(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said: string;
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			await withTimeout(page.connect, "the page to connect");
			said = await withTimeout(
				page.evaluate(
					([namespace, trackName]) =>
						window.xmoqOwn.fetchRange(
							{
								type: "Standalone",
								namespace,
								trackName,
								start: ["0", "0"],
								end: ["1", "0"],
							},
							// Its own cancel, long after the wait for the answer.
							15_000,
						),
					[NAMESPACE, TRACK] satisfies [string, string],
				),
				"the page to give its FETCH up",
			);
			if (!said.includes("no-answer-in-time")) {
				throw new Error(
					`the page's FETCH was to end for no answer in time: ${said}`,
				);
			}
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"fetch",
				TRACK,
				[
					"--fetch-start",
					"0,0",
					"--fetch-end",
					"1,0",
					"--answer-wait-ms",
					String(WAIT_MS),
				],
			);
			const [line] = await rust.waitForLine(
				new RegExp(`^ANSWER_TIMED_OUT request=fetch#0 after_ms=${WAIT_MS}$`),
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			if (code === 0) throw new Error("own-rust was to fail, and left with 0");
			said = `${line}, and it left with ${code}`;
			break;
		}
		case "xmoq-watch":
			throw new Error("the terminal viewer sends no FETCH but for its catalog");
	}
	const [, stream] = await context.relay.waitForLine(
		/^REQUEST session=0 stream=(\d+) FETCH$/,
	);
	await context.relay.waitForLine(
		new RegExp(
			`^REQUEST_CLOSED session=0 stream=${stream} how=reset, code 0x1$`,
		),
	);
	return `${client} gave up a FETCH nobody answered, and the peer saw its request stream reset with CANCELLED: ${said}`;
}

async function noStream(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said: string;
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			await withTimeout(page.connect, "the page to connect");
			said = await withTimeout(
				page
					.evaluate(
						([namespace, track]) => window.xmoq.subscribe(namespace, track),
						[NAMESPACE, TRACK] satisfies [string, string],
					)
					.then(
						() => "it was answered",
						(error: unknown) => String(error),
					),
				"the page to give up a SUBSCRIBE it cannot send",
			);
			if (!said.includes("not-sent-in-time")) {
				throw new Error(
					`the page's subscribe() was to fail as not sent: ${said}`,
				);
			}
			const state = await page.evaluate(() => window.xmoqOwn.state());
			if (!state.includes('"session":null')) {
				throw new Error(`the page's session was to stay open: ${state}`);
			}
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
				["--open-wait-ms", String(WAIT_MS)],
			);
			const [line] = await rust.waitForLine(
				new RegExp(`^NOT_SENT request=subscribe after_ms=${WAIT_MS}$`),
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			if (code === 0) throw new Error("own-rust was to fail, and left with 0");
			said = `${line}, and it left with ${code}`;
			break;
		}
		case "xmoq-watch": {
			const viewer = spawnTerminalViewer(context, "client:xmoq-watch", [
				...VIEWER_ARGS,
				"--open-wait-ms",
				String(WAIT_MS),
			]);
			const [line] = await viewer.waitForLine(
				new RegExp(
					`the relay allowed no stream for ${TRACK} within ${WAIT_MS} ms`,
				),
			);
			await withTimeout(viewer.exited, "xmoq-watch to leave");
			said = line;
			break;
		}
	}
	// Nothing was sent. For the two that leave, every line of the peer
	// before the end of their session has been read by then.
	if (client !== "own-browser") {
		const sent = await context.relay.linesBefore(
			{ line: /^SESSION_ENDED session=0 ClosedByPeer \{ code: 0,/ },
			/^REQUEST /,
		);
		if (sent.length > 0) throw new Error("the client was to send no request");
	} else if (context.relay.hasPrinted(/^REQUEST /)) {
		throw new Error("the page was to send no request");
	}
	return `${client} could not send a SUBSCRIBE to a peer that allowed it no stream, and failed it as not sent after its wait (${WAIT_MS} ms); the peer read no request: ${said}`;
}

async function noSubgroupStream(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said: string;
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			await withTimeout(page.connect, "the page to connect");
			await withTimeout(
				page.evaluate(
					([namespace, track]) => window.xmoq.publishTrack(namespace, track),
					[NAMESPACE, TRACK] satisfies [string, string],
				),
				"the page to publish its track",
			);
			said = await withTimeout(
				page
					.evaluate(() => Promise.resolve(window.xmoq.sendObject("00")))
					.then(
						() => "it was sent",
						(error: unknown) => String(error),
					),
				"the page to send its Object, or not",
			);
			if (!said.includes("StreamNotOpenedError")) {
				throw new Error(
					`the page's Object was to be not sent, for a stream that was not opened: ${said}`,
				);
			}
			const state = await page.evaluate(() => window.xmoqOwn.state());
			if (!state.includes('"session":null')) {
				throw new Error(`the page's session was to stay open: ${state}`);
			}
			if (context.relay.hasPrinted(/^SESSION_ENDED /)) {
				throw new Error("the peer was to see the session go on");
			}
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"publish",
				TRACK,
				["--payload-hex", "00", "--publish-on-line"],
			);
			await rust.waitForLine(/^READY_TO_PUBLISH$/);
			rust.child.stdin.write("publish\n");
			await rust.waitForLine(/^PUBLISHED /);
			// WHY six: the peer allows this client a stream or two beyond its
			// control stream (its HTTP/3 layer uses fewer than the peer sets
			// aside), does not take them, and so has no room after those.
			const objects = 6;
			rust.child.stdin.write("send\n".repeat(objects));
			await rust.waitForLine(
				new RegExp(`^(?:NOT_SENT|SENT) group=${objects - 1} object=0`),
			);
			const sent = rust.matches(/^SENT group=\d+ /).length;
			const notSent = rust.matches(/^NOT_SENT group=\d+ object=0$/).length;
			if (notSent === 0 || sent + notSent !== objects) {
				throw new Error(
					`own-rust was to leave out the Objects the peer had no stream for: ${sent} sent, ${notSent} not sent, of ${objects}`,
				);
			}
			// Its input ends: the track is over, and it says so. The streams
			// that were opened are counted, and no other (§10.11).
			rust.child.stdin.end();
			const [, counted] = await context.relay.waitForLine(
				/^REQUEST session=0 stream=\d+ PUBLISH_DONE status_code=0x2 stream_count=(\d+) /,
			);
			if (Number(counted) !== sent) {
				throw new Error(
					`the Stream Count of PUBLISH_DONE was to be the ${sent} streams that were opened, not ${counted}`,
				);
			}
			said = `${sent} of ${objects} Objects sent and ${notSent} not sent (NOT_SENT), then PUBLISH_DONE with Stream Count ${counted}`;
			break;
		}
		case "xmoq-watch":
			throw new Error("the terminal viewer publishes nothing");
	}
	return `${client} did not send a Subgroup the peer allowed no stream for, said so, and its session went on: ${said}`;
}

const CASES: Record<
	string,
	{
		quiet: "setup" | "requests" | "publish" | "streams" | "subgroups";
		run: (context: RunContext, client: QuietClient) => Promise<string>;
	}
> = {
	"no-setup": { quiet: "setup", run: noSetup },
	"no-publish-answer": { quiet: "publish", run: noPublishAnswer },
	"no-fetch-answer": { quiet: "requests", run: noFetchAnswer },
	"no-stream": { quiet: "streams", run: noStream },
	"no-subgroup-stream": { quiet: "subgroups", run: noSubgroupStream },
};

async function relayNoSetup(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
		relayArgs: ["--setup-timeout-ms", String(WAIT_MS)],
	});
	const began = Date.now();
	const client = spawnQuietClient(context, "client:quiet-peer", 20_000);
	// 17 is 0x11, CONTROL_MESSAGE_TIMEOUT.
	const [said] = await client.waitForLine(
		/^SESSION_ENDED ClosedByPeer \{ code: 17, reason: "no SETUP in time" \}$/,
	);
	const after = Date.now() - began;
	if (after < WAIT_MS || after > BEFORE_MS) {
		throw new Error(
			`the relay was to close between ${WAIT_MS} and ${BEFORE_MS} ms, not after ${after}`,
		);
	}
	await context.relay.waitForLine(
		/^CLOSE_SESSION session=0 code=0x11 reason="no SETUP in time"$/,
	);
	return `the own relay closed the session of a client that sent no SETUP with CONTROL_MESSAGE_TIMEOUT (0x11) ${after} ms after the client began (the relay's wait: ${WAIT_MS} ms): ${said}`;
}

async function relayIdleSession(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-rust"],
		verbose: options.verbose,
		relayArgs: ["--idle-session-ms", String(WAIT_MS)],
	});
	// A session that holds something: a namespace, and nobody asks for it.
	const publisher = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish-namespace",
		TRACK,
		["--payload-hex", "00", "--serve-on-line", "--timeout-secs", "60"],
	);
	await publisher.waitForLine(/^NAMESPACE_PUBLISHED/);
	const began = Date.now();
	const client = spawnQuietClient(context, "client:quiet-peer", 20_000, true);
	const [said] = await client.waitForLine(
		/^SESSION_ENDED ClosedByPeer \{ code: 0, reason: "the session held nothing" \}$/,
	);
	const after = Date.now() - began;
	if (after < WAIT_MS || after > BEFORE_MS) {
		throw new Error(
			`the relay was to close between ${WAIT_MS} and ${BEFORE_MS} ms, not after ${after}`,
		);
	}
	const closed =
		/^CLOSE_SESSION session=1 code=0x0 reason="the session held nothing"$/;
	const others = await context.relay.linesBefore(
		{ line: closed },
		/^(?:CLOSE_SESSION|SESSION_CLOSED) session=0\b/,
	);
	if (others.length > 0) {
		throw new Error("the publisher that announced was to stay");
	}
	return `the own relay closed a session that sent its SETUP and held nothing with NO_ERROR ${after} ms after the client began (the relay's wait: ${WAIT_MS} ms), and kept the session of a publisher that had announced a namespace: ${said}`;
}

const RELAY_CASES: Record<string, () => Promise<string>> = {
	"relay-no-setup": relayNoSetup,
	"relay-idle-session": relayIdleSession,
};

async function main(): Promise<void> {
	const ofTheRelay = RELAY_CASES[options.case];
	if (ofTheRelay !== undefined) {
		let summary: string;
		try {
			summary = await ofTheRelay();
		} finally {
			await runCleanups();
		}
		log("driver", `RESULT ${JSON.stringify({ case: options.case })}`);
		log("driver", `SUMMARY ${summary}`);
		return;
	}
	const chosen = CASES[options.case];
	if (chosen === undefined) throw new Error(`unknown --case ${options.case}`);
	const client = options.client;
	if (!isQuietClient(client)) throw new Error(`unknown --client ${client}`);
	let summary: string;
	try {
		const context = await startQuietPeer({
			port: options.port,
			quiet: chosen.quiet,
			clients: client === "own-browser" ? ["own-browser"] : [],
		});
		summary = await chosen.run(context, client);
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ case: options.case, client })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED quiet peer ${options.case}: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
