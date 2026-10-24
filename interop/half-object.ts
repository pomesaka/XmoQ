// Interop driver: a viewer comes while the own relay is receiving an Object,
// and the relay counts that Object from its first byte (§5.1.2).
//
// Usage: bun half-object.ts --case <case> [--relay own] [--port N]
//
// The publisher is `xmoq-quiet-peer --quiet half-object` (crates/quiet-peer):
// it publishes the track `half`, sends Object 0 of Group 0 whole, then the
// first half of Object 0 of Group 1 (512 KiB), and the rest after HOLD_MS.
// The viewer subscribes while that Object is half there, and sends a
// Relative Joining FETCH from the Group of its Joining Location.
//
//   rust-finish         own-rust, without a filter (`--mode subscribe --join
//                       relative:0`). The rest of the Object comes, then
//                       Object 1 of 16 bytes.
//   rust-largest        The same, with the Largest Object filter.
//   rust-reset          own-rust, without a filter. The publisher resets the
//                       stream in place of the rest, then sends Object 0 of
//                       Group 2.
//   page-finish         An own page, without a filter, as `rust-finish`.
//
// Written before the first run, from the notes ("LARGEST_OBJECT in
// SUBSCRIBE_OK", "A FETCH for an Object the relay is receiving", "The
// Subscription Filter of a SUBSCRIBE, in the relay"):
//
// - The relay holds the FETCH (`HOLD_FETCH_FOR_OBJECT session=1 stream=S
//   wait_ms=5000`).
// - `*-finish`, `rust-largest`: FETCH_OK with End Location {1, 1}, which says
//   that the Joining Location was {1, 0}; the fetch stream has {1, 0} alone,
//   of 524,288 bytes, and the relay wrote it from its cache
//   (`WRITE_FETCH_STREAM … pieces=1`) and asked the publisher nothing. The
//   answer comes after the rest of the Object, so not before HOLD_MS / 2
//   from when the first half was sent. The subscription brings {1, 1} and
//   not {1, 0}.
// - `rust-reset`: the relay asks the publisher for {1, 0} to {1, 1}
//   (`OPEN_REQUEST session=0 own=N FETCH`), the publisher refuses with
//   DOES_NOT_EXIST, and the client is refused with 0x10. The subscription
//   brings {2, 0} and not {1, 0}.

