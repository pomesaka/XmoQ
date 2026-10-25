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
//   goaway            The peer refuses the first request of its first
//                     session with GOING_AWAY and then sends GOAWAY, naming
//                     the URI
//                     `https://127.0.0.1:<port>/moved` of itself, and
//                     answers SUBSCRIBE on its later sessions (§3.6, §10.4; the row "GOAWAY" of
//                     docs/moqt-implementation-notes.md). Every client
//                     closes that session with NO_ERROR: the peer prints
//                     `SESSION_ENDED session=0 ClosedByPeer { code: 0,`.
//                     own-browser (`keepPlaying` of `@xmoq/media`) and the
//                     terminal viewer then connect to that URI (`SESSION
//                     session=1 path=/moved`); own-rust prints `GOAWAY ...`
//                     and leaves with 0. The peer refuses every request of
//                     that session with GOING_AWAY, as §10.4 has it, and the
//                     client reads the refusal before the GOAWAY.
//   goaway-request    The peer answers the first SUBSCRIBE and sends GOAWAY
//                     on its stream, naming the same URI (§10.4). The page
//                     (`movedBy` of `keepPlaying`) and the terminal viewer
//                     close the session with NO_ERROR and connect to the
//                     URI; own-rust prints `REQUEST_GOAWAY ...` and leaves
//                     with 0.
//   request-id-parity The peer sends the client a SUBSCRIBE with the Request
//                     ID 0, a client's (§10.1). own-browser, own-rust and
//                     the terminal viewer close the session with
//                     INVALID_REQUEST_ID: the peer
//                     prints `SESSION_ENDED session=0 ClosedByPeer { code:
//                     4,`, and own-rust `CLOSED_FOR_A_REQUEST_OF_THE_PEER
//                     code=0x4 ...`.
//   datagram          The peer sends the client, once its SETUP has come, a
//                     datagram of the type 0x30, which is no
//                     OBJECT_DATAGRAM (§11, §11.3.1: "MUST close the
//                     session with a PROTOCOL_VIOLATION"). own-rust and the
//                     terminal viewer close the session with
//                     PROTOCOL_VIOLATION: the peer prints `SESSION_ENDED
//                     session=0 ClosedByPeer { code: 3,`, and own-browser
//                     too, whose `@xmoq/moqt` reads every datagram. The peer
//                     answers no request: the clients close while they wait
//                     for the answer to their SUBSCRIBE (the terminal viewer
//                     reads its streams' events while it waits since
//                     2026-10-24; before, it closed with NO_ERROR when the
//                     answer did not come in 10 s).
//   datagram-before-setup
//                     The same datagram, from a peer that sends no SETUP
//                     (added on 2026-10-24, written before its first run):
//                     the clients have it while they wait for the SETUP, and
//                     close the session with PROTOCOL_VIOLATION all the same.
//                     Under load the datagram of `datagram` came first as
//                     well, and the terminal viewer closed with NO_ERROR then
//                     (1 of a run of the interop on 2026-10-24).
//   silent-subgroup   The peer answers the client's SUBSCRIBE and opens a
//                     subgroup stream of the track with its Subgroup Header
//                     and nothing after it. The client, given an idle time
//                     of 1 second (own-rust `--idle-subgroup-stream-ms`, the
//                     page `idleSubgroupStreamTimeoutMs`; 30 by themselves),
//                     stops that stream alone: the peer prints
//                     `STOP_SENDING_RECEIVED session=0 code=0x0 after_ms=N`
//                     with N from 1,000 to 5,000, and a second later the
//                     session is open at both ends (own-rust has printed
//                     `STREAM_ABORTED uni#N` and not left; the page's state
//                     has no end of its session). own-browser and own-rust.
//                     (The row "A subgroup stream of the peer that sends
//                     nothing after its header".)
//   request-id-again  The same, with a SUBSCRIBE of the Request ID 1, which
//                     the client refuses, and then a REQUEST_UPDATE with the
//                     same Request ID on its stream.
//   goaway-publisher  The same, with the page as a publisher
//                     (`keepPublishing`): its PUBLISH_NAMESPACE is refused,
//                     and it announces on the URI (`REQUEST session=1
//                     stream=N PUBLISH_NAMESPACE`, then `publishing 1`).
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
//   relay-many-announcements
//                     The own relay with its own limits, an own-rust
//                     publisher and subscriber of a track, and
//                     `xmoq-quiet-peer` as a client that sends
//                     PUBLISH_NAMESPACE 20 times a second for 3 seconds
//                     and withdraws each when it is answered
//                     (`--announce-per-second 20`). The relay accepts 5 to
//                     8 of them (five at once, and one more each second)
//                     and refuses the others with EXCESSIVE_LOAD (`SEND
//                     session=2 stream=S REQUEST_ERROR error_code=0x9`)
//                     and a Retry Interval of 1,001: one more may be sent
//                     after a second.
//                     It closes no session, and the subscriber gets the
//                     Object the publisher sends afterwards.
//
//   relay-one-session-asks-for-much
//                     The own relay, and `xmoq-quiet-peer` three times: as
//                     a publisher that announces `(xmoq)`, publishes the
//                     track `live` and answers nothing the relay asks of it
//                     (`--announce-and-stay-quiet yes`); as a session that
//                     sends 30 SUBSCRIBEs a second, each for a new track
//                     name; and as a viewer that sends one SUBSCRIBE a
//                     second for a name of its own. The relay has 8
//                     requests open on the publisher for the first session
//                     and refuses its others with EXCESSIVE_LOAD (0x9);
//                     every SUBSCRIBE of the viewer is passed on to the
//                     publisher, which shows as TIMEOUT (0x2) 5 seconds
//                     later. Then the same with FETCH for Group 0 of
//                     `live`: 10 a second, of which the relay has 4 open,
//                     and the viewer's, one every 1.5 seconds, are all
//                     passed on.
//
//   relay-slow-cached-fetch
//                     The own relay, told to give a subscriber 3 seconds
//                     for the whole of a fetch stream it writes from its
//                     cache (`--cached-fetch-whole-ms 3000`; 30 seconds and
//                     more by itself), and `xmoq-quiet-peer` as a publisher
//                     of one Group of 8 MiB and as two readers that FETCH
//                     it. The one that takes 16 KiB a second, each Object
//                     in time and the whole too slowly, has its stream
//                     reset with TOO_FAR_BEHIND (`FETCH_WRITE_ENDED
//                     session=1 stream=S too_slow pieces=N code=0x5`); the
//                     one that reads gets the whole Group and a FIN.
//
//   relay-group-kept-beyond-the-cache
//                     The own relay, told to let the fetch streams it
//                     writes keep 16 MB of Groups its cache has let go of
//                     (`--held-beyond-cache-bytes 16000000`; 256 MiB by
//                     itself), which is 2 MB for one session. A reader
//                     takes a Group of 8 MiB at 64 KiB a second; four
//                     seconds later the publisher sends the next Group, and
//                     the cache lets go of the first. The relay stops the
//                     reader's stream with EXCESSIVE_LOAD
//                     (`HELD_BEYOND_CACHE session=1 stream=S stopped
//                     relay_bytes=B session_bytes=B`, with B what the
//                     stream carries of the Group, then
//                     `FETCH_WRITE_ENDED session=1 stream=S aborted
//                     pieces=N code=0x9`). Another reader's FETCH for the
//                     Group the cache has then is answered.
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
	pollFor,
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
				new RegExp(`the peer sent no SETUP within ${WAIT_MS} ms`),
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

