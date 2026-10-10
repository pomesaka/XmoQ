// Interop driver: an own page sends one FETCH through a relay, and what it
// gets is compared with what is known of that relay and that publisher.
//
// Usage: bun fetch.ts --fetch <case> [--relay moqtail|own] [--port N]
//
// Each case has its publisher, the one that makes the case what it is:
//
//   standalone        moqtail's test client, which answers a FETCH. The page
//                     asks for {0, 1} to {1, 2} of the track it publishes.
//   fill-timeout-0    The same publisher, the range {0, 0} to {2, 0}, with
//                     FILL_TIMEOUT 0. Own relay only.
//   cancelled         The moqtail-rust client, which answers no FETCH. The
//                     page gives the FETCH up after a second. Own relay only.
//   publisher-refuses The own Rust client, which refuses a FETCH.
//   absent-track      A Standalone FETCH for a track nobody published.
//
// What is expected is in `EXPECTED`, written before the runs from the notes
// (docs/moqt-implementation-notes.md) for the own relay, and from the records
// of the moqtail relay and of moqtail's test client for the rest; where a
// run corrected it, the correction has its reason next to it.

import { parseArgs } from "node:util";
import { type Publisher, startPublisher, startSubscriber } from "./clients.ts";
import {
	type Client,
	captureGroup,
	type LoggedProcess,
	log,
	NAMESPACE,
	type RunContext,
	runCleanups,
	spawnMoqtailTestPublisher,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { PageFetch, PageFetchReport } from "./xmoq-page.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "standalone" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "44360" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The FETCH is cancelled when it is not over by then, and the report says so. */
const FETCH_TIMEOUT_MS = 5_000;
/** The page gives the FETCH of `cancelled` up after this long, well inside the 5 seconds the own relay waits for a publisher's answer. */
const CANCEL_AFTER_MS = 1_000;
/**
 * How many Groups moqtail's test client publishes, three Objects each, four
 * a second: 30 seconds, of which a run takes a few.
 */
const TEST_PUBLISHER_GROUPS = 40;

const CASES = [
	"standalone",
	"fill-timeout-0",
	"cancelled",
	"publisher-refuses",
	"absent-track",
] as const;
type Case = (typeof CASES)[number];

/** Who publishes in a case: a client of the harness, or moqtail's test client. */
const PUBLISHERS: Record<Case, Client | "moqtail-test"> = {
	standalone: "moqtail-test",
	"fill-timeout-0": "moqtail-test",
	cancelled: "moqtail-rust",
	"publisher-refuses": "own-rust",
	"absent-track": "own-rust",
};

const FETCHES: Record<Case, PageFetch> = {
	standalone: {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "1"],
		end: ["1", "2"],
	},
	"fill-timeout-0": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["2", "0"],
		fillTimeout: "0",
	},
	cancelled: {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["1", "0"],
	},
	"publisher-refuses": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["1", "0"],
	},
	"absent-track": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: "nobody-published-this",
		start: ["0", "0"],
		end: ["1", "0"],
	},
};

/**
 * What the page has to report. `items` is what the fetch stream decodes to,
 * each as `{group, object}` for an Object that has the payload of the
 * publisher's Objects, or as `<kind> to {group, object}` for an End of Range
 * indicator.
 */
type Expected =
	| { answer: "FETCH_OK"; endLocation: [string, string]; items: string[] }
	| { answer: "REQUEST_ERROR"; errorCode: string }
	| { answer: "cancelled" };

/** 2^64 - 1, the Object ID of the last Location of a Group. */
const LAST_OBJECT = "18446744073709551615";