import { parseArgs } from "node:util";
import { startSubscriber } from "./clients.ts";
import {
	captureGroup,
	log,
	pollFor,
	runCleanups,
	STEP_TIMEOUT_MS,
	spawnQuietClient,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		case: { type: "string", default: "rust-finish" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24390" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const CASES = [
	"rust-finish",
	"rust-largest",
	"rust-reset",
	"page-finish",
] as const;
type Case = (typeof CASES)[number];

/** How long the publisher holds the second half of the Object back. */
const HOLD_MS = 2000;
/** The payload of the Object that comes in two halves. */
const HALF_OBJECT_BYTES = 512 * 1024;
const TRACK = "half";

/** The relay's line for a FETCH it holds for an Object it is receiving. */
const HELD = /^HOLD_FETCH_FOR_OBJECT session=1 stream=(\d+) wait_ms=(\d+)$/;

async function run(which: Case): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: which.startsWith("page-") ? ["own-browser"] : ["own-rust"],
		verbose: options.verbose,
	});
	const then = which === "rust-reset" ? "reset" : "finish";
	const publisher = spawnQuietClient(
		context,
		"publisher:quiet-peer",
		30_000,
		false,
		["--quiet", "half-object", "--then", then, "--hold-ms", String(HOLD_MS)],
	);
	await publisher.waitForLine(/^HALF_OBJECT_SENT group=1 object=0$/);
	const sentAt = Date.now();

	if (which === "page-finish") {
		const page = await startSubscriber(
			"own-browser",
			"viewer:own-browser",
			{ context, payloadHex: "" },
			{ track: TRACK },
		);
		const received = page.receivedOfGroup;
		if (received === undefined) throw new Error("the run needs an own page");
		const fetched = await page.fetch(
			{ type: "RelativeJoining", joiningStart: "0" },
			STEP_TIMEOUT_MS,
		);
		const answeredAfterMs = Date.now() - sentAt;
		const items = fetched.items.map((item) =>
			item.type === "object"
				? `{${item.group}, ${item.object}} ${item.payloadHex.length / 2}`
				: `${item.kind} {${item.group}, ${item.object}}`,
		);
		log(
			"driver",
			`REPORT response=${JSON.stringify(fetched.response)} items=[${items.join("; ")}] end=${JSON.stringify(fetched.end)}`,
		);
		if (
			!("endLocation" in fetched.response) ||
			fetched.response.endLocation.join(",") !== "1,1"
		) {
			throw new Error(
				`the Joining FETCH was to get FETCH_OK with End Location {1, 1}: ${JSON.stringify(fetched.response)}`,
			);
		}
		if (items.join("; ") !== `{1, 0} ${HALF_OBJECT_BYTES}`) {
			throw new Error(
				`the fetch stream was to have {1, 0} of ${HALF_OBJECT_BYTES} bytes alone: [${items.join("; ")}]`,
			);
		}
		await checkHeldAndAnswered(context, answeredAfterMs);
		const bySubscription = await pollFor(async () => {
			const objects = await received(1);
			return objects.length >= 1 ? objects : undefined;
		}, "the page to have Object 1 of Group 1 by its subscription");
		const ids = bySubscription.map((object) => object.object);
		if (ids.join(",") !== "1") {
			throw new Error(
				`the subscription was to bring {1, 1} and not {1, 0}; it brought Objects [${ids.join(", ")}] of Group 1`,
			);
		}
		return `the relay held the page's Joining FETCH for {1, 0}, answered it from its cache with End Location {1, 1} ${answeredAfterMs} ms after the first half was sent, and the subscription brought {1, 1} alone`;
	}

	const filter =
		which === "rust-largest" ? ["--subscription-filter", "largest-object"] : [];
	const client = spawnRustClient(
		context,
		"viewer:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		[
			"--join",
			"relative:0",
			"--objects",
			"1",
			...filter,
			"--timeout-secs",
			"20",
		],
	);
	const answer = client
		.waitForLine(/^FETCH_(?:OK|REFUSED) request_id=\d+ .*$/)
		.then((line) => ({ line, afterMs: Date.now() - sentAt }));
	const exit = await withTimeout(client.exited, "the viewer to exit");
	const { line: answered, afterMs } = await answer;
	const seen = client
		.matches(/^(?:FETCH_(?:OK|REFUSED|OBJECT|RANGE_END|ENDED)|OBJECT) .*$/)
		.map((match) =>
			match[0]
				.replace(/ payload_hex=\S*$/, "")
				.replace(/ track_properties=\S*/, ""),
		);
	log("driver", `REPORT ${seen.join("; ")}`);
	if (exit !== 0) {
		throw new Error(
			`the viewer exited with ${exit}, not 0: ${seen.join("; ")}`,
		);
	}
	const bySubscription = client
		.matches(/^OBJECT group=(\d+) object=(\d+) /)
		.map((match) => `{${captureGroup(match, 1)}, ${captureGroup(match, 2)}}`);

	if (which === "rust-reset") {
		if (!/^FETCH_REFUSED request_id=\d+ error_code=0x10$/.test(answered[0])) {
			throw new Error(
				`the Joining FETCH was to be refused with DOES_NOT_EXIST (0x10), the publisher's answer: ${seen.join("; ")}`,
			);
		}
		const fence = {
			line: /^SEND session=1 stream=\d+ REQUEST_ERROR error_code=0x10$/,
		};
		const held = await context.relay.linesBefore(fence, HELD);
		const asked = await context.relay.linesBefore(
			fence,
			/^OPEN_REQUEST session=0 own=\d+ FETCH/,
		);
		if (held.length !== 1 || asked.length !== 1) {
			throw new Error(
				`the relay was to hold the FETCH once and then ask the publisher once; it printed ${held.length} holds and ${asked.length} FETCHes to the publisher`,
			);
		}
		if (bySubscription.join("; ") !== "{2, 0}") {
			throw new Error(
				`the subscription was to bring {2, 0} and not {1, 0}: [${bySubscription.join("; ")}]`,
			);
		}
		return `the relay held the Joining FETCH for {1, 0}, asked the publisher when the stream was reset, and passed its DOES_NOT_EXIST on; the subscription brought {2, 0}`;
	}

	const ok = answered[0].match(
		/^FETCH_OK request_id=\d+ end_group=(\d+) end_object=(\d+) /,
	);
	if (ok === null || `${ok[1]},${ok[2]}` !== "1,1") {
		throw new Error(
			`the Joining FETCH was to get FETCH_OK with End Location {1, 1}: ${seen.join("; ")}`,
		);
	}
	const fetched = client
		.matches(
			/^FETCH_OBJECT request_id=\d+ group=(\d+) object=(\d+) .* payload_len=(\d+) /,
		)
		.map(
			(match) =>
				`{${captureGroup(match, 1)}, ${captureGroup(match, 2)}} ${captureGroup(match, 3)}`,
		);
	if (fetched.join("; ") !== `{1, 0} ${HALF_OBJECT_BYTES}`) {
		throw new Error(
			`the fetch stream was to have {1, 0} of ${HALF_OBJECT_BYTES} bytes alone: [${fetched.join("; ")}]`,
		);
	}
	if (
		!client.matches(/^FETCH_ENDED request_id=\d+ how=fin code=\S+ objects=1$/)
			.length
	) {
		throw new Error(
			`the fetch stream did not end with a FIN after 1 Object: ${seen.join("; ")}`,
		);
	}
	await checkHeldAndAnswered(context, afterMs);
	if (bySubscription.join("; ") !== "{1, 1}") {
		throw new Error(
			`the subscription was to bring {1, 1} and not {1, 0}: [${bySubscription.join("; ")}]`,
		);
	}
	return `the relay held the Joining FETCH for {1, 0}, answered it from its cache with End Location {1, 1} ${afterMs} ms after the first half was sent, and the subscription brought {1, 1} alone`;
}

