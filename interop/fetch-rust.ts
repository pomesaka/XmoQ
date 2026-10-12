// Interop driver: the own Rust client sends one FETCH through a relay, a
// Standalone one or a Joining one for its own subscription, and what it
// prints is compared with what is known of that relay and that publisher.
//
// Usage: bun fetch-rust.ts --fetch <case> [--relay moqtail|own] [--port N]
//
// The cases that begin with `page-` have an own page as the publisher, as in
// fetch.ts: it only announced, serves the track when the relay asks, has
// sent Object 0 of Groups 0, 1 and 2 to another subscriber, and answers a
// FETCH from what it has sent. Own relay only.
//
//   page-standalone       {0, 0} to {2, 1}.
//   page-join-relative    The client subscribes and, when it has its
//                         SUBSCRIBE_OK, sends a Relative Joining FETCH, one
//                         Group back.
//   page-join-absolute    The same, an Absolute Joining FETCH from Group 1.
//   page-join-at-once     The Relative one, sent right after the SUBSCRIBE,
//                         before its SUBSCRIBE_OK: the two travel on streams
//                         of their own, and the relay may read the FETCH
//                         first.
//
//   page-large-request-only
//                         The page sends one more Group, 8 Objects of
//                         900,000 bytes, which the relay then holds as the
//                         latest. The client asks for that Group, reads
//                         nothing of the fetch stream, and gives the FETCH
//                         up after a second **on the request stream alone**
//                         (`--fetch-cancel-request-only`): it resets and
//                         stops that stream and leaves the fetch stream as
//                         it is. Written before the first run, from the
//                         notes ("The relay's cache", "The subscriber gives
//                         its FETCH up"): the client has FETCH_OK with End
//                         Location {3, 8} and `FETCH_ENDED … how=cancelled …
//                         objects=0`, and exits with 0. The relay answers
//                         itself (`WRITE_FETCH_STREAM … pieces=8`, no FETCH
//                         to the publisher), is still writing when the
//                         request stream is reset, and stops by its own
//                         order: `ABORT_FETCH_WRITE session=S stream=T
//                         code=0x1` (**corrected after the first run**:
//                         expected once, and it came twice, for the reset of
//                         the request stream and for its stop; the run takes
//                         one or more), then `FETCH_WRITE_ENDED session=S
//                         stream=T aborted pieces=K code=0x1` with K below
//                         8. `stopped` in place of `aborted` would say that
//                         the client stopped the fetch stream as well.
//
// The cases that begin with `test-` have moqtail's test client as the
// publisher, which publishes by itself, three Objects a Group.
//
//   test-standalone       {0, 1} to {1, 2}.
//   test-join-relative    A Relative Joining FETCH, one Group back. Own
//                         relay only.
//
// What is expected is in `EXPECTED`, written before the runs.

import { parseArgs } from "node:util";
import { startSubscriber } from "./clients.ts";
import {
	PAGE_GROUPS_BEFORE,
	publishFromAPage,
	startTestPublisher,
} from "./fetch-publishers.ts";
import {
	captureGroup,
	log,
	pollFor,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "page-standalone" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24370" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const CASES = [
	"page-standalone",
	"page-join-relative",
	"page-join-absolute",
	"page-join-at-once",
	"test-standalone",
	"test-join-relative",
] as const;
type Case = (typeof CASES)[number];

/** The mode and the flags the own Rust client is started with. */
const CLIENT: Record<Case, { mode: "fetch" | "subscribe"; args: string[] }> = {
	"page-standalone": {
		mode: "fetch",
		args: ["--fetch-start", "0,0", "--fetch-end", "2,1"],
	},
	"page-join-relative": { mode: "subscribe", args: ["--join", "relative:1"] },
	"page-join-absolute": { mode: "subscribe", args: ["--join", "absolute:1"] },
	"page-join-at-once": {
		mode: "subscribe",
		args: ["--join", "relative:1", "--join-at-once"],
	},
	"test-standalone": {
		mode: "fetch",
		args: ["--fetch-start", "0,1", "--fetch-end", "1,2"],
	},
	"test-join-relative": { mode: "subscribe", args: ["--join", "relative:1"] },
};