/**
 * A GOAWAY of the peer (§3.6, §10.4) on the first session, with the New
 * Session URI `/moved` of the same peer. The page (`keepPlaying` of
 * `@xmoq/media`) and the terminal viewer close that session with NO_ERROR
 * and connect to that URI; own-rust closes it with NO_ERROR and leaves with
 * 0. The peer answers their SUBSCRIBEs, so that each is playing when it
 * reads the GOAWAY.
 */
/**
 * The milliseconds between the peer printing `first` and `second`, as the
 * driver reads its lines: to within a turn of the driver's loop. Called
 * before either is printed.
 */
async function timeBetween(
	context: RunContext,
	first: RegExp,
	second: RegExp,
): Promise<number> {
	const [from, to] = await Promise.all([
		context.relay.waitForLine(first).then(() => performance.now()),
		context.relay.waitForLine(second).then(() => performance.now()),
	]);
	return Math.round(to - from);
}

async function goAway(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	const sent = context.relay.waitForLine(/^GOAWAY_SENT session=0 uri=(\S+)$/);
	// own-rust does not connect again.
	const moving =
		client === "own-rust"
			? undefined
			: timeBetween(
					context,
					/^GOAWAY_SENT session=0 /,
					/^SESSION session=1 path=\/moved$/,
				);
	let said = "";
	switch (client) {
		case "own-browser": {
			const page = await openPage(context, "client:own-browser", "own-browser");
			await withTimeout(
				page.evaluate(
					([url, hash, namespace, track]) =>
						window.xmoqOwn.keepVideoPlayed(
							url,
							hash,
							namespace,
							track,
							"avc1.42001f",
						),
					[
						context.relayUrl,
						context.certSha256Hex,
						NAMESPACE,
						TRACK,
					] satisfies [string, string, string, string],
				),
				"the page to begin playing",
			);
			const report = async () =>
				(await page.evaluate(() => window.xmoqOwn.keptReport())).playing;
			await context.relay.waitForLine(/^SESSION session=1 path=\/moved$/);
			// On the new session the peer answers the SUBSCRIBE, and the page plays.
			await pollFor(
				async () => ((await report()).includes("playing 1") ? true : undefined),
				"the page to play on the new session",
			);
			said = `its states: ${(await report()).join(", ")}`;
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
			);
			const [line] = await rust.waitForLine(
				/^GOAWAY uri=\S+\/moved timeout_ms=5000 request_id=0$/,
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			if (code !== 0)
				throw new Error(`own-rust was to leave with 0, not ${code}`);
			said = `${line}, and it left with 0`;
			break;
		}
		case "xmoq-watch": {
			const viewer = spawnTerminalViewer(
				context,
				"client:xmoq-watch",
				VIEWER_ARGS,
			);
			const [line] = await viewer.waitForLine(/MOVED https:\/\/\S+\/moved/);
			said = line;
			break;
		}
	}
	const [, uri] = await sent;
	// 0 is NO_ERROR (§3.5).
	await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 0,/,
	);
	if (client === "own-rust") {
		if (context.relay.hasPrinted(/^SESSION session=1 /)) {
			throw new Error("own-rust was to connect once");
		}
		return `own-rust closed its session with NO_ERROR when the peer sent GOAWAY naming ${uri}: ${said}`;
	}
	return `${client} closed its session with NO_ERROR when the peer sent GOAWAY, and connected to the URI it named (${uri}) ${await moving} ms after the GOAWAY was sent: ${said}`;
}