/**
 * By relay and case; a case that is absent for a relay is not run there.
 *
 * `standalone`, both relays: FETCH_OK with End Location {1, 2} and the four
 * Objects of the range. Through the own relay it is the publisher's answer,
 * passed on (run 1 of docs/experiments/2026-10-11-standalone-fetch-through-own-relay.md);
 * through the moqtail relay it is capture I and K of fixtures/moqt-18. The
 * range ends inside a Group and is asked in Ascending order, so that it does
 * not touch the two places where moqtail's publisher leaves the draft (a
 * Group named by Object 0 as the end is left out; Descending gives the first
 * Group alone).
 *
 * `fill-timeout-0`, own relay: the notes, "FILL_TIMEOUT 0 in a FETCH":
 * "Nothing is asked of a publisher. For a track the relay has a source of
 * and knows a largest Location L for: FETCH_OK with [...] the End Location
 * that was asked for [...] and a fetch stream with one End of Unknown Range
 * indicator at the last Location of the range ({G, 2^64 - 1} for an End
 * Location {G, 0})". The run waits until the publisher is past Group 2, so
 * that the End Location asked for is not beyond L.
 *
 * `cancelled`, own relay: the notes, "The subscriber gives its FETCH up":
 * "the relay's FETCH is cancelled".
 *
 * `publisher-refuses`: the own Rust client refuses a FETCH with
 * NOT_SUPPORTED. The own relay: "`DOES_NOT_EXIST` for `NOT_SUPPORTED`" (the
 * notes, "What the subscriber of a FETCH gets from the publisher's answer").
 * The moqtail relay then answers from what it holds: FETCH_OK with the
 * Location after the last Object it has, and that Object
 * (docs/experiments/2026-10-11-fetch-from-a-page.md).
 *
 * `absent-track`: DOES_NOT_EXIST from both. The own relay: the notes, "Whom
 * a Standalone FETCH is asked of", "With neither: REQUEST_ERROR
 * `DOES_NOT_EXIST`". The moqtail relay: run 11 of its record.
 */
const EXPECTED: Record<string, Partial<Record<Case, Expected>>> = {
	own: {
		standalone: {
			answer: "FETCH_OK",
			endLocation: ["1", "2"],
			items: ["{0, 1}", "{0, 2}", "{1, 0}", "{1, 1}"],
		},
		"fill-timeout-0": {
			answer: "FETCH_OK",
			endLocation: ["2", "0"],
			items: [`Unknown to {2, ${LAST_OBJECT}}`],
		},
		cancelled: { answer: "cancelled" },
		"publisher-refuses": { answer: "REQUEST_ERROR", errorCode: "16" },
		"absent-track": { answer: "REQUEST_ERROR", errorCode: "16" },
	},
	moqtail: {
		standalone: {
			answer: "FETCH_OK",
			endLocation: ["1", "2"],
			items: ["{0, 1}", "{0, 2}", "{1, 0}", "{1, 1}"],
		},
		"publisher-refuses": {
			answer: "FETCH_OK",
			endLocation: ["0", "1"],
			items: ["{0, 0}"],
		},
		"absent-track": { answer: "REQUEST_ERROR", errorCode: "16" },
	},
};

/** Throws with what differs, unless the report is what `expected` has. */
function compare(
	report: PageFetchReport,
	expected: Expected,
	payloadHex: string,
): void {
	const seen = JSON.stringify(report);
	if (expected.answer === "cancelled") {
		if (
			report.end.type !== "cancelled" ||
			"endLocation" in report.response ||
			report.items.length > 0
		) {
			throw new Error(
				`the FETCH that was given up did not end as cancelled, without an answer and without Objects: ${seen}`,
			);
		}
		return;
	}
	if (expected.answer === "REQUEST_ERROR") {
		if (
			!("errorCode" in report.response) ||
			report.response.errorCode !== expected.errorCode
		) {
			throw new Error(
				`the FETCH was not refused with REQUEST_ERROR ${expected.errorCode}: ${seen}`,
			);
		}
		if (report.end.type !== "rejected" || report.items.length > 0) {
			throw new Error(
				`the FETCH that was refused did not end as rejected and without Objects: ${seen}`,
			);
		}
		return;
	}
	if (!("endLocation" in report.response)) {
		throw new Error(`the FETCH got no FETCH_OK: ${seen}`);
	}
	if (
		report.response.endLocation.join(",") !== expected.endLocation.join(",")
	) {
		throw new Error(
			`FETCH_OK has the End Location {${report.response.endLocation}}, not {${expected.endLocation}}: ${seen}`,
		);
	}
	const items = report.items.map((item) => {
		if (item.type === "end-of-range") {
			return `${item.kind} to {${item.group}, ${item.object}}`;
		}
		// An Object with another payload than the publisher's shows as it is.
		return item.payloadHex === payloadHex
			? `{${item.group}, ${item.object}}`
			: JSON.stringify(item);
	});
	if (items.join("; ") !== expected.items.join("; ")) {
		throw new Error(
			`the fetch stream had [${items.join("; ")}], not [${expected.items.join("; ")}] with the payload ${payloadHex}: ${seen}`,
		);
	}
	if (report.bodyEnd !== "closed" || report.end.type !== "served") {
		throw new Error(
			`the FETCH did not end with its answer and a finished fetch stream: ${seen}`,
		);
	}
}