/**
 * What the client has to print of its FETCH: FETCH_OK with this End
 * Location, these Objects as `{group, object}`, and the fetch stream
 * finished. `oneGroupBack` is for a publisher that goes on publishing: the
 * End Location is where the publisher was, {G, o}, and the Objects are the
 * three of Group G - 1 and the `o` first of Group G.
 */
type Expected =
	| { endLocation: [number, number]; objects: string[] }
	| "oneGroupBack";

/**
 * By relay and case; a case that is absent for a relay is not run there.
 *
 * The `page-` cases, own relay: as the cases of the same names in fetch.ts,
 * where the reasons are. The publishing page has Object 0 of Groups 0 to 2
 * when the client comes; the Joining Location of the client's subscription
 * is {2, 0}, the LARGEST_OBJECT of the SUBSCRIBE_OK the relay sends it (the
 * notes, "A Joining FETCH in the relay"), so both Joining FETCHes are for
 * {1, 0} to {2, 1}: one Group back, and from Group 1.
 *
 * `page-join-at-once`: the same answer. The notes, "A Joining FETCH that
 * names a Request ID the peer has not used yet": the relay holds a FETCH it
 * read before its SUBSCRIBE, and goes on with it when the SUBSCRIBE has
 * come. Which of the two the relay reads first is not the driver's to
 * choose; the run says which it was, by the relay's `HOLD_REQUEST` line.
 *
 * `test-standalone`, both relays: as `standalone` in fetch.ts.
 *
 * `test-join-relative`, own relay: the publisher goes on publishing, so the
 * Joining Location is where it was when the client subscribed, and the
 * range is one Group back from there (the same row of the notes). The
 * range ends inside a Group, which is where moqtail's publisher does as the
 * draft says (fetch.ts, `standalone`).
 */
const EXPECTED: Record<string, Partial<Record<Case, Expected>>> = {
	own: {
		"page-standalone": {
			endLocation: [2, 1],
			objects: ["{0, 0}", "{1, 0}", "{2, 0}"],
		},
		"page-join-relative": {
			endLocation: [2, 1],
			objects: ["{1, 0}", "{2, 0}"],
		},
		"page-join-absolute": {
			endLocation: [2, 1],
			objects: ["{1, 0}", "{2, 0}"],
		},
		"page-join-at-once": {
			endLocation: [2, 1],
			objects: ["{1, 0}", "{2, 0}"],
		},
		"test-standalone": {
			endLocation: [1, 2],
			objects: ["{0, 1}", "{0, 2}", "{1, 0}", "{1, 1}"],
		},
		"test-join-relative": "oneGroupBack",
	},
	moqtail: {
		"test-standalone": {
			endLocation: [1, 2],
			objects: ["{0, 1}", "{0, 2}", "{1, 0}", "{1, 1}"],
		},
	},
};

/** How many Objects a Group of moqtail's test client has. */
const TEST_OBJECTS_A_GROUP = 3;

/** The Objects of the large Group, and the bytes of each: 7.2 MB, under the 8 MiB the relay keeps of a track. The relay had written 1 of the 8 when the client gave up, in 7 runs of 7. */
const LARGE_GROUP = { objects: 8, bytes: 900_000 };
/** After how long the client gives its FETCH up, in milliseconds. */
const CANCEL_AFTER_MS = 1000;