/**
 * A GOAWAY on the stream of the client's first SUBSCRIBE (§10.4), after its
 * SUBSCRIBE_OK, naming `/moved` of the same peer. The page and the terminal
 * viewer move their session there, as for a GOAWAY on the control stream;
 * own-rust prints `REQUEST_GOAWAY ...` and leaves with 0.
 */
async function goAwayOnARequest(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	const sent = context.relay.waitForLine(
		/^REQUEST_GOAWAY_SENT session=0 stream=0 uri=(\S+)$/,
	);
	// own-rust does not connect again.
	const moving =
		client === "own-rust"
			? undefined
			: timeBetween(
					context,
					/^REQUEST_GOAWAY_SENT session=0 /,
					/^SESSION session=1 path=\/moved$/,
				);
	let said = "";
	switch (client) {
		case "own-browser": {
			const page = await openPage(context, "client:own-browser", "own-browser");
			await withTimeout(
				page.evaluate(
					([url, hash, namespace, track]) =>
						window.xmoqOwn.keepVideoPlayed(
							url,
							hash,
							namespace,
							track,
							"avc1.42001f",
						),
					[
						context.relayUrl,
						context.certSha256Hex,
						NAMESPACE,
						TRACK,
					] satisfies [string, string, string, string],
				),
				"the page to begin playing",
			);
			const states = await pollFor(async () => {
				const { playing } = await page.evaluate(() =>
					window.xmoqOwn.keptReport(),
				);
				return playing.includes("playing 2") ? playing : undefined;
			}, "the page to play on the new session");
			said = `its states: ${states.join(", ")}`;
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
			);
			const [line] = await rust.waitForLine(
				/^REQUEST_GOAWAY uri=\S+\/moved timeout_ms=5000$/,
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			if (code !== 0)
				throw new Error(`own-rust was to leave with 0, not ${code}`);
			said = `${line}, and it left with 0`;
			break;
		}
		case "xmoq-watch": {
			const viewer = spawnTerminalViewer(
				context,
				"client:xmoq-watch",
				VIEWER_ARGS,
			);
			const [line] = await viewer.waitForLine(/MOVED https:\/\/\S+\/moved/);
			said = line;
			break;
		}
	}
	const [, uri] = await sent;
	await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 0,/,
	);
	if (client === "own-rust") {
		return `own-rust closed its session with NO_ERROR when the peer sent GOAWAY on its SUBSCRIBE's stream naming ${uri}: ${said}`;
	}
	return `${client} closed its session with NO_ERROR when the peer sent GOAWAY on its SUBSCRIBE's stream, and connected to the URI it named (${uri}) ${await moving} ms after the GOAWAY was sent: ${said}`;
}

/**
 * The same peer, and the page as a publisher (`keepPublishing` of
 * `@xmoq/media`): its PUBLISH_NAMESPACE is refused with GOING_AWAY on the
 * first session, it closes that session with NO_ERROR, and announces on the
 * URI the GOAWAY names.
 */
