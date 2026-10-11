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
// The cases that begin with `page-` have an own page as the publisher,
// which announced the namespace, serves the track when the relay asks for
// it, and answers a FETCH from the Objects it has sent (`serve` of
// `@xmoq/moqt`). It has sent Object 0 of Groups 0, 1 and 2 to another
// subscriber when the page that fetches comes. Own relay only.
//
//   page-standalone   {0, 0} to {2, 1}: the three Objects.
//   page-joining      A Relative Joining FETCH, one Group back: the relay
//                     makes a Standalone FETCH of it for the publisher.
//   page-nothing-there  {0, 1} to the end of Group 0, where the page has
//                     nothing: one End of Range indicator.
//   page-unsent       {100, 0} to {102, 1}: Objects the page has and never
//                     sent, so that a relay has to ask the page for them.
//                     The page sends Groups 103, 104 and 105 here.
//                     Through the moqtail relay as well.
//   page-cancelled    The publisher accepts and writes nothing; the page
//                     that fetches gives the FETCH up after a second.
//   page-refuses      The publisher refuses with DOES_NOT_EXIST.
//
// What is expected is in `EXPECTED`, written before the runs from the notes
// (docs/moqt-implementation-notes.md) for the own relay, and from the records
// of the moqtail relay and of moqtail's test client for the rest; where a
// run corrected it, the correction has its reason next to it.

