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
				window.xmoq.connect(url, hash, undefined, wait, wait),
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

const CASES: Record<
	string,
	{
		quiet: "setup" | "requests" | "publish";
		run: (context: RunContext, client: QuietClient) => Promise<string>;
	}
> = {
	"no-setup": { quiet: "setup", run: noSetup },
	"no-publish-answer": { quiet: "publish", run: noPublishAnswer },
	"no-fetch-answer": { quiet: "requests", run: noFetchAnswer },
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

async function main(): Promise<void> {
	if (options.case === "relay-no-setup") {
		let summary: string;
		try {
			summary = await relayNoSetup();
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
