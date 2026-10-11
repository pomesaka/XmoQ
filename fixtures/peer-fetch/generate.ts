// Writes opened.tsv and updates.tsv, the tables of README.md.
// Run: bun fixtures/peer-fetch/generate.ts
//
// WHY a script: every FETCH is one of two captures with a few bytes changed,
// and the rows say which.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const here = import.meta.dir;
const capture = (name: string): string[] =>
	readFileSync(join(here, "..", "moqt-18", `${name}.hex`), "utf8")
		.trim()
		.split(/\s+/);
const byte = (value: number): string => {
	// A varint of one byte is 0xxxxxxx in draft 18 (§1.4.1): 0 to 127.
	if (!Number.isInteger(value) || value < 0 || value > 0x7f) {
		throw new Error(`${value} is not a varint of one byte`);
	}
	return value.toString(16).padStart(2, "0");
};

// Type, a length of two bytes, Request ID 0, Fetch Type 1, the track
// ("xmoq", "phase0") "object", Start {0, 1}, End {1, 2}, and one parameter,
// SUBSCRIBER_PRIORITY 200.
const standaloneCapture = capture("fetch-standalone-moqtail-rs");
// Request ID 2, Fetch Type 2, Joining Request ID 0, Joining Start 0, and the
// same parameter.
const joiningCapture = capture("fetch-relative-joining-moqtail-rs");
if (
	standaloneCapture.length !== 32 ||
	standaloneCapture.slice(25, 32).join(" ") !== "00 01 01 02 01 20 c8" ||
	joiningCapture.join(" ") !== "16 00 07 02 02 00 00 01 20 c8"
) {
	throw new Error("the captures are not the ones this was written for");
}

/** FORWARD 1, which no FETCH may carry (§10.2.12), in place of SUBSCRIBER_PRIORITY 200. */
const FORWARD = ["01", "10", "01"];

/** A Standalone FETCH for the captured track. */
const standalone = (
	requestId: number,
	start: [group: number, object: number],
	end: [group: number, object: number],
	parameters?: string[],
): string => {
	const bytes = [...standaloneCapture];
	bytes[3] = byte(requestId);
	bytes.splice(25, 4, ...[...start, ...end].map(byte));
	if (parameters !== undefined) bytes.splice(29, 3, ...parameters);
	return bytes.join("");
};

/** A Joining FETCH: Fetch Type 2 is Relative, 3 Absolute; 4 is none. */
const joining = (
	requestId: number,
	fetchType: 2 | 3 | 4,
	joiningRequestId: number,
	joiningStart: number,
): string => {
	const bytes = [...joiningCapture];
	bytes.splice(3, 4, ...[requestId, fetchType, joiningRequestId, joiningStart].map(byte));
	return bytes.join("");
};

/** What this endpoint did, or saw, before the row's FETCH: see README.md. */
type Before =
	| "nothing"
	| "publish"
	| "subscribe"
	| "subscribe-accepted"
	| "subscribe-done"
	| "subscribe-rejected";
/** What the FETCH is; `neither` for a Fetch Type the draft does not have. */
type Kind = "standalone" | "joining" | "neither";
type Outcome = "handed" | `refused 0x${string}` | `closed 0x${string}`;