async function goAwayPublisher(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	if (client !== "own-browser") {
		throw new Error("only the page publishes with keepPublishing");
	}
	const page = await openPage(context, "client:own-browser", "own-browser");
	await withTimeout(
		page.evaluate(
			([url, hash, namespace, track]) =>
				window.xmoqOwn.keepVideoPublished(url, hash, namespace, {
					trackName: track,
					codec: "avc1.42E01F",
					catalogCodec: "avc3.42E01F",
					width: 320,
					height: 180,
					framerate: 25,
					bitrate: 500_000,
					keyFrameIntervalMs: 1000,
					firstTimestamp: 5_000_000,
					noise: false,
					mapping: "stream-per-group",
				}),
			[context.relayUrl, context.certSha256Hex, NAMESPACE, TRACK] satisfies [
				string,
				string,
				string,
				string,
			],
		),
		"the page to begin publishing",
	);
	const [, uri] = await context.relay.waitForLine(
		/^GOAWAY_SENT session=0 uri=(\S+)$/,
	);
	await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 0,/,
	);
	await context.relay.waitForLine(/^SESSION session=1 path=\/moved$/);
	await context.relay.waitForLine(
		/^REQUEST session=1 stream=\d+ PUBLISH_NAMESPACE$/,
	);
	const states = await pollFor(async () => {
		const { publishing } = await page.evaluate(() =>
			window.xmoqOwn.keptReport(),
		);
		return publishing.includes("publishing 1") ? publishing : undefined;
	}, "the page to publish on the new session");
	return `own-browser closed its session with NO_ERROR when the peer refused its PUBLISH_NAMESPACE with GOING_AWAY and sent GOAWAY, and announced on the URI it named (${uri}): its states: ${states.join(", ")}`;
}

/**
 * A request the peer opens towards the client with a Request ID §10.1 does
 * not allow: of a client's parity (`request-id-parity`), or used again by a
 * REQUEST_UPDATE on the stream of a request the client refused
 * (`request-id-again`). The client closes the session with
 * INVALID_REQUEST_ID (0x4). own-rust refuses the peer's requests in its
 * `subscribe` mode, the page for want of `serve`, and the terminal viewer
 * always.
 */
async function peerRequestId(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said = "";
	switch (client) {
		case "own-browser": {
			const page = await connectedPage(context);
			said = await withTimeout(page.connect, "the page to connect");
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
			);
			const [line] = await rust.waitForLine(
				/^CLOSED_FOR_A_REQUEST_OF_THE_PEER code=0x4 /,
			);
			said = line;
			break;
		}
		case "xmoq-watch": {
			// It plays (the peer answers its SUBSCRIBEs), refuses the peer's
			// request, and leaves with the session's error.
			const viewer = spawnTerminalViewer(
				context,
				"client:xmoq-watch",
				VIEWER_ARGS,
			);
			const code = await withTimeout(viewer.exited, "xmoq-watch to leave");
			said = `it left with ${code}`;
			break;
		}
	}
	// 4 is INVALID_REQUEST_ID (§3.5).
	const [line] = await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 4,.*$/,
	);
	return `${client} closed its session with INVALID_REQUEST_ID: the peer printed ${line}; ${said}`;
}

const CASES: Record<
	string,
	{
		quiet:
			| "setup"
			| "requests"
			| "publish"
			| "streams"
			| "subgroups"
			| "goaway"
			| "goaway-request"
			| "request-id-parity"
			| "request-id-again"
			| "datagram"
			| "datagram-before-setup"
			| "silent-subgroup";
		run: (context: RunContext, client: QuietClient) => Promise<string>;
	}
> = {
	"no-setup": { quiet: "setup", run: noSetup },
	"no-publish-answer": { quiet: "publish", run: noPublishAnswer },
	"no-fetch-answer": { quiet: "requests", run: noFetchAnswer },
	"no-stream": { quiet: "streams", run: noStream },
	"no-subgroup-stream": { quiet: "subgroups", run: noSubgroupStream },
	goaway: { quiet: "goaway", run: goAway },
	"goaway-publisher": { quiet: "goaway", run: goAwayPublisher },
	"goaway-request": { quiet: "goaway-request", run: goAwayOnARequest },
	"request-id-parity": { quiet: "request-id-parity", run: peerRequestId },
	"request-id-again": { quiet: "request-id-again", run: peerRequestId },
	datagram: { quiet: "datagram", run: unknownDatagram },
	"datagram-before-setup": {
		quiet: "datagram-before-setup",
		run: unknownDatagram,
	},
	"silent-subgroup": { quiet: "silent-subgroup", run: silentSubgroup },
};

/** The idle time the clients are given in `silent-subgroup`; 30 seconds by themselves. */
const IDLE_MS = 1000;

/**
 * The peer answers the client's SUBSCRIBE and opens a subgroup stream of the
 * track that sends its Subgroup Header and nothing after it (`--quiet
 * silent-subgroup`). The client, given an idle time of `IDLE_MS`, stops that
 * stream alone with INTERNAL_ERROR (0x0), and its session goes on.
 */
