// Interop driver: an own page sends one FETCH through a relay, and what it
// gets is compared with what the relay is known to answer.
//
// Usage: bun fetch.ts --fetch <case> [--relay moqtail|own] [--publisher <client>] [--port N]
//
// A publisher publishes the track and sends one Object, {0, 0}; the page
// subscribes and has it; then the page sends the FETCH. The cases:
//
//   standalone       Standalone, from {0, 0} to {1, 0}
//   relative-joining Relative Joining, Joining Start 0, for the page's SUBSCRIBE
//   absent-track     Standalone for a track nobody published
//
// The publisher is an own client by default: the moqtail relay passes a
// FETCH on to the publisher, and the moqtail-rust client here answers none.
//
// What is expected is in `EXPECTED`, written before the first run from
// docs/experiments/2026-10-11-moqtail-relay-fetch.md for the moqtail relay
// and from docs/moqt-implementation-notes.md for the own one.

import { randomBytes } from "node:crypto";
import { parseArgs } from "node:util";
import { startPublisher, startSubscriber } from "./clients.ts";
import {
	log,
	NAMESPACE,
	parseClient,
	runCleanups,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { PageFetch, PageFetchReport } from "./xmoq-page.ts";

const { values: options } = parseArgs({
	options: {
		fetch: { type: "string", default: "standalone" },
		publisher: { type: "string", default: "own-rust" },
		relay: { type: "string", default: "moqtail" },
		port: { type: "string", default: "44360" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The FETCH is cancelled when it is not over by then, and the report says so. */
const FETCH_TIMEOUT_MS = 5_000;

const CASES = ["standalone", "relative-joining", "absent-track"] as const;
type Case = (typeof CASES)[number];

const FETCHES: Record<Case, PageFetch> = {
	standalone: {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: TRACK,
		start: ["0", "0"],
		end: ["1", "0"],
	},
	"relative-joining": { type: "RelativeJoining", joiningStart: "0" },
	"absent-track": {
		type: "Standalone",
		namespace: NAMESPACE,
		trackName: "nobody-published-this",
		start: ["0", "0"],
		end: ["1", "0"],
	},
};

/** The one Object the publisher sent, with its payload filled in by the run. */
const THE_OBJECT = "the Object {0, 0}";

/**
 * What the page has to report, by relay and case. `items` is what the fetch
 * stream decodes to; `errorCode` the code of a REQUEST_ERROR.
 *
 * The moqtail relay 0.15.0, from the record of its FETCH:
 * - Standalone {0, 0} to {1, 0}: FETCH_OK and the Objects of Group 0. Run 6
 *   of the record had an End Location with Object 0 leave that Group out and
 *   give the ones before it. The End Location of the FETCH_OK is {0, 1}, the
 *   Object after the last one the relay holds.
 *   WHY not {1, 0}, the one asked for, which was written here first from
 *   runs 7 and 9 of the record: in those the publisher was moqtail's test
 *   client, which answers a FETCH that the relay passes on to it. An own
 *   publisher refuses the FETCH, and the relay then answers from what it
 *   holds: {0, 1} in the first run with each own publisher. With the
 *   moqtail-rust client of this directory as the publisher nothing came
 *   back at all: the relay passed the FETCH on, and that client does not
 *   answer one (docs/experiments/2026-10-11-fetch-from-a-page.md).
 * - Relative Joining, Joining Start 0: the Group of the largest Object and
 *   what follows up to it (runs 2 and 4), here Group 0 alone. The End
 *   Location is what that relay counts and is not compared.
 * - A track nobody published: REQUEST_ERROR DOES_NOT_EXIST, 0x10 (run 11).
 *
 * The own relay refuses every FETCH with NOT_SUPPORTED (0x3) until it
 * serves one (the notes, row on requests XmoQ does not implement).
 */
type Expected =
	| { answer: "FETCH_OK"; endLocation?: [string, string]; items: string[] }
	| { answer: "REQUEST_ERROR"; errorCode: string };

const EXPECTED: Record<string, Record<Case, Expected>> = {
	moqtail: {
		standalone: {
			answer: "FETCH_OK",
			endLocation: ["0", "1"],
			items: [THE_OBJECT],
		},
		"relative-joining": { answer: "FETCH_OK", items: [THE_OBJECT] },
		"absent-track": { answer: "REQUEST_ERROR", errorCode: "16" },
	},
	own: {
		standalone: { answer: "REQUEST_ERROR", errorCode: "3" },
		"relative-joining": { answer: "REQUEST_ERROR", errorCode: "3" },
		"absent-track": { answer: "REQUEST_ERROR", errorCode: "3" },
	},
};

/** Throws with what differs, unless the report is what `expected` has. */
function compare(
	report: PageFetchReport,
	expected: Expected,
	payloadHex: string,
): void {
	const seen = JSON.stringify(report);
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
		expected.endLocation !== undefined &&
		report.response.endLocation.join(",") !== expected.endLocation.join(",")
	) {
		throw new Error(
			`FETCH_OK has the End Location {${report.response.endLocation}}, not {${expected.endLocation}}: ${seen}`,
		);
	}
	const items = report.items.map((item) =>
		item.type === "object" &&
		item.group === "0" &&
		item.object === "0" &&
		item.payloadHex === payloadHex
			? THE_OBJECT
			: JSON.stringify(item),
	);
	if (items.join("; ") !== expected.items.join("; ")) {
		throw new Error(
			`the fetch stream had [${items.join("; ")}], not [${expected.items.join("; ")}]: ${seen}`,
		);
	}
	if (report.bodyEnd !== "closed" || report.end.type !== "served") {
		throw new Error(
			`the FETCH did not end with its answer and a finished fetch stream: ${seen}`,
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
		throw new Error(`nothing is expected of the relay "${options.relay}"`);
	}
	const publisher = parseClient("publisher", options.publisher);
	const payloadHex = Buffer.from(
		`xmoq-fetch ${randomBytes(8).toString("hex")}`,
	).toString("hex");
	let report: PageFetchReport;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients: [publisher, "own-browser"],
			verbose: options.verbose,
		});
		const run = { context, payloadHex };
		const publishing = await startPublisher(publisher, run, {});
		const page = await startSubscriber(
			"own-browser",
			"fetcher:own-browser",
			run,
			{},
		);
		const group = await withTimeout(publishing.send(), "the publisher to send");
		// The page has the Object by its subscription, so the relay has it.
		const received = await page.object(group);
		if (group !== 0 || received !== payloadHex) {
			throw new Error(
				`the page got Group ${group} with ${received} by its subscription, not Group 0 with what was sent`,
			);
		}
		report = await page.fetch(FETCHES[which], FETCH_TIMEOUT_MS);
		log("driver", `REPORT ${JSON.stringify(report)}`);
		compare(report, expected, payloadHex);
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ fetch: which, relay: options.relay, publisher, expected, report })}`,
	);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.fetch} FETCH through the ${options.relay} relay: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