/** `page-large-request-only`: nothing of the fetch stream is read, so the client prints no Object. */
async function largeGroupRequestOnly(): Promise<string> {
	const sentHex = Buffer.from("xmoq-fetch large").toString("hex");
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: ["own-browser", "own-browser", "own-rust"],
		verbose: options.verbose,
	});
	const publisher = await publishFromAPage("serve", context, sentHex);
	// Says when the relay has passed an Object on, and so holds it.
	const watcher = await startSubscriber(
		"own-browser",
		"watcher:own-browser",
		{ context, payloadHex: sentHex },
		{},
	);
	const watched = watcher.objectsOfGroup;
	if (publisher.sendLargeGroup === undefined || watched === undefined) {
		throw new Error(
			"the run needs own pages for the publisher and the watcher",
		);
	}
	const { objects, bytes } = LARGE_GROUP;
	const group = await withTimeout(
		publisher.sendLargeGroup(objects, bytes),
		"the publisher to send the large Group",
	);
	await pollFor(
		async () => ((await watched(group)) === objects ? true : undefined),
		`the watching page to have the ${objects} Objects of Group ${group} by its subscription`,
	);
	const client = spawnRustClient(
		context,
		"fetcher:own-rust",
		"own-rust",
		"fetch",
		TRACK,
		[
			"--fetch-start",
			`${group},0`,
			"--fetch-end",
			`${group + 1},0`,
			// Longer than the run: nothing of the fetch stream is read. The
			// stream is open when the FETCH is given up, which a stream that
			// came after that would not be: the session stops it.
			"--fetch-read-after-ms",
			"60000",
			"--fetch-cancel-after-ms",
			String(CANCEL_AFTER_MS),
			"--fetch-cancel-request-only",
			// The relay's lines for the end of its writing come while the
			// session is there.
			"--fetch-stay-ms",
			"3000",
			"--timeout-secs",
			"20",
		],
	);
	const ended = await client.waitForLine(
		/^FETCH_ENDED request_id=(\d+) how=(\S+) code=(\S+) objects=(\d+)$/,
	);
	const seen = `${client
		.matches(/^FETCH_(?:OK|REFUSED|OBJECT|RANGE_END|STREAM_REFUSED) .*$/)
		.map((match) => match[0])
		.join("; ")}; ${ended[0]}`;
	log("driver", `REPORT ${seen}`);
	const ok = client
		.matches(/^FETCH_OK request_id=\d+ end_group=(\d+) end_object=(\d+) /)
		.at(0);
	if (
		ok === undefined ||
		`${captureGroup(ok, 1)},${captureGroup(ok, 2)}` !== `${group},${objects}`
	) {
		throw new Error(
			`the FETCH got no FETCH_OK with End Location {${group}, ${objects}}: ${seen}`,
		);
	}
	if (
		captureGroup(ended, 2) !== "cancelled" ||
		captureGroup(ended, 4) !== "0"
	) {
		throw new Error(
			`the FETCH did not end as cancelled, without an Object read: ${seen}`,
		);
	}

	const written = await context.relay.waitForLine(
		/^WRITE_FETCH_STREAM session=(\d+) request_id=\d+ pieces=(\d+)$/,
	);
	if (captureGroup(written, 2) !== String(objects)) {
		throw new Error(
			`the relay wrote ${captureGroup(written, 2)} pieces, not the ${objects} Objects of the Group: ${written[0]}`,
		);
	}
	const session = captureGroup(written, 1);
	const ENDED = new RegExp(
		`^FETCH_WRITE_ENDED session=${session} stream=(\\d+) (\\S+) pieces=(\\d+)(?: code=(\\S+))?$`,
	);
	const end = await context.relay.waitForLine(ENDED);
	const [stream, how, pieces, code] = [1, 2, 3, 4].map((index) => end[index]);
	if (how !== "aborted" || code !== "0x1" || Number(pieces) >= objects) {
		throw new Error(
			`the relay's writing was to end by the relay's own order (aborted) with CANCELLED (0x1), before all ${objects} Objects were written: ${end[0]}`,
		);
	}
	const orders = await context.relay.linesBefore(
		{ line: ENDED },
		new RegExp(
			`^ABORT_FETCH_WRITE session=${session} stream=${stream} code=(\\S+)$`,
		),
	);
	// One for the reset of the request stream and one for its stop, where
	// both have come before the writing ended: the second does nothing.
	if (orders.length === 0 || orders.some((order) => order[1] !== "0x1")) {
		throw new Error(
			`the relay was to order its writing stopped with CANCELLED (0x1) before the writing ended; it printed [${orders.map((order) => order[0]).join("; ")}]`,
		);
	}
	const asked = await context.relay.linesBefore(
		{ line: ENDED },
		/^OPEN_REQUEST session=\d+ own=\d+ FETCH/,
	);
	if (asked.length > 0) {
		throw new Error(
			`the relay was to answer the FETCH itself, and sent one to a publisher: ${asked.map((match) => match[0]).join("; ")}`,
		);
	}
	const exit = await withTimeout(client.exited, "the client to exit");
	if (exit !== 0) {
		throw new Error(`the client exited with ${exit}, not 0`);
	}
	return `the relay answered with FETCH_OK for {${group}, ${objects}} and began to write ${objects} Objects of ${bytes} bytes; the client read none and gave the FETCH up on the request stream alone, and the relay stopped its writing by its own order, ${pieces} Objects written (${end[0]})`;
}