async function silentSubgroup(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said: string;
	let stillThere: () => Promise<string>;
	switch (client) {
		case "own-browser": {
			const page = await openPage(context, "client:own-browser", "own-browser");
			await withTimeout(
				page.evaluate(
					([url, hash, idle]) =>
						window.xmoq.connect(
							url,
							hash,
							undefined,
							undefined,
							undefined,
							undefined,
							idle,
						),
					[context.relayUrl, context.certSha256Hex, IDLE_MS] satisfies [
						string,
						string,
						number,
					],
				),
				"the page to connect",
			);
			said = await withTimeout(
				page.evaluate(
					([namespace, track]) => window.xmoq.subscribe(namespace, track),
					[NAMESPACE, TRACK] satisfies [string, string],
				),
				"the page's SUBSCRIBE to be answered",
			);
			stillThere = async () => {
				const state = await page.evaluate(() => window.xmoqOwn.state());
				if (!state.includes('"session":null')) {
					throw new Error(`the page's session was to stay open: ${state}`);
				}
				return state;
			};
			break;
		}
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
				["--idle-subgroup-stream-ms", String(IDLE_MS)],
			);
			stillThere = async () => {
				const [line] = await rust.waitForLine(/^STREAM_ABORTED uni#\d+$/);
				const left = await Promise.race([
					rust.exited.then((code) => `it left with ${code}`),
					new Promise<null>((resolve) => setTimeout(() => resolve(null), 0)),
				]);
				if (left !== null) throw new Error(`own-rust was to stay: ${left}`);
				return line;
			};
			said = "own-rust subscribed";
			break;
		}
		case "xmoq-watch":
			throw new Error("the terminal viewer has no flag for the idle time");
	}
	const [line, code, afterMs] = await context.relay.waitForLine(
		/^STOP_SENDING_RECEIVED session=0 code=(\S+) after_ms=(\d+)$/,
	);
	if (code !== "0x0") {
		throw new Error(`the stream was to be stopped with 0x0: ${line}`);
	}
	const after = Number(afterMs);
	if (after < IDLE_MS || after > BEFORE_MS) {
		throw new Error(
			`the stream was to be stopped between ${IDLE_MS} and ${BEFORE_MS} ms after its header: ${line}`,
		);
	}
	// The session goes on: a while later it is still open at both ends.
	await new Promise((resolve) => setTimeout(resolve, IDLE_MS));
	const there = await stillThere();
	if (context.relay.hasPrinted(/^SESSION_ENDED /)) {
		throw new Error("the peer was to see the session go on");
	}
	return `${client} stopped a subgroup stream that sent nothing after its header ${after} ms after it (its idle time: ${IDLE_MS} ms), with STOP_SENDING 0x0, and its session went on: ${line}; ${said}; ${there}`;
}

/**
 * The peer sends a datagram of an unknown type (`--quiet datagram`): the
 * client closes the session with PROTOCOL_VIOLATION (0x3).
 */
async function unknownDatagram(
	context: RunContext,
	client: QuietClient,
): Promise<string> {
	let said = "";
	switch (client) {
		case "own-rust": {
			const rust = spawnRustClient(
				context,
				"client:own-rust",
				client,
				"subscribe",
				TRACK,
			);
			const code = await withTimeout(rust.exited, "own-rust to leave");
			said = `own-rust left with ${code}`;
			break;
		}
		case "xmoq-watch": {
			const viewer = spawnTerminalViewer(
				context,
				"client:xmoq-watch",
				VIEWER_ARGS,
			);
			const code = await withTimeout(viewer.exited, "xmoq-watch to leave");
			said = `xmoq-watch left with ${code}`;
			break;
		}
		case "own-browser": {
			// The datagram comes right after the peer's SETUP, so connect()
			// may end either way: what has to be seen is the peer's line.
			const page = await connectedPage(context);
			said = `the page's connect() ended: ${await withTimeout(page.connect, "the page's connect()")}`;
			break;
		}
	}
	await context.relay.waitForLine(/^DATAGRAM_SENT session=0 type=0x30$/);
	// 3 is PROTOCOL_VIOLATION (§3.5).
	const [line] = await context.relay.waitForLine(
		/^SESSION_ENDED session=0 ClosedByPeer \{ code: 3,.*$/,
	);
	return `${client} closed its session with PROTOCOL_VIOLATION at a datagram of an unknown type: the peer printed ${line}; ${said}`;
}

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

/**
 * The other way round: `xmoq-quiet-peer` as a client sends the own relay a
 * datagram of the type 0x30 after its SETUP. The relay closes the session
 * with PROTOCOL_VIOLATION (0x3): `CLOSE_SESSION session=0 code=0x3`, and the
 * client `SESSION_ENDED ClosedByPeer { code: 3, ... }`.
 */
async function relayUnknownDatagram(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
	});
	const client = spawnQuietClient(context, "client:quiet-peer", 20_000, true, [
		"--send-datagram-hex",
		"30",
	]);
	await client.waitForLine(/^DATAGRAM_SENT /);
	const [said] = await client.waitForLine(
		/^SESSION_ENDED ClosedByPeer \{ code: 3,.*$/,
	);
	const [closed] = await context.relay.waitForLine(
		/^CLOSE_SESSION session=0 code=0x3 .*$/,
	);
	return `the own relay closed the session of a client that sent a datagram of an unknown type with PROTOCOL_VIOLATION: ${closed}; the client: ${said}`;
}