/**
 * The relay held the FETCH, wrote one Object from its cache for it and asked
 * the publisher nothing; the answer came after the rest of the Object.
 */
async function checkHeldAndAnswered(
	context: Awaited<ReturnType<typeof startRun>>,
	answeredAfterMs: number,
): Promise<void> {
	const fence = {
		line: /^FETCH_WRITE_ENDED session=1 stream=\d+ finished pieces=1$/,
	};
	const held = await context.relay.linesBefore(fence, HELD);
	const [hold] = held;
	if (
		held.length !== 1 ||
		hold === undefined ||
		captureGroup(hold, 2) !== "5000"
	) {
		throw new Error(
			`the relay was to hold the FETCH once, for 5000 ms: [${held.map((line) => line[0]).join("; ")}]`,
		);
	}
	const written = await context.relay.linesBefore(
		fence,
		/^WRITE_FETCH_STREAM session=1 request_id=\d+ pieces=(\d+)$/,
	);
	const asked = await context.relay.linesBefore(
		fence,
		/^OPEN_REQUEST session=0 own=\d+ FETCH/,
	);
	if (written.length !== 1 || asked.length !== 0) {
		throw new Error(
			`the relay was to answer the FETCH from its cache and ask the publisher nothing; it wrote ${written.length} fetch streams and sent the publisher ${asked.length} FETCHes`,
		);
	}
	if (answeredAfterMs < HOLD_MS / 2) {
		throw new Error(
			`the FETCH was answered ${answeredAfterMs} ms after the first half of the Object was sent, before the rest was due (${HOLD_MS} ms)`,
		);
	}
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error("half-object.ts runs through the own relay");
	}
	const which = CASES.find((each) => each === options.case);
	if (which === undefined) {
		throw new Error(
			`--case "${options.case}" is not one of ${CASES.join(", ")}`,
		);
	}
	let summary: string;
	try {
		summary = await run(which);
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ case: which, relay: "own" })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.case} through the own relay: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
