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
// The cases that begin with `test-` have moqtail's test client as the
// publisher, which publishes by itself, three Objects a Group.
//
//   test-standalone       {0, 1} to {1, 2}.
//   test-join-relative    A Relative Joining FETCH, one Group back. Own
//                         relay only.
//
// What is expected is in `EXPECTED`, written before the runs.

import { parseArgs } from "node:util";
import {
	PAGE_GROUPS_BEFORE,
	publishFromAPage,
	startTestPublisher,
} from "./fetch-publishers.ts";
import {
	captureGroup,
	log,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "page-standalone" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "44370" },
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

async function main(): Promise<void> {
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
			// line when it has written the whole fetch stream, after
			// everything else about the FETCH.
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
			if (hold !== undefined) {
				const answers = await context.relay.linesBefore(
					fence,
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