/**
 * A padding datagram (0x132B3E29) the own relay is sent after the client's
 * SETUP is let go of (§11.5.2: "The receiver MUST discard all data received
 * in a padding datagram"): the session stays, and the client prints
 * `STAYED` after its time.
 */
async function relayPaddingDatagram(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
	});
	const client = spawnQuietClient(context, "client:quiet-peer", 1500, true, [
		"--send-datagram-hex",
		"f0132b3e290000",
	]);
	await client.waitForLine(/^DATAGRAM_SENT /);
	const [stayed] = await client.waitForLine(/^STAYED ms=1500$/);
	return `the own relay let a padding datagram go and kept the session: the client printed ${stayed}`;
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

/**
 * A session announces a namespace and withdraws it 20 times a second for 3
 * seconds, beside a publisher and a subscriber of a track.
 */
async function relayManyAnnouncements(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-rust"],
		verbose: options.verbose,
	});
	const payload = "616e6e6f756e6365";
	const publisher = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish",
		TRACK,
		["--payload-hex", payload, "--timeout-secs", "60"],
	);
	await publisher.waitForLine(/^PUBLISHED /);
	const subscriber = spawnRustClient(
		context,
		"sub:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		["--timeout-secs", "60"],
	);
	await subscriber.waitForLine(/^SUBSCRIBED /);
	// The third session of the run.
	const client = spawnQuietClient(context, "client:quiet-peer", 3000, false, [
		"--announce-per-second",
		"20",
	]);
	const [said, sent = "", ok = "", refused = "", failed = ""] =
		await client.waitForLine(
			/^ANNOUNCED sent=(\d+) ok=(\d+) refused=(\d+) failed=(\d+) asked=\d+ retry_interval=1001 ms=\d+$/,
		);
	// Five at once and one more each second: 5 to 8 in 3 seconds.
	if (
		Number(ok) < 5 ||
		Number(ok) > 8 ||
		Number(failed) > 1 ||
		Number(refused) < Number(sent) - 10
	) {
		throw new Error(
			`5 to 8 of the announcements were to be accepted and the others refused: ${said}`,
		);
	}
	await context.relay.waitForLine(
		/^SEND session=2 stream=\d+ REQUEST_ERROR error_code=0x9\b/,
	);
	if (context.relay.hasPrinted(/^CLOSE_SESSION session=2\b/)) {
		throw new Error("the relay was to refuse the requests, and close nothing");
	}
	if (client.matches(/^SESSION_ENDED /).length > 0) {
		throw new Error("the session was to stay open to its own end");
	}
	publisher.child.stdin.write("send\n");
	const [, got = ""] = await subscriber.waitForLine(
		/^OBJECT .*payload_hex=([0-9a-f]*)$/,
	);
	if (got !== payload) {
		throw new Error(
			`the subscriber beside it was to get the Object, not ${got}`,
		);
	}
	return `a session sent ${sent} PUBLISH_NAMESPACE requests in 3 s, each withdrawn when answered: the own relay accepted ${ok} (five at once and one more each second), refused ${refused} with EXCESSIVE_LOAD (0x9) and closed nothing, and the subscriber of a track beside it got its Object afterwards: ${said}`;
}

/**
 * One session asks a publisher that answers nothing for more than its
 * share, and another viewer's requests reach that publisher all the same.
 */