/**
 * Starts the publisher of the case. Returns the client of the harness that
 * sends one Object when it is told to, or moqtail's test client, which has
 * published past Group 2 by then: the ranges of its cases are behind it.
 */
async function publish(
	which: Case,
	context: RunContext,
	payloadHex: string,
): Promise<
	| { told: Publisher; byItself?: undefined }
	| { told?: undefined; byItself: LoggedProcess }
> {
	const kind = PUBLISHERS[which];
	if (kind !== "moqtail-test") {
		return { told: await startPublisher(kind, { context, payloadHex }, {}) };
	}
	const publisher = spawnMoqtailTestPublisher(
		context,
		"pub:moqtail-test",
		TEST_PUBLISHER_GROUPS,
	);
	await publisher.waitForLine(/Track published/);
	await publisher.waitForLine(/Stream flushed for group 2$/);
	return { byItself: publisher };
}

/**
 * A Group of moqtail's test client that the page, subscribed by now, gets
 * from its first Object on: the second after the one the publisher is at.
 */
function groupStillToCome(publisher: LoggedProcess): number {
	const opened = publisher.matches(/Opening stream for group (\d+)$/);
	const latest = opened.at(-1);
	if (latest === undefined) throw new Error("the publisher has sent nothing");
	return Number(captureGroup(latest, 1)) + 2;
}

/** The own relay cancelled the FETCH it had sent the publisher, session 0, for the page's. */
async function expectUpstreamCancelled(context: RunContext): Promise<void> {
	// The publisher is the first client of the run, so its session is 0.
	const asked = await context.relay.waitForLine(
		/^OPEN_REQUEST session=0 own=(\d+) FETCH/,
	);
	const own = captureGroup(asked, 1);
	const cancelled = await context.relay.waitForLine(
		new RegExp(`^CANCEL_OWN_REQUEST session=0 own=${own} code=(\\S+)$`),
	);
	if (captureGroup(cancelled, 1) !== "0x1") {
		throw new Error(
			`the relay cancelled its FETCH with ${captureGroup(cancelled, 1)}, not with CANCELLED (0x1): ${cancelled[0]}`,
		);
	}
	// The publisher never answered, so nothing but the cancel ended the
	// relay's request: no answer of the publisher stands before it.
	const answers = await context.relay.linesBefore(
		{ line: new RegExp(`^CANCEL_OWN_REQUEST session=0 own=${own} `) },
		new RegExp(`^RECV session=0 own=${own} `),
	);
	if (answers.length > 0) {
		throw new Error(
			`the publisher answered the relay's FETCH before the page gave its own up, so the run does not show a cancel: ${answers.map((match) => match[0]).join("; ")}`,
		);
	}
}

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
	const kind = PUBLISHERS[which];
	// What a publisher of the harness sends as its one Object.
	const sentHex = Buffer.from(`xmoq-fetch ${which}`).toString("hex");
	let report: PageFetchReport;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients:
				kind === "moqtail-test" ? ["own-browser"] : [kind, "own-browser"],
			verbose: options.verbose,
		});
		const publisher = await publish(which, context, sentHex);
		const page = await startSubscriber(
			"own-browser",
			"fetcher:own-browser",
			{ context, payloadHex: sentHex },
			{},
		);
		const group =
			publisher.told === undefined
				? groupStillToCome(publisher.byItself)
				: await withTimeout(publisher.told.send(), "the publisher to send");
		// The page has an Object by its subscription, so the relay has the
		// track, and the payload of the publisher's Objects is known.
		const payloadHex = await page.object(group);
		report = await page.fetch(
			FETCHES[which],
			which === "cancelled" ? CANCEL_AFTER_MS : FETCH_TIMEOUT_MS,
		);
		log("driver", `REPORT ${JSON.stringify(report)}`);
		compare(report, expected, payloadHex);
		if (which === "cancelled") await expectUpstreamCancelled(context);
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ fetch: which, relay: options.relay, publisher: kind, expected, report })}`,
	);
	const summary =
		expected.answer === "FETCH_OK"
			? `FETCH_OK with End Location {${expected.endLocation}} and [${expected.items.join("; ")}] on the fetch stream`
			: expected.answer === "REQUEST_ERROR"
				? `REQUEST_ERROR ${expected.errorCode}`
				: "the page gave the FETCH up, and the relay cancelled the one it had sent the publisher";
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.fetch} FETCH through the ${options.relay} relay: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
