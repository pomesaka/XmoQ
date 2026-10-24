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
//   cancelled         The moqtail-rust client, which answers no FETCH, sends
//                     Group 0, and a second publisher, own-rust, Group 1. The
//                     page asks for Group 0, which the relay no longer holds
//                     and asks the first publisher for, and gives the FETCH
//                     up after a second. Own relay only.
//   publisher-refuses The own Rust client, which refuses a FETCH, sends
//                     Groups 0 and 1. The page asks for Group 0, which the
//                     relay asks the publisher for. Own relay only.
//   latest-group      The own Rust client sends Group 0. The page asks for
//                     {0, 0} to {1, 0}: the latest Group, which a relay
//                     that keeps it answers itself.
//   absent-track      A Standalone FETCH for a track nobody published.
//   other-publisher   The own Rust client, which refuses a FETCH, sends
//                     Group 0 with PUBLISH. Then an own page announces the
//                     namespace; it has Object 0 of Groups 100 to 102 and
//                     never sent them ("serve-unsent"). The page that
//                     fetches asks for {100, 0} to {102, 1}: the relay asks
//                     the Rust client first, and the page after its
//                     refusal. Own relay only.
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
//   silent-publisher-first
//                     As page-unsent, with a publisher before the page that
//                     answers nothing: `xmoq-quiet-peer`, which published
//                     the track first with PUBLISH and sends no Object
//                     (`--announce-and-stay-quiet yes --quiet-track`). The
//                     relay asks it first, the first source of the track
//                     (the page becomes the second when the relay
//                     subscribes to it); its answer does not come in 5 s, and
//                     the relay, with 5 s of the 10 the page that fetches
//                     waits left, asks the page (added on 2026-10-24,
//                     written before its first run). Own relay only.
//   page-cancelled    The publisher accepts and writes nothing; the page
//                     that fetches gives the FETCH up after a second.
//   page-refuses      The publisher refuses with DOES_NOT_EXIST.
//   page-large-group-cancelled
//                     The publisher sends one more Group, of 83 Objects of
//                     100,000 bytes each, which the relay then holds as the
//                     latest. The page asks for that Group, reads nothing
//                     of the fetch stream, and gives the FETCH up after a
//                     second. Written before the first run, from the notes
//                     ("The relay's cache", "The subscriber gives its FETCH
//                     up"): the relay answers itself, FETCH_OK with End
//                     Location {3, 83} and `WRITE_FETCH_STREAM … pieces=83`,
//                     and sends no FETCH to the publisher; the 8.3 MB do not
//                     fit into what a subscriber that reads nothing takes,
//                     so the relay is still writing when the page gives up.
//                     A page that gives a FETCH up stops the fetch stream as
//                     well as the request stream (`cancel` of `@xmoq/moqt`),
//                     so the relay's writer meets the stop of its own
//                     stream: `FETCH_WRITE_ENDED … stopped pieces=K
//                     code=0x1` with K below 83. A write that was whole says
//                     `finished pieces=83`, and the run fails for it. WHY 83
//                     Objects of 100,000 bytes: the relay had handed about
//                     6.2 MB to the transport when the page gave up (62
//                     Objects; 6 of 8 with Objects of 900,000 bytes), and
//                     8.3 MB is what fits under the 8 MiB the relay keeps of
//                     a track, so 2.1 MB is the margin there is. (The
//                     first expectation, `ABORT_FETCH_WRITE`, was wrong, 7
//                     runs of 7: that is the relay's own order to stop, for
//                     a subscriber that leaves the fetch stream alone.)
//   page-subgroups    The publisher sends one more Group, 3, on two subgroup
//                     streams: Subgroup 0 with Publisher Priority 10 and one
//                     property of an odd type on every Object, Subgroup 1
//                     with Publisher Priority 200 and one of an even type.
//                     Objects 0, 2, 4 go to Subgroup 0 and 1, 3, 5 to
//                     Subgroup 1, each with a payload of its own. Written
//                     before the first run, from the notes ("The relay's
//                     cache", "A Joining FETCH in the relay", "The
//                     Subscription Filter of a SUBSCRIBE"):
//                     1. After Objects 0 to 2, with both streams open, a
//                        page subscribes and sends a Relative Joining FETCH
//                        with Joining Start 0. The relay answers itself:
//                        FETCH_OK with End Location {3, 3}, and Objects 0,
//                        1, 2 in that order, each with its Subgroup ID, the
//                        Publisher Priority of its Subgroup, its property
//                        and its payload. `WRITE_FETCH_STREAM … pieces=3`
//                        and `FETCH_WRITE_ENDED … finished pieces=3`.
//                     2. The publisher sends Objects 3 to 5 and finishes
//                        the streams. The subscription of that page has
//                        exactly these three of the Group, with the same
//                        fields.
//                     3. The page asks for {3, 0} to {4, 0}: FETCH_OK with
//                        End Location {3, 6} and all six in the order of
//                        their Object IDs, `pieces=6` twice.
//                     The relay sends no FETCH to the publisher.
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
	STEP_TIMEOUT_MS,
	spawnQuietClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type {
	PageFetch,
	PageFetchReport,
	PageFetchServing,
	PageProperty,
} from "./xmoq-page.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "standalone" },
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24360" },
		verbose: { type: "boolean", default: false },
		// Prints what the page saw and compares it with nothing: for looking
		// at what a relay does with a case that has no expectation for it.
		"report-only": { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The FETCH is cancelled when it is not over by then, and the report says so. */
const FETCH_TIMEOUT_MS = 5_000;

/** `silent-publisher-first`: the page waits as long as its library does, 10 s (`fixtures/request-answer-wait/`). */
const SILENT_FETCH_TIMEOUT_MS = 10_000;
/** The page gives the FETCH of `cancelled` up after this long, well inside the 5 seconds the own relay waits for a publisher's answer. */
const CANCEL_AFTER_MS = 1_000;
const CASES = [
	"standalone",
	"fill-timeout-0",
	"cancelled",
	"publisher-refuses",
	"latest-group",
	"absent-track",
	"other-publisher",
	"page-standalone",
	"page-joining",
	"page-nothing-there",
	"page-unsent",
	"silent-publisher-first",
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
	"latest-group": "own-rust",
	"absent-track": "own-rust",
	"other-publisher": "own-rust",
	"page-standalone": "own-browser",
	"page-joining": "own-browser",
	"page-nothing-there": "own-browser",
	"page-unsent": "own-browser",
	"silent-publisher-first": "own-browser",
	"page-cancelled": "own-browser",
	"page-refuses": "own-browser",
};

/** What the publishing page of a `page-` case does with a FETCH. */
const PAGE_SERVES: Partial<Record<Case, PageFetchServing>> = {
	"page-standalone": "serve",
	"page-joining": "serve",
	"page-nothing-there": "serve",
	"page-unsent": "serve-unsent",
	"silent-publisher-first": "serve-unsent",
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
	"latest-group": {
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
	"other-publisher": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["100", "0"],
		end: ["102", "1"],
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
	"silent-publisher-first": {
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
 * `other-publisher`, own relay: the notes, "Whom a Standalone FETCH is asked
 * of": a publisher that refuses, or leaves before it answers, is followed by
 * the next one there is. The page that announced answers as in
 * `page-unsent`: FETCH_OK with {102, 1}, the Location after its Largest
 * Object {102, 0} (§10.13), and its three Objects.
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
/**
 * Who answers the FETCH of a case through the own relay, where the case is
 * about that: the relay from its cache of the latest Group, or the
 * publisher. The relay's log has `WRITE_FETCH_STREAM … pieces=N` for a
 * fetch stream it writes itself, and `OPEN_REQUEST … FETCH` for a FETCH it
 * sends a publisher (docs/moqt-implementation-notes.md, "The relay's cache:
 * what it holds, and which FETCH it answers").
 *
 * WHY written down: a case that is to show what a publisher's answer
 * becomes shows nothing when the relay answers in its place, and passes
 * all the same where the two answers are alike.
 */
const ANSWERED_BY: Partial<Record<Case, "relay" | "publisher">> = {
	// Group 0 is not the latest once Group 1 has come.
	cancelled: "publisher",
	"publisher-refuses": "publisher",
	// The one Group there is, whole: Object 0, and the range ends beyond it.
	"latest-group": "relay",
	"other-publisher": "publisher",
	"page-standalone": "publisher",
	"page-joining": "publisher",
	"page-nothing-there": "publisher",
	"page-unsent": "publisher",
	"silent-publisher-first": "publisher",
	"page-cancelled": "publisher",
	"page-refuses": "publisher",
};

/** How many Groups the publisher of a case sends before the FETCH, where it is more than one. */
const GROUPS_BEFORE: Partial<Record<Case, number>> = {
	"publisher-refuses": 2,
};

/** `silent-publisher-first`: the own relay sent the FETCH to two sessions, the silent one and the page, and cancelled the first. */
async function expectAskedTwice(context: RunContext): Promise<void> {
	const asked = (
		await context.relay.waitForLines(
			/^OPEN_REQUEST session=(\d+) own=\d+ FETCH/,
			2,
		)
	).map((match) => captureGroup(match, 1));
	if (asked[0] === asked[1]) {
		throw new Error(
			`the relay was to ask two sessions for the FETCH, one after the other, not ${asked.join(", ") || "none"}`,
		);
	}
}

/** Throws unless the own relay's log says that the one who is to answer the FETCH of the case did. */
async function expectAnsweredBy(
	context: RunContext,
	who: "relay" | "publisher",
): Promise<void> {
	const ASKED = /^OPEN_REQUEST session=\d+ own=\d+ FETCH/;
	if (who === "publisher") {
		// A FETCH the relay sent on is one it did not answer itself.
		await context.relay.waitForLine(ASKED);
		return;
	}
	const WRITTEN = /^WRITE_FETCH_STREAM session=\d+ request_id=\d+ pieces=\d+$/;
	await context.relay.waitForLine(WRITTEN);
	const asked = await context.relay.linesBefore({ line: WRITTEN }, ASKED);
	if (asked.length > 0) {
		throw new Error(
			`the relay was to answer the FETCH itself, and sent one to a publisher: ${asked.map((match) => match[0]).join("; ")}`,
		);
	}
}

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
		"latest-group": {
			answer: "FETCH_OK",
			endLocation: ["0", "1"],
			items: ["{0, 0}"],
		},
		"absent-track": { answer: "REQUEST_ERROR", errorCode: "16" },
		"other-publisher": {
			answer: "FETCH_OK",
			endLocation: ["102", "1"],
			items: ["{100, 0}", "{101, 0}", "{102, 0}"],
		},
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
		"silent-publisher-first": {
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
		"latest-group": {
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

/** The Objects of the large Group, and the bytes of each: 8.3 MB, just under the 8 MiB (8,388,608 bytes) the relay keeps of a track. */
const LARGE_GROUP = { objects: 83, bytes: 100_000 };

/** `page-large-group-cancelled`, which has a report of its own: nothing of the fetch stream is read. */
async function largeGroupCancelled(): Promise<string> {
	const sentHex = Buffer.from("xmoq-fetch large").toString("hex");
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: ["own-browser", "own-browser", "own-rust"],
		verbose: options.verbose,
	});
	const publisher = await publishFromAPage("serve", context, sentHex);
	const page = await startSubscriber(
		"own-browser",
		"fetcher:own-browser",
		{ context, payloadHex: sentHex },
		{},
	);
	if (
		publisher.sendLargeGroup === undefined ||
		page.objectsOfGroup === undefined ||
		page.fetchAndLeaveUnread === undefined
	) {
		throw new Error("the run needs own pages on both sides");
	}
	const { objects, bytes } = LARGE_GROUP;
	const group = await withTimeout(
		publisher.sendLargeGroup(objects, bytes),
		"the publisher to send the large Group",
	);
	// The relay has passed all of them on, so it holds all of them.
	const countObjects = page.objectsOfGroup;
	await pollFor(
		async () => ((await countObjects(group)) === objects ? true : undefined),
		`the page to have the ${objects} Objects of Group ${group} by its subscription`,
	);
	const report = await page.fetchAndLeaveUnread(
		{
			type: "Standalone",
			namespace: NAMESPACE,
			trackName: TRACK,
			start: [String(group), "0"],
			end: [String(group + 1), "0"],
		},
		CANCEL_AFTER_MS,
	);
	log("driver", `REPORT ${JSON.stringify(report)}`);
	if (
		report.endLocation.join(",") !== `${group},${objects}` ||
		report.ended !== "cancelled"
	) {
		throw new Error(
			`the FETCH was not accepted with End Location {${group}, ${objects}} and then given up: ${JSON.stringify(report)}`,
		);
	}
	const WRITTEN =
		/^WRITE_FETCH_STREAM session=(\d+) request_id=\d+ pieces=(\d+)$/;
	const written = await context.relay.waitForLine(WRITTEN);
	if (captureGroup(written, 2) !== String(objects)) {
		throw new Error(
			`the relay wrote ${captureGroup(written, 2)} pieces, not the ${objects} Objects of the Group: ${written[0]}`,
		);
	}
	const session = captureGroup(written, 1);
	// How the relay's writing ended, and how many Objects it had written
	// whole by then.
	const ENDED = new RegExp(
		`^FETCH_WRITE_ENDED session=${session} stream=\\d+ (\\S+) pieces=(\\d+)(?: code=(\\S+))?$`,
	);
	const ended = await context.relay.waitForLine(ENDED);
	const [how, pieces, code] = [1, 2, 3].map((index) => ended[index]);
	if (how !== "stopped" || code !== "0x1" || Number(pieces) >= objects) {
		throw new Error(
			`the relay's writing was to end as stopped with CANCELLED (0x1), before all ${objects} Objects were written: ${ended[0]}`,
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
	return `the relay answered with FETCH_OK for {${group}, ${objects}} and began to write ${objects} Objects of ${bytes} bytes; the page read none and gave up, and the relay's writing ended there, ${pieces} Objects written (${ended[0]})`;
}

/** The two subgroup streams of `page-subgroups`, and what every Object on each has. */
const SUBGROUPS: {
	subgroup: number;
	publisherPriority: number;
	properties: PageProperty[];
}[] = [
	{
		subgroup: 0,
		publisherPriority: 10,
		// Odd type: a length and bytes. Types no draft defines, so the relay
		// has to pass them on unchanged (§2.5).
		properties: [{ type: "13", bytesHex: "0142e01fffe1" }],
	},
	{
		subgroup: 1,
		publisherPriority: 200,
		// Even type: one variable-length integer.
		properties: [{ type: "16", varint: "1760140800123456" }],
	},
];

/** What Object `object` of `page-subgroups` is, as a FETCH and a subscription report it. */
function subgroupObject(group: number, object: number): string {
	// Objects 0, 2, 4 on the first stream and 1, 3, 5 on the second.
	const stream = SUBGROUPS[object % 2];
	if (stream === undefined) throw new Error("no such stream");
	return JSON.stringify({
		group: String(group),
		object: String(object),
		subgroup: String(stream.subgroup),
		publisherPriority: stream.publisherPriority,
		properties: stream.properties,
		payloadHex: Buffer.from(`xmoq-subgroups ${object}`).toString("hex"),
	});
}

/** The Objects in the fields `subgroupObject` has, in the order given. */
const asSubgroupObjects = (
	objects: {
		group: string;
		object: string;
		subgroup?: string | null;
		publisherPriority?: number | null;
		properties?: PageProperty[];
		payloadHex: string;
	}[],
): string[] =>
	objects.map(
		({ group, object, subgroup, publisherPriority, properties, payloadHex }) =>
			JSON.stringify({
				group,
				object,
				subgroup,
				publisherPriority,
				properties,
				payloadHex,
			}),
	);

/** Throws unless the FETCH was answered with the End Location and exactly these Objects of the Group, in this order, on a stream that was finished. */
function expectSubgroupObjects(
	what: string,
	report: PageFetchReport,
	group: number,
	objects: number[],
	endObject: number,
): void {
	const seen = JSON.stringify(report);
	if (
		!("endLocation" in report.response) ||
		report.response.endLocation.join(",") !== `${group},${endObject}`
	) {
		throw new Error(
			`${what}: no FETCH_OK with End Location {${group}, ${endObject}}: ${seen}`,
		);
	}
	const items = asSubgroupObjects(
		report.items.flatMap((item) => (item.type === "object" ? [item] : [])),
	);
	const expected = objects.map((object) => subgroupObject(group, object));
	if (
		items.length !== report.items.length ||
		items.join("\n") !== expected.join("\n")
	) {
		throw new Error(
			`${what}: the fetch stream was to have the Objects ${objects.join(", ")} of Group ${group} in that order, each with its Subgroup ID, Publisher Priority, property and payload, and had: ${seen}`,
		);
	}
	if (report.bodyEnd !== "closed" || report.end.type !== "served") {
		throw new Error(
			`${what}: the FETCH did not end with its answer and a finished fetch stream: ${seen}`,
		);
	}
}

/** `page-subgroups`: a Group on two subgroup streams, fetched from the relay's cache in its middle and when it is whole. */
async function subgroupsFromTheCache(): Promise<string> {
	const sentHex = Buffer.from("xmoq-fetch subgroups").toString("hex");
	const context = await startRun({
		relay: options.relay,
		port: options.port,
		clients: ["own-browser", "own-browser", "own-browser", "own-rust"],
		verbose: options.verbose,
	});
	const run = { context, payloadHex: sentHex };
	const publisher = await publishFromAPage("serve", context, sentHex);
	// Says when the relay has passed an Object on, and so holds it.
	const watcher = await startSubscriber(
		"own-browser",
		"watcher:own-browser",
		run,
		{},
	);
	const { takeGroup, writeToSubgroup, closeSubgroups } = publisher;
	const watched = watcher.objectsOfGroup;
	if (
		takeGroup === undefined ||
		writeToSubgroup === undefined ||
		closeSubgroups === undefined ||
		watched === undefined
	) {
		throw new Error("the run needs own pages on both sides");
	}
	const group = takeGroup.call(publisher);
	const send = async (objects: number[]): Promise<void> => {
		for (const object of objects) {
			const stream = SUBGROUPS[object % 2];
			if (stream === undefined) throw new Error("no such stream");
			await withTimeout(
				writeToSubgroup(
					group,
					stream,
					object,
					Buffer.from(`xmoq-subgroups ${object}`).toString("hex"),
					stream.properties,
				),
				`the publisher to write Object ${object}`,
			);
		}
	};
	const relayHas = (count: number): Promise<true> =>
		pollFor(
			async () => ((await watched(group)) === count ? true : undefined),
			`the watching page to have ${count} Objects of Group ${group} by its subscription`,
		);

	// Not in the order of the Object IDs: the relay puts them in order.
	await send([1, 0, 2]);
	await relayHas(3);
	const page = await startSubscriber(
		"own-browser",
		"fetcher:own-browser",
		run,
		{},
	);
	const received = page.receivedOfGroup;
	if (received === undefined) throw new Error("the run needs an own page");
	const joining = await page.fetch(
		{ type: "RelativeJoining", joiningStart: "0" },
		STEP_TIMEOUT_MS,
	);
	log("driver", `REPORT joining ${JSON.stringify(joining)}`);
	expectSubgroupObjects("the Joining FETCH", joining, group, [0, 1, 2], 3);

	await send([4, 3, 5]);
	await withTimeout(
		closeSubgroups(group),
		"the publisher to finish the streams",
	);
	await relayHas(6);
	const bySubscription = await pollFor(async () => {
		const objects = await received(group);
		return objects.length >= 3 ? objects : undefined;
	}, `the page to have the rest of Group ${group} by its subscription`);
	const rest = asSubgroupObjects(bySubscription).sort();
	const expectedRest = [3, 4, 5].map((object) => subgroupObject(group, object));
	if (rest.join("\n") !== expectedRest.join("\n")) {
		throw new Error(
			`the subscription was to bring Objects 3, 4, 5 of Group ${group} and no other of it, each with its Subgroup ID, Publisher Priority, property and payload, and brought: ${JSON.stringify(bySubscription)}`,
		);
	}

	const whole = await page.fetch(
		{
			type: "Standalone",
			namespace: NAMESPACE,
			trackName: TRACK,
			start: [String(group), "0"],
			end: [String(group + 1), "0"],
		},
		STEP_TIMEOUT_MS,
	);
	log("driver", `REPORT whole ${JSON.stringify(whole)}`);
	expectSubgroupObjects(
		"the FETCH of the whole Group",
		whole,
		group,
		[0, 1, 2, 3, 4, 5],
		6,
	);

	const ENDED =
		/^FETCH_WRITE_ENDED session=\d+ stream=\d+ (\S+) pieces=(\d+)(?: code=\S+)?$/;
	const ends = await context.relay.waitForLines(ENDED, 2);
	const written = await context.relay.linesBefore(
		{ line: ENDED, nth: 2 },
		/^WRITE_FETCH_STREAM session=\d+ request_id=\d+ pieces=(\d+)$/,
	);
	const wrote = written.map((line) => `pieces=${line[1]}`).join(", ");
	const ended = ends.map((line) => `${line[1]} pieces=${line[2]}`).join(", ");
	if (
		wrote !== "pieces=3, pieces=6" ||
		ended !== "finished pieces=3, finished pieces=6"
	) {
		throw new Error(
			`the relay was to write two fetch streams itself, of 3 and of 6 Objects, to their ends; it began [${wrote}] and ended [${ended}]`,
		);
	}
	const asked = await context.relay.linesBefore(
		{ line: ENDED, nth: 2 },
		/^OPEN_REQUEST session=\d+ own=\d+ FETCH/,
	);
	if (asked.length > 0) {
		throw new Error(
			`the relay was to answer both FETCHes itself, and sent one to a publisher: ${asked.map((match) => match[0]).join("; ")}`,
		);
	}
	return `Group ${group} on Subgroups 0 and 1: the relay answered a Joining FETCH in its middle with Objects 0 to 2, the subscription brought 3 to 5, and the relay answered a FETCH of the whole Group with all six in the order of their Object IDs, each with its Subgroup ID, Publisher Priority, property and payload`;
}

async function main(): Promise<void> {
	if (options.fetch === "page-subgroups") {
		if (options.relay !== "own") {
			throw new Error("--fetch page-subgroups runs through the own relay");
		}
		let summary: string;
		try {
			summary = await subgroupsFromTheCache();
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
	if (options.fetch === "page-large-group-cancelled") {
		if (options.relay !== "own") {
			throw new Error(
				"--fetch page-large-group-cancelled runs through the own relay",
			);
		}
		let summary: string;
		try {
			summary = await largeGroupCancelled();
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
							...(served === undefined && which !== "cancelled"
								? []
								: ["own-rust" as const]),
						],
			verbose: options.verbose,
		});
		if (which === "silent-publisher-first") {
			const silent = spawnQuietClient(context, "silent", 30_000, false, [
				"--announce-and-stay-quiet",
				"yes",
				"--quiet-track",
				`${NAMESPACE}/${TRACK}`,
			]);
			await silent.waitForLine(/^PUBLISHED_QUIET /);
		}
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
		if (publisher.told !== undefined) {
			for (let sent = 1; sent < (GROUPS_BEFORE[which] ?? 1); sent++) {
				await page.object(
					await withTimeout(publisher.told.send(), "the publisher to send"),
				);
			}
		}
		if (which === "cancelled") {
			// A later Group from a second publisher: Group 0, which the first
			// one sent and answers no FETCH for, is not the latest any more.
			const second = await startPublisher(
				"own-rust",
				{ context, payloadHex: sentHex },
				{ label: "pub2:own-rust", firstGroup: 1 },
			);
			await page.object(
				await withTimeout(second.send(), "the second publisher to send"),
			);
		}
		if (which === "other-publisher") {
			await startPublisher(
				"own-browser",
				{ context, payloadHex: sentHex },
				{
					label: "pub2:own-browser",
					servesWhenAsked: "at-once",
					servesFetches: "serve-unsent",
				},
			);
		}
		report = await page.fetch(
			FETCHES[which],
			which === "cancelled" || which === "page-cancelled"
				? CANCEL_AFTER_MS
				: which === "silent-publisher-first"
					? SILENT_FETCH_TIMEOUT_MS
					: FETCH_TIMEOUT_MS,
		);
		log("driver", `REPORT ${JSON.stringify(report)}`);
		if (!reportOnly) compare(report, expected, payloadHex);
		const answeredBy = ANSWERED_BY[which];
		if (!reportOnly && options.relay === "own" && answeredBy !== undefined) {
			await expectAnsweredBy(context, answeredBy);
		}
		if (which === "cancelled" && !reportOnly) {
			await expectUpstreamCancelled(context);
		}
		if (which === "silent-publisher-first" && !reportOnly) {
			await expectAskedTwice(context);
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