async function relayOneSessionAsksForMuch(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
	});
	const publisher = spawnQuietClient(context, "publisher", 40_000, false, [
		"--announce-and-stay-quiet",
		"yes",
	]);
	await publisher.waitForLine(/^PUBLISHED_LIVE$/);
	const said =
		/^ASKED sent=(\d+) ok=0 refused=(\d+) \[([^\]]*)\] failed=0 unanswered=\d+ ms=\d+$/;
	const parts: string[] = [];
	// (what is asked; how often the one session asks; how often the viewer)
	const scenes = [
		["subscribe", "subscribe-own", "30", "1000", "SUBSCRIBEs"],
		["fetch-live", "fetch-live", "10", "1500", "FETCHes"],
	] as const;
	for (const [muchAsked, viewerAsked, aSecond, every, name] of scenes) {
		const much = spawnQuietClient(context, `much:${muchAsked}`, 9000, false, [
			"--ask-per-second",
			aSecond,
			"--ask",
			muchAsked,
			"--cancel",
			"answered",
		]);
		await new Promise((resolve) => setTimeout(resolve, 1000));
		const viewer = spawnQuietClient(
			context,
			`viewer:${viewerAsked}`,
			7500,
			false,
			["--ask-every-ms", every, "--ask", viewerAsked, "--cancel", "answered"],
		);
		const [viewerSaid, , viewerRefused = "", viewerCodes = ""] =
			await viewer.waitForLine(said, 20_000);
		// The publisher answers nothing, so a request that reached it is
		// answered TIMEOUT (0x2) by the relay 5 seconds later; one that was
		// not passed on is answered EXCESSIVE_LOAD (0x9) at once.
		if (viewerCodes !== `0x2=${viewerRefused}` || Number(viewerRefused) < 2) {
			throw new Error(
				`every request of the viewer was to be passed on to the publisher (TIMEOUT), 2 or more of them: ${viewerSaid}`,
			);
		}
		const [muchSaid, , , muchCodes = ""] = await much.waitForLine(said, 20_000);
		const turnedAway = Number(/0x9=(\d+)/.exec(muchCodes)?.[1] ?? "0");
		const passedOn = Number(/0x2=(\d+)/.exec(muchCodes)?.[1] ?? "0");
		// Its share is 8 (4 for FETCH) at a time, for 5 seconds each: no more
		// than two shares were answered in the 9 seconds.
		const share = muchAsked === "subscribe" ? 8 : 4;
		if (turnedAway < 20 || passedOn > share * 2) {
			throw new Error(
				`the session that asks for much was to have ${share} requests passed on at a time and the others refused with EXCESSIVE_LOAD: ${muchSaid}`,
			);
		}
		parts.push(
			`${name}: the session that sent ${aSecond} a second had ${passedOn} passed on and ${turnedAway} refused with EXCESSIVE_LOAD, and all ${viewerRefused} of the viewer beside it were passed on`,
		);
	}
	return `a publisher that answers nothing, and one session that asks it for more than its share while another viewer asks beside it. ${parts.join(". ")}`;
}

/**
 * A FETCH the relay answers from its cache: a reader that takes the whole
 * too slowly is cut, and one that reads is not.
 */
async function relaySlowCachedFetch(): Promise<string> {
	const WHOLE_MS = 3000;
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
		// The time for the whole is 3 s whatever the bytes: the least rate is
		// one at which the megabyte takes less.
		relayArgs: [
			"--cached-fetch-whole-ms",
			String(WHOLE_MS),
			"--cached-fetch-least-kbps",
			"100000",
		],
	});
	// 8 MiB in one Group, which the relay caches. WHY that much: what fits
	// into the windows of the connection is written at once and waits
	// there, and the time for the whole is of what the relay still has to
	// write.
	const publisher = spawnQuietClient(context, "publisher", 40_000, false, [
		"--publish-group",
		"64x131072",
	]);
	await publisher.waitForLine(/^GROUP_SENT group=0 /);
	// 16 KiB a second: each Object of 128 KiB in 8 seconds, the whole in 512.
	const slow = spawnQuietClient(context, "reader:slow", 12_000, false, [
		"--fetch-cached",
		"1",
		"--read-bytes-a-second",
		"16384",
	]);
	await slow.waitForLine(/^FETCH n=0 FETCH_OK$/);
	const [, , , slowHow = ""] = await slow.waitForLine(
		/^FETCH_STREAM_ENDED after_ms=(\d+) bytes=(\d+) how=(Reset.*)$/,
		15_000,
	);
	// The reader's is the relay's second session.
	const [written] = await context.relay.waitForLine(
		/^FETCH_WRITE_ENDED session=1 stream=\d+ (\S+) pieces=\d+ code=0x5$/,
	);
	if (!/ too_slow /.test(written) || !/Reset\(Some\(5\)\)/.test(slowHow)) {
		throw new Error(
			`the relay was to give the stream up as taken too slowly, with TOO_FAR_BEHIND (0x5): ${written}; the reader: ${slowHow}`,
		);
	}
	const fast = spawnQuietClient(context, "reader:fast", 6000, false, [
		"--fetch-cached",
		"1",
		"--read-bytes-a-second",
		"100000000",
	]);
	const [, , fastBytes = "", fastHow = ""] = await fast.waitForLine(
		/^FETCH_STREAM_ENDED after_ms=(\d+) bytes=(\d{6,}) how=(.*)$/,
		15_000,
	);
	if (fastHow !== "finished" || Number(fastBytes) < 64 * 131072) {
		throw new Error(
			`a reader that reads was to get the whole Group: ${fastBytes} bytes, ${fastHow}`,
		);
	}
	return `the own relay, told to give a subscriber ${WHOLE_MS} ms for the whole of a fetch stream it writes from its cache: a reader that took 16 KiB a second of a Group of 8 MiB had its stream reset with TOO_FAR_BEHIND (${written}), and a reader that read as fast as it could got all ${fastBytes} bytes and a FIN`;
}