async function main(): Promise<void> {
	if (options.fetch === "page-large-request-only") {
		if (options.relay !== "own") {
			throw new Error(
				"--fetch page-large-request-only runs through the own relay",
			);
		}
		let summary: string;
		try {
			summary = await largeGroupRequestOnly();
		} finally {
			await runCleanups();
		}
		log(
			"driver",
			`RESULT ${JSON.stringify({ fetch: options.fetch, relay: options.relay })}`,
		);
		log("driver", `SUMMARY ${summary}`);
		return;
	}
	const which = CASES.find((each) => each === options.fetch);
	if (which === undefined) {
		throw new Error(
			`--fetch "${options.fetch}" is not one of ${CASES.join(", ")}`,
		);
	}
	const expected = EXPECTED[options.relay]?.[which];
	if (expected === undefined) {
		throw new Error(
			`--fetch ${which} is not run through the relay "${options.relay}"`,
		);
	}
	const fromAPage = which.startsWith("page-");
	// What the publishing page sends as its Objects.
	const sentHex = Buffer.from(`xmoq-fetch ${which}`).toString("hex");
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: fromAPage ? ["own-browser", "own-rust"] : ["own-rust"],
			verbose: options.verbose,
		});
		if (fromAPage) await publishFromAPage("serve", context, sentHex);
		else await startTestPublisher(context);
		// The page is session 0 and its first subscriber 1; moqtail's test
		// client is session 0.
		const session = fromAPage ? 2 : 1;

		const { mode, args } = CLIENT[which];
		const client = spawnRustClient(
			context,
			"fetcher:own-rust",
			"own-rust",
			mode,
			TRACK,
			[...args, "--timeout-secs", "20"],
		);
		const ended = await client.waitForLine(
			/^FETCH_ENDED request_id=(\d+) how=(\S+) code=(\S+) objects=(\d+)$/,
		);
		const id = captureGroup(ended, 1);
		const seen = `${client
			.matches(/^FETCH_(?:OK|REFUSED|OBJECT|RANGE_END|STREAM_REFUSED) .*$/)
			.map((match) => match[0])
			.join("; ")}; ${ended[0]}`;
		const ok = client
			.matches(
				new RegExp(
					`^FETCH_OK request_id=${id} end_group=(\\d+) end_object=(\\d+) `,
				),
			)
			.at(0);
		if (ok === undefined) {
			throw new Error(`the FETCH got no FETCH_OK: ${seen}`);
		}
		const end: [number, number] = [
			Number(captureGroup(ok, 1)),
			Number(captureGroup(ok, 2)),
		];
		const objects = client.matches(
			new RegExp(
				`^FETCH_OBJECT request_id=${id} group=(\\d+) object=(\\d+) subgroup=\\S+ properties=\\S+ payload_len=(\\d+) payload_hex=(\\S*)$`,
			),
		);
		const wanted =
			expected === "oneGroupBack"
				? {
						endLocation: end,
						objects: [
							...Array.from(
								{ length: TEST_OBJECTS_A_GROUP },
								(_unused, object) => `{${end[0] - 1}, ${object}}`,
							),
							...Array.from(
								{ length: end[1] },
								(_unused, object) => `{${end[0]}, ${object}}`,
							),
						],
					}
				: expected;
		if (expected === "oneGroupBack" && (end[0] < 1 || end[1] < 1)) {
			throw new Error(
				`FETCH_OK has the End Location {${end}}, which is no Location after an Object of a Group past the first: ${seen}`,
			);
		}
		if (end.join(",") !== wanted.endLocation.join(",")) {
			throw new Error(
				`FETCH_OK has the End Location {${end}}, not {${wanted.endLocation}}: ${seen}`,
			);
		}
		const got = objects.map(
			(match) => `{${captureGroup(match, 1)}, ${captureGroup(match, 2)}}`,
		);
		if (got.join("; ") !== wanted.objects.join("; ")) {
			throw new Error(
				`the fetch stream had [${got.join("; ")}], not [${wanted.objects.join("; ")}]: ${seen}`,
			);
		}
		// The page's Objects carry what the driver gave it; the test client's
		// are 8 bytes, the same in every Object.
		const payloads = new Set(objects.map((match) => captureGroup(match, 4)));
		const [payload, ...others] = payloads;
		const right = fromAPage ? payload === sentHex : payload?.length === 16;
		if (!right || others.length > 0) {
			throw new Error(
				`the Objects do not carry the publisher's payload${fromAPage ? ` ${sentHex}` : ", 8 bytes that are the same in each"}: ${seen}`,
			);
		}
		if (
			captureGroup(ended, 2) !== "fin" ||
			captureGroup(ended, 4) !== String(wanted.objects.length)
		) {
			throw new Error(
				`the FETCH did not end with a finished fetch stream and ${wanted.objects.length} Objects: ${seen}`,
			);
		}
		log("driver", `REPORT ${seen}`);
		summary = `FETCH_OK with End Location {${end}} and [${got.join("; ")}] on the fetch stream`;
		if (which === "page-join-at-once") {
			// The relay prints the line when it holds a FETCH whose SUBSCRIBE
			// it has not read. Whether it did is a matter of which stream it
			// read first.
			//
			// WHY a fence: the client's lines and the relay's reach the driver
			// in no order, so what the relay printed "by now" depends on when
			// the driver looks (harness.ts, `RelayLog`). The relay prints this
			// line when it has written the whole fetch stream, which is after
			// it held the FETCH or did not.
			const fence = {
				line: new RegExp(
					`^FETCH_PIPE_ENDED session=${session} stream=\\d+ finished$`,
				),
			};
			const [hold] = await context.relay.linesBefore(
				fence,
				new RegExp(`^HOLD_REQUEST session=${session} stream=(\\d+) FETCH$`),
			);
			const held = hold !== undefined;
			// A FETCH that was held is answered on the stream it was held on.
			//
			// WHY another fence for the answer: the publisher's FETCH_OK and
			// its fetch stream are two streams, which the relay reads in no
			// order, and a publisher may send its Objects before its FETCH_OK
			// (§10.13). So the relay may have passed the whole fetch stream
			// on, and printed the line above, before it reads the FETCH_OK
			// and prints that it sent one. It finishes its direction of the
			// request stream when both are done.
			if (hold !== undefined) {
				const answered = {
					line: new RegExp(
						`^FINISH_REQUEST session=${session} stream=${captureGroup(hold, 1)}$`,
					),
				};
				const answers = await context.relay.linesBefore(
					answered,
					new RegExp(
						`^SEND session=${session} stream=${captureGroup(hold, 1)} FETCH_OK$`,
					),
				);
				if (answers.length !== 1) {
					throw new Error(
						`the relay held the FETCH (${hold[0]}) and printed ${answers.length} FETCH_OK for that stream, not 1`,
					);
				}
			}
			summary += held
				? "; the relay read the FETCH first and held it for its SUBSCRIBE"
				: "; the relay read the SUBSCRIBE first";
			log("driver", `ORDER ${held ? "fetch-first" : "subscribe-first"}`);
		}
		if (fromAPage && got.length > PAGE_GROUPS_BEFORE) {
			throw new Error("more Objects than the page had sent");
		}
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ fetch: which, relay: options.relay })}`,
	);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.fetch} FETCH of own-rust through the ${options.relay} relay: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