import { parseArgs } from "node:util";
import { type Publisher, startPublisher, startSubscriber } from "./clients.ts";
import { publishFromAPage, startTestPublisher } from "./fetch-publishers.ts";
import {
	type Client,
	captureGroup,
	type LoggedProcess,
	log,
	NAMESPACE,
	pollFor,
	type RunContext,
	runCleanups,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type {
	PageFetch,
	PageFetchReport,
	PageFetchServing,
} from "./xmoq-page.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "standalone" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "44360" },
		verbose: { type: "boolean", default: false },
		// Prints what the page saw and compares it with nothing: for looking
		// at what a relay does with a case that has no expectation for it.
		"report-only": { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The FETCH is cancelled when it is not over by then, and the report says so. */
const FETCH_TIMEOUT_MS = 5_000;
/** The page gives the FETCH of `cancelled` up after this long, well inside the 5 seconds the own relay waits for a publisher's answer. */
const CANCEL_AFTER_MS = 1_000;
const CASES = [
	"standalone",
	"fill-timeout-0",
	"cancelled",
	"publisher-refuses",
	"absent-track",
	"page-standalone",
	"page-joining",
	"page-nothing-there",
	"page-unsent",
	"page-cancelled",
	"page-refuses",
] as const;
type Case = (typeof CASES)[number];

/** Who publishes in a case: a client of the harness, or moqtail's test client. */
const PUBLISHERS: Record<Case, Client | "moqtail-test"> = {
	standalone: "moqtail-test",
	"fill-timeout-0": "moqtail-test",
	cancelled: "moqtail-rust",
	"publisher-refuses": "own-rust",
	"absent-track": "own-rust",
	"page-standalone": "own-browser",
	"page-joining": "own-browser",
	"page-nothing-there": "own-browser",
	"page-unsent": "own-browser",
	"page-cancelled": "own-browser",
	"page-refuses": "own-browser",
};

/** What the publishing page of a `page-` case does with a FETCH. */
const PAGE_SERVES: Partial<Record<Case, PageFetchServing>> = {
	"page-standalone": "serve",
	"page-joining": "serve",
	"page-nothing-there": "serve",
	"page-unsent": "serve-unsent",
	"page-cancelled": "stall",
	"page-refuses": "refuse",
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
	"page-standalone": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["2", "1"],
	},
	"page-joining": { type: "RelativeJoining", joiningStart: "1" },
	"page-nothing-there": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "1"],
		end: ["0", "0"],
	},
	"page-unsent": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["100", "0"],
		end: ["102", "1"],
	},
	"page-cancelled": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["2", "1"],
	},
	"page-refuses": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["2", "1"],
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
	| { answer: "cancelled" }
	/** Given up after its FETCH_OK came, and before any Object. */
	| { answer: "cancelled-after-FETCH_OK"; endLocation: [string, string] }
	/**
	 * What the page that fetches saw is not looked at beyond its FETCH_OK:
	 * the relay has to say that the publisher accepted its FETCH with
	 * `publisherEndLocation`, and the publishing page that it served one
	 * FETCH whole. `endLocation` is the one of the FETCH_OK the relay sends
	 * the page that fetches, before the publisher has answered.
	 */
	| {
			answer: "served-to-the-relay";
			endLocation: [string, string];
			publisherEndLocation: [string, string];
	  };

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
 *
 * The `page-` cases, own relay. The publishing page has Object 0 of Groups
 * 0 to 2, and of Group 3 by the time of the FETCH. It answers FETCH_OK with
 * the End Location it was asked with where it has an Object at or after the
 * end of the range, and the notes have the relay pass that on ("What the
 * subscriber of a FETCH gets from the publisher's answer": "FETCH_OK with
 * the publisher's End Of Track, End Location and Track Properties").
 *
 * `page-standalone`: {0, 0} to {2, 1} is Object 0 of Groups 0, 1 and 2.
 *
 * `page-joining`: the notes, "A Joining FETCH in the relay": "The Joining
 * Location is the LARGEST_OBJECT of the SUBSCRIBE_OK the relay sent for the
 * subscription the FETCH names", which is {2, 0}, the largest the relay had
 * seen when the page that fetches subscribed. "End Location: {Joining
 * Location.Group, Joining Location.Object + 1}", so {2, 1}; "Start Location:
 * {Joining Location.Group - Joining Start, 0}", so {1, 0}: Object 0 of
 * Groups 1 and 2.
 *
 * `page-nothing-there`: {0, 1} to the end of Group 0. The page has Object 0
 * of that Group alone, and says of the rest that it does not exist, with
 * one End of Range indicator at the last Location of the range; the relay
 * moves the bytes of a fetch stream as they are (the notes, "The
 * publisher's fetch stream").
 *
 * `page-unsent`: {100, 0} to {102, 1} is Object 0 of three Groups the page
 * has and never sent, with the payload of its other Objects. The own relay
 * asks the publisher for every range (it has no cache).
 *
 * `page-unsent`, moqtail relay: written after a first run, which showed
 * what that relay does (docs/experiments/2026-10-11-a-page-answers-a-fetch.md).
 * It asks the publisher, with the whole of Group 102 as the end and
 * GROUP_ORDER Ascending, takes the page's FETCH_OK and its fetch stream,
 * and answers the page that fetches with FETCH_OK for {102, 1}. The page's
 * own FETCH_OK has {102, 0}, the End Location the relay asked with: the
 * page has sent Groups 103 and on, so the end of Group 102 is not beyond
 * its Largest Object (§10.13). **It then
 * sends that page a fetch stream whose FETCH_HEADER has the Request ID of
 * the relay's own FETCH**, not of the page's, which `@xmoq/moqt` closes the
 * session for (the notes, "A fetch stream whose Request ID is no open FETCH
 * of the receiver"). So the run checks the publisher's side alone: what the
 * relay says of the publisher's answer, and that the publishing page served
 * its FETCH whole.
 *
 * `page-cancelled`: the publisher accepts, so the page that fetches has a
 * FETCH_OK with the End Location it asked with, and then gives up. The
 * notes, "The subscriber gives its FETCH up": "the relay's FETCH is
 * cancelled, the publisher's stream stopped". The publishing page then
 * says that the FETCH it served ended as cancelled by the peer.
 *
 * `page-refuses`: the page refuses with DOES_NOT_EXIST, which the relay
 * passes on as it is (the same row: "with the publisher's code where the
 * code means the same one hop on").
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
		"page-standalone": {
			answer: "FETCH_OK",
			endLocation: ["2", "1"],
			items: ["{0, 0}", "{1, 0}", "{2, 0}"],
		},
		"page-joining": {
			answer: "FETCH_OK",
			endLocation: ["2", "1"],
			items: ["{1, 0}", "{2, 0}"],
		},
		"page-nothing-there": {
			answer: "FETCH_OK",
			endLocation: ["0", "0"],
			items: [`NonExistent to {0, ${LAST_OBJECT}}`],
		},
		"page-unsent": {
			answer: "FETCH_OK",
			endLocation: ["102", "1"],
			items: ["{100, 0}", "{101, 0}", "{102, 0}"],
		},
		"page-cancelled": {
			answer: "cancelled-after-FETCH_OK",
			endLocation: ["2", "1"],
		},
		"page-refuses": { answer: "REQUEST_ERROR", errorCode: "16" },
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
		"page-unsent": {
			answer: "served-to-the-relay",
			endLocation: ["102", "1"],
			// The whole of Group 102, as the relay asks: the page has sent
			// Groups after it, so that is not beyond its Largest Object, and
			// the End Location that was asked for is the answer (§10.13).
			publisherEndLocation: ["102", "0"],
		},
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
	if (expected.answer === "served-to-the-relay") {
		if (
			!("endLocation" in report.response) ||
			report.response.endLocation.join(",") !== expected.endLocation.join(",")
		) {
			throw new Error(
				`the FETCH got no FETCH_OK for {${expected.endLocation}}: ${seen}`,
			);
		}
		return;
	}
	if (expected.answer === "cancelled-after-FETCH_OK") {
		if (
			report.end.type !== "cancelled" ||
			!("endLocation" in report.response) ||
			report.response.endLocation.join(",") !==
				expected.endLocation.join(",") ||
			report.items.length > 0
		) {
			throw new Error(
				`the FETCH that was given up did not end as cancelled, with FETCH_OK for {${expected.endLocation}} and without Objects: ${seen}`,
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
	return { byItself: await startTestPublisher(context) };
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

/**
 * The own relay cancelled the FETCH it had sent the publishing page, after
 * the page's FETCH_OK, and the page says that the FETCH it served ended as
 * cancelled by the peer.
 */
async function expectServedFetchCancelled(
	context: RunContext,
	publisher: Publisher,
): Promise<void> {
	// The publisher is the first client of the run, so its session is 0.
	const asked = await context.relay.waitForLine(
		/^OPEN_REQUEST session=0 own=(\d+) FETCH/,
	);
	const own = captureGroup(asked, 1);
	const cancel = new RegExp(
		`^CANCEL_OWN_REQUEST session=0 own=${own} code=0x1$`,
	);
	const answers = await context.relay.linesBefore(
		{ line: cancel },
		new RegExp(`^RECV session=0 own=${own} FETCH_OK`),
	);
	if (answers.length !== 1) {
		throw new Error(
			`the relay had ${answers.length} FETCH_OK of the publisher before it cancelled its FETCH, not 1`,
		);
	}
	const ends = await pollFor(async () => {
		const seen = await publisher.servedFetchEnds();
		return seen.length > 0 ? seen : undefined;
	}, "the publishing page to say how the FETCH it served ended");
	const [end, ...more] = ends;
	if (
		end === undefined ||
		more.length > 0 ||
		!end.includes('"peer-cancelled"')
	) {
		throw new Error(
			`the publishing page did not see one FETCH end as cancelled by the peer: ${ends.join("; ")}`,
		);
	}
	log("driver", `the publishing page: ${end}`);
}

/**
 * The moqtail relay says that the publisher accepted the FETCH it sent it,
 * with this End Location, and the publishing page says that it served one
 * FETCH whole. "Served" is the page's: its `close()` was taken. That the
 * Objects reached the relay is not looked at.
 */
async function expectServedToTheMoqtailRelay(
	context: RunContext,
	publisher: Publisher,
	[group, object]: [string, string],
): Promise<void> {
	await context.relay.waitForLine(
		new RegExp(
			`Upstream FETCH \\d+ accepted, end location Location \\{ group: ${group}, object: ${object} \\}`,
		),
	);
	const ends = await pollFor(async () => {
		const seen = await publisher.servedFetchEnds();
		return seen.length > 0 ? seen : undefined;
	}, "the publishing page to say how the FETCH it served ended");
	if (ends.join("; ") !== '{"type":"served"}') {
		throw new Error(
			`the publishing page did not serve one FETCH whole: ${ends.join("; ")}`,
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
	const reportOnly = options["report-only"];
	const expected: Expected | undefined = reportOnly
		? { answer: "cancelled" }
		: EXPECTED[options.relay]?.[which];
	if (expected === undefined) {
		throw new Error(
			`--fetch ${which} is not run through the relay "${options.relay}"`,
		);
	}
	const kind = PUBLISHERS[which];
	const served = PAGE_SERVES[which];
	// What a publisher of the harness sends as its one Object.
	const sentHex = Buffer.from(`xmoq-fetch ${which}`).toString("hex");
	let report: PageFetchReport;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients:
				kind === "moqtail-test"
					? ["own-browser"]
					: [
							kind,
							"own-browser",
							...(served === undefined ? [] : ["own-rust" as const]),
						],
			verbose: options.verbose,
		});
		const publisher =
			served === undefined
				? await publish(which, context, sentHex)
				: { told: await publishFromAPage(served, context, sentHex) };
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
			which === "cancelled" || which === "page-cancelled"
				? CANCEL_AFTER_MS
				: FETCH_TIMEOUT_MS,
		);
		log("driver", `REPORT ${JSON.stringify(report)}`);
		if (!reportOnly) compare(report, expected, payloadHex);
		if (which === "cancelled" && !reportOnly) {
			await expectUpstreamCancelled(context);
		}
		if (
			which === "page-cancelled" &&
			!reportOnly &&
			publisher.told !== undefined
		) {
			await expectServedFetchCancelled(context, publisher.told);
		}
		if (
			expected.answer === "served-to-the-relay" &&
			publisher.told !== undefined
		) {
			await expectServedToTheMoqtailRelay(
				context,
				publisher.told,
				expected.publisherEndLocation,
			);
		}
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ fetch: which, relay: options.relay, publisher: kind, expected, report })}`,
	);
	const summary = reportOnly
		? "the report above was compared with nothing"
		: expected.answer === "FETCH_OK"
			? `FETCH_OK with End Location {${expected.endLocation}} and [${expected.items.join("; ")}] on the fetch stream`
			: expected.answer === "REQUEST_ERROR"
				? `REQUEST_ERROR ${expected.errorCode}`
				: expected.answer === "served-to-the-relay"
					? `the relay asked the publishing page, which served the FETCH, and FETCH_OK has End Location {${expected.endLocation}}`
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