/**
 * A fetch stream that keeps a Group the cache has let go of, for a session
 * that has more than its share kept so: it is reset, and another reader is
 * answered.
 */
async function relayGroupKeptBeyondTheCache(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: [],
		verbose: options.verbose,
		// 16 MB for the relay, so 2 MB for one session: less than the Group.
		relayArgs: ["--held-beyond-cache-bytes", "16000000"],
	});
	// Group 0 of 8 MiB, and Group 1 four seconds later.
	const publisher = spawnQuietClient(context, "publisher", 40_000, false, [
		"--publish-group",
		"64x131072",
		"--next-group-after-ms",
		"4000",
	]);
	await publisher.waitForLine(/^GROUP_SENT group=0 /);
	// The relay's second session: it takes 64 KiB a second, each Object in
	// two seconds.
	const slow = spawnQuietClient(context, "reader:slow", 12_000, false, [
		"--fetch-cached",
		"1",
		"--read-bytes-a-second",
		"65536",
	]);
	await slow.waitForLine(/^FETCH n=0 FETCH_OK$/);
	await publisher.waitForLine(/^GROUP_SENT group=1 /);
	const [written] = await context.relay.waitForLine(
		/^FETCH_WRITE_ENDED session=1 stream=\d+ (\S+) pieces=\d+ code=(0x[0-9a-f]+)$/,
	);
	// What the relay says it counted when it stopped the stream: what the
	// stream carries of the Group, for the relay and for the session. WHY
	// not 8,388,608 to the byte: the reader may ask while the relay has not
	// read the whole Group, and is answered with what the cache has of it.
	const [counted, relayBytes = "", sessionBytes = ""] =
		await context.relay.waitForLine(
			/^HELD_BEYOND_CACHE session=1 stream=\d+ stopped relay_bytes=(\d+) session_bytes=(\d+)$/,
		);
	if (relayBytes !== sessionBytes || Number(relayBytes) <= 2_000_000) {
		throw new Error(
			`the relay was to count the same for itself and for the session, and more than the session's 2 MB: ${counted}`,
		);
	}
	if (!/ aborted pieces=\d+ code=0x9$/.test(written)) {
		throw new Error(
			`the relay was to stop the stream that kept the Group, with EXCESSIVE_LOAD (0x9): ${written}`,
		);
	}
	const [, , , slowHow = ""] = await slow.waitForLine(
		/^FETCH_STREAM_ENDED after_ms=(\d+) bytes=(\d+) how=(Reset.*)$/,
		15_000,
	);
	if (!/Reset\(Some\(9\)\)/.test(slowHow)) {
		throw new Error(
			`the slow reader's stream was to be reset with EXCESSIVE_LOAD (0x9): ${slowHow}`,
		);
	}
	// Another reader asks for the Group the cache has now.
	const other = spawnQuietClient(context, "reader:other", 6000, false, [
		"--fetch-cached",
		"1",
		"--fetch-group",
		"1",
		"--read-bytes-a-second",
		"100000000",
	]);
	await other.waitForLine(/^FETCH n=0 FETCH_OK$/);
	const [, , otherBytes = "", otherHow = ""] = await other.waitForLine(
		/^FETCH_STREAM_ENDED after_ms=(\d+) bytes=(\d\d+) how=(.*)$/,
		15_000,
	);
	if (otherHow !== "finished") {
		throw new Error(
			`another reader was to be answered from the cache: ${otherBytes} bytes, ${otherHow}`,
		);
	}
	return `the own relay, told to let its fetch streams keep 16 MB beyond its cache (2 MB for one session): a reader took a Group of 8 MiB slowly, the cache moved on to the next Group, and the relay stopped that stream with EXCESSIVE_LOAD (${counted}; ${written}); another reader's FETCH for the Group the cache had then was answered and finished`;
}

const RELAY_CASES: Record<string, () => Promise<string>> = {
	"relay-no-setup": relayNoSetup,
	"relay-unknown-datagram": relayUnknownDatagram,
	"relay-padding-datagram": relayPaddingDatagram,
	"relay-idle-session": relayIdleSession,
	"relay-many-announcements": relayManyAnnouncements,
	"relay-one-session-asks-for-much": relayOneSessionAsksForMuch,
	"relay-slow-cached-fetch": relaySlowCachedFetch,
	"relay-group-kept-beyond-the-cache": relayGroupKeptBeyondTheCache,
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