const opened: [name: string, before: Before, kind: Kind, sent: string, outcome: Outcome][] = [
	["the-captured-range", "nothing", "standalone", standalone(1, [0, 1], [1, 2]), "handed"],
	["one-object", "nothing", "standalone", standalone(1, [3, 4], [3, 5]), "handed"],
	// §10.12.3 asks for "the same or a larger Location": the same is taken,
	// though it names no Object.
	["end-is-the-start", "nothing", "standalone", standalone(1, [3, 4], [3, 4]), "handed"],
	["end-before-start-in-the-group", "nothing", "standalone", standalone(1, [3, 4], [3, 3]), "refused 0x11"],
	["end-is-the-whole-group-of-the-start", "nothing", "standalone", standalone(1, [3, 4], [3, 0]), "handed"],
	["end-in-an-earlier-group", "nothing", "standalone", standalone(1, [3, 0], [2, 9]), "refused 0x11"],
	["end-is-an-earlier-whole-group", "nothing", "standalone", standalone(1, [3, 0], [2, 0]), "refused 0x11"],
	["a-track-this-endpoint-published", "publish", "standalone", standalone(1, [0, 1], [1, 2]), "handed"],
	["a-track-the-peer-subscribed-to", "subscribe-accepted", "standalone", standalone(3, [0, 1], [1, 2]), "handed"],
	["joins-itself", "nothing", "joining", joining(1, 2, 1, 0), "refused 0x32"],
	["joins-a-later-request-id", "nothing", "joining", joining(1, 2, 3, 0), "refused 0x32"],
	["joins-a-publish-of-this-endpoint", "publish", "joining", joining(1, 2, 0, 0), "refused 0x32"],
	["joins-a-subscribe-that-waits", "subscribe", "joining", joining(3, 2, 1, 0), "handed"],
	["joins-an-accepted-subscribe", "subscribe-accepted", "joining", joining(3, 2, 1, 0), "handed"],
	["joins-an-accepted-subscribe-absolute", "subscribe-accepted", "joining", joining(3, 3, 1, 5), "handed"],
	["joins-a-subscribe-after-publish-done", "subscribe-done", "joining", joining(3, 2, 1, 0), "refused 0x32"],
	["joins-a-rejected-subscribe", "subscribe-rejected", "joining", joining(3, 2, 1, 0), "refused 0x32"],
	["request-id-of-the-wrong-parity", "nothing", "standalone", standalone(2, [0, 1], [1, 2]), "closed 0x4"],
	["request-id-used-before", "subscribe", "standalone", standalone(1, [0, 1], [1, 2]), "closed 0x4"],
	["a-parameter-that-is-none-of-a-fetch", "nothing", "standalone", standalone(1, [0, 1], [1, 2], FORWARD), "closed 0x3"],
	["a-fetch-type-that-is-none", "nothing", "neither", joining(1, 4, 1, 0), "closed 0x3"],
	["wrong-in-its-range-and-in-its-request-id", "nothing", "standalone", standalone(2, [3, 4], [3, 3]), "closed 0x4"],
];

// What happens to one Standalone FETCH of the peer, Request ID 1, for
// Start Location {3, 4} and End Location {5, 1}, in order; and what this endpoint sends for each
// step. A REQUEST_UPDATE has the next Request ID of the peer.
type Step =
	/** REQUEST_UPDATE with no parameter. */
	| "update"
	/** REQUEST_UPDATE with SUBSCRIBER_PRIORITY 200, which an update of a FETCH may carry. */
	| "update-priority"
	/** REQUEST_UPDATE with FORWARD 1, which it may not. */
	| "update-forward"
	/** The application accepts, with End Location {5, 1}: what was asked for. */
	| "accept"
	/** It accepts with End Location {3, 3}, which is before the Start Location. */
	| "accept-before-start"
	/** It accepts with End Location {9, 0}, beyond what was asked for. */
	| "accept-beyond-end"
	/** It rejects, with DOES_NOT_EXIST. */
	| "reject";
/** What a step sends: nothing, messages in order, a mistake of the caller that sends nothing, or the end of the session. */
type Sent = "-" | "FETCH_OK" | "REQUEST_OK" | "REQUEST_ERROR" | "usage-error" | "closed 0x3";

const updates: [name: string, steps: Step[], sent: Sent[][]][] = [
	["accepted", ["accept"], [["FETCH_OK"]]],
	["update-then-accepted", ["update", "accept"], [["-"], ["FETCH_OK", "REQUEST_OK"]]],
	["two-updates-then-accepted", ["update", "update-priority", "accept"], [["-"], ["-"], ["FETCH_OK", "REQUEST_OK", "REQUEST_OK"]]],
	["accepted-then-update", ["accept", "update", "update-priority"], [["FETCH_OK"], ["REQUEST_OK"], ["REQUEST_OK"]]],
	["update-then-rejected", ["update", "reject"], [["-"], ["REQUEST_ERROR"]]],
	["rejected-then-update", ["reject", "update"], [["REQUEST_ERROR"], ["-"]]],
	["update-with-forward", ["update-forward"], [["closed 0x3"]]],
	["accepted-then-update-with-forward", ["accept", "update-forward"], [["FETCH_OK"], ["closed 0x3"]]],
	["rejected-then-update-with-forward", ["reject", "update-forward"], [["REQUEST_ERROR"], ["-"]]],
	["accepted-twice", ["accept", "accept"], [["FETCH_OK"], ["usage-error"]]],
	["rejected-then-accepted", ["reject", "accept"], [["REQUEST_ERROR"], ["usage-error"]]],
	["end-before-the-start-then-accepted", ["accept-before-start", "accept"], [["usage-error"], ["FETCH_OK"]]],
	["end-beyond-what-was-asked", ["accept-beyond-end"], [["FETCH_OK"]]],
];

writeFileSync(
	join(here, "opened.tsv"),
	opened.map((row) => `${row.join("\t")}\n`).join(""),
);
writeFileSync(
	join(here, "updates.tsv"),
	updates
		.map(
			([name, steps, sent]) =>
				`${name}\t${steps.join(" ")}\t${sent.map((each) => each.join(" ")).join(" | ")}\n`,
		)
		.join(""),
);
