// Runs every pairing of `run.ts` in which at least one client is an own one,
// and the scenarios of `disconnect.ts`, and prints a table of the outcomes.
//
// Usage: bun matrix.ts [--relay moqtail] [--relay own] [--base-port N]
//                      [--only TEXT] [--pairing-timeout-seconds N]
//   --relay: the relays to run the pairings through (default: both). The
//     scenarios of `disconnect.ts` run through the own relay only.
//   --only: runs the pairings whose name contains TEXT. The name begins with
//     the relay, as in "own relay: own-rust to own-browser".
//   --pairing-timeout-seconds: how long one pairing may take before it is
//     stopped and counted as failed.
//
// The exit code is 0 only if every pairing ended as the table below expects.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { ensureCertificate, MOQLIVEMOCK_SIDE_PORT_OFFSET } from "./harness.ts";

const here = import.meta.dir;
const logDir = join(here, ".logs", "matrix");

const CLIENTS = [
	"own-browser",
	"own-rust",
	"moqtail-browser",
	"moqtail-rust",
] as const;
type Client = (typeof CLIENTS)[number];

const isOwn = (client: Client): boolean => client.startsWith("own-");

interface Pairing {
	name: string;
	/** The driver that runs it. */
	script:
		| "run.ts"
		| "disconnect.ts"
		| "fetch.ts"
		| "fetch-rust.ts"
		| "catalog.ts"
		| "video.ts"
		| "video-rust.ts"
		| "watch-rust.ts"
		| "no-answer.ts"
		| "quiet-peer.ts"
		| "namespaces.ts"
		| "chat.ts"
		| "rendezvous.ts"
		| "new-group.ts";
	/** The arguments of the driver, without the relay and the port. */
	args: string[];
	/**
	 * Set where the run spends most of its time waiting: for the relay to give
	 * up on a client that vanished (6 to 9 seconds each, measured), or for a
	 * page to give up on late streams (5 seconds). These run
	 * beside the others, so that the matrix does not grow by the sum of the
	 * waits.
	 */
	mostlyWaits?: boolean;
	/**
	 * Absent when the Object has to arrive. Otherwise the run has to fail, and
	 * the line in which the driver says why it failed has to show this: the
	 * reason, not only that something failed.
	 */
	expectedFailure?: RegExp;
	/**
	 * Set where the reason is in a line of a client and the driver's own line
	 * says only that the client exited. The whole log is searched then.
	 */
	reasonIsInAClientLine?: boolean;
	/** More environment for the driver, beside this process's own. */
	env?: Record<string, string>;
}

function transfers(): Pairing[] {
	const pairings: Pairing[] = [];
	for (const publisher of CLIENTS) {
		for (const subscriber of CLIENTS) {
			// A run between two moqtail peers checks nothing of this repository.
			if (!isOwn(publisher) && !isOwn(subscriber)) continue;
			// Only the own clients report how their subscription ended.
			const awaitsDone = isOwn(subscriber);
			pairings.push({
				name: `${publisher} to ${subscriber}${awaitsDone ? ", then PUBLISH_DONE" : ""}`,
				script: "run.ts",
				args: [
					"--publisher",
					publisher,
					"--subscriber",
					subscriber,
					...(awaitsDone ? ["--await-publish-done"] : []),
				],
			});
		}
	}
	return pairings;
}

const OWN_CLIENTS = CLIENTS.filter(isOwn);

/**
 * An Object with Object Properties no relay knows, one of each form of a
 * Key-Value-Pair. Draft 18 §2.5 has a relay forward them unchanged.
 *
 * WHY the own clients alone: they are the ones that send properties with
 * content and report the ones they get. The moqtail peers send an empty set.
 */
const OBJECT_PROPERTIES: Pairing[] = OWN_CLIENTS.flatMap((publisher) =>
	OWN_CLIENTS.map(
		(subscriber): Pairing => ({
			name: `${publisher} to ${subscriber}, an Object with Object Properties, then PUBLISH_DONE`,
			script: "run.ts",
			args: [
				"--publisher",
				publisher,
				"--subscriber",
				subscriber,
				"--object-properties",
				"--await-publish-done",
			],
		}),
	),
);

/**
 * A page that announces the namespace and sends no PUBLISH: the relay asks
 * it for the track with a SUBSCRIBE of its own, and the page sends on that
 * subscription (`serve` of `@xmoq/moqt`). Both relays ask.
 */
const ANNOUNCE_ONLY: Pairing[] = ["moqtail-rust", "own-rust"].map(
	(subscriber): Pairing => ({
		name: `own-browser only announces the namespace, and serves ${subscriber} when the relay asks it`,
		script: "run.ts",
		args: [
			"--publisher",
			"own-browser",
			"--subscriber",
			subscriber,
			"--announce-only",
		],
	}),
);

/**
 * One publisher and three subscribers: every subscriber has to get the Object
 * and then see PUBLISH_DONE. One row per own client as the subscribers.
 */
const ONE_TO_MANY: Pairing[] = [
	{
		// §10.9: the relay answers REQUEST_OK, and the subscription goes on.
		name: "own-browser to own-rust, which changes its delivery timeout with a REQUEST_UPDATE first",
		script: "run.ts",
		args: [
			"--publisher",
			"own-browser",
			"--subscriber",
			"own-rust",
			"--update-subgroup-timeout-ms",
			"5000",
		],
	},
	{
		name: "own-rust to 3 own-browser, then PUBLISH_DONE",
		script: "run.ts",
		args: [
			"--publisher",
			"own-rust",
			"--subscriber",
			"own-browser",
			"--subscribers",
			"3",
			"--await-publish-done",
		],
	},
	{
		name: "moqtail-browser to 3 own-rust, then PUBLISH_DONE",
		script: "run.ts",
		args: [
			"--publisher",
			"moqtail-browser",
			"--subscriber",
			"own-rust",
			"--subscribers",
			"3",
			"--await-publish-done",
		],
	},
];

/** A SUBSCRIBE for a track nobody published: the relay answers REQUEST_ERROR DOES_NOT_EXIST (0x10). */
const REFUSALS: Pairing[] = [
	{
		name: "own-browser subscribes to a track nobody published",
		script: "run.ts",
		args: [
			"--publisher",
			"moqtail-rust",
			"--subscriber",
			"own-browser",
			"--subscribe-track",
			"missing",
		],
		expectedFailure: /RequestFailedError: REQUEST_ERROR 0x10/,
	},
	{
		name: "own-rust subscribes to a track nobody published",
		script: "run.ts",
		args: [
			"--publisher",
			"moqtail-rust",
			"--subscriber",
			"own-rust",
			"--subscribe-track",
			"missing",
		],
		expectedFailure: /SUBSCRIBE rejected: error_code=0x10/,
		reasonIsInAClientLine: true,
	},
];

/**
 * An own page sends a FETCH. What each has to show, and where that is
 * written, is at `EXPECTED` in fetch.ts. `relays` is where the case is run:
 * two of them are about what the own relay does without a publisher's
 * answer, which nothing is known of for the moqtail relay.
 *
 * WHY they run one after the other, and not beside the runs that mostly
 * wait: each starts a page, and eight more pages at once made a run of
 * that group miss a time of its own in CI (2 of 3 runs; the FETCH runs took
 * 9 to 10 seconds there, and 1 to 3 alone).
 */
const FETCHES: (Pairing & { relays: string[] })[] = [
	{
		name: "own-browser fetches {0, 1} to {1, 2} of what moqtail's test client publishes",
		script: "fetch.ts",
		args: ["--fetch", "standalone"],
		relays: ["moqtail", "own"],
	},
	{
		name: "own-browser fetches with FILL_TIMEOUT 0 and gets one End of Unknown Range",
		script: "fetch.ts",
		args: ["--fetch", "fill-timeout-0"],
		relays: ["own"],
	},
	{
		name: "own-browser gives its FETCH for a Group the relay no longer holds up while moqtail-rust does not answer; the relay cancels its own",
		script: "fetch.ts",
		args: ["--fetch", "cancelled"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches a Group the relay no longer holds from own-rust, which refuses a FETCH",
		script: "fetch.ts",
		args: ["--fetch", "publisher-refuses"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches the latest Group of own-rust, which answers no FETCH; the relay answers",
		script: "fetch.ts",
		args: ["--fetch", "latest-group"],
		relays: ["moqtail", "own"],
	},
	{
		name: "own-browser fetches a track nobody published",
		script: "fetch.ts",
		args: ["--fetch", "absent-track"],
		relays: ["moqtail", "own"],
	},
	{
		name: "own-browser fetches what own-rust, the first publisher, refuses; the relay asks an own page that announced the namespace, which answers",
		script: "fetch.ts",
		args: ["--fetch", "other-publisher"],
		relays: ["own"],
	},
	// The publisher is an own page that only announced: it serves the track
	// when the relay asks, and answers a FETCH from what it has sent (`serve`
	// of `@xmoq/moqt`).
	{
		name: "own-browser fetches {0, 0} to {2, 1} from an own page that answers a FETCH",
		script: "fetch.ts",
		args: ["--fetch", "page-standalone"],
		relays: ["own"],
	},
	{
		name: "own-browser subscribes and sends a Joining FETCH; the relay asks the own page for the range, which answers",
		script: "fetch.ts",
		args: ["--fetch", "page-joining"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches where the own page has nothing, and gets its End of Range indicator",
		script: "fetch.ts",
		args: ["--fetch", "page-nothing-there"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches Objects the own page has and never sent; the relay asks the page",
		script: "fetch.ts",
		args: ["--fetch", "page-unsent"],
		relays: ["moqtail", "own"],
	},
	{
		name: "own-browser gives its FETCH up while the own page serves it; the page sees it cancelled",
		script: "fetch.ts",
		args: ["--fetch", "page-cancelled"],
		relays: ["own"],
	},
	{
		name: "own-browser asks for a Group of 8.3 MB the relay holds, reads nothing and gives up; the relay gives the FETCH up",
		script: "fetch.ts",
		args: ["--fetch", "page-large-group-cancelled"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches a Group on two subgroup streams from the relay's cache, with a Joining FETCH in its middle and whole",
		script: "fetch.ts",
		args: ["--fetch", "page-subgroups"],
		relays: ["own"],
	},
	{
		name: "own-browser fetches from an own page that refuses with DOES_NOT_EXIST",
		script: "fetch.ts",
		args: ["--fetch", "page-refuses"],
		relays: ["own"],
	},
	// The own Rust client is the one that fetches. What each has to show is
	// at `EXPECTED` in fetch-rust.ts.
	{
		name: "own-rust fetches {0, 0} to {2, 1} from an own page that answers a FETCH",
		script: "fetch-rust.ts",
		args: ["--fetch", "page-standalone"],
		relays: ["own"],
	},
	{
		name: "own-rust subscribes and sends a Relative Joining FETCH; the relay asks the own page for the range",
		script: "fetch-rust.ts",
		args: ["--fetch", "page-join-relative"],
		relays: ["own"],
	},
	{
		name: "own-rust subscribes and sends an Absolute Joining FETCH; the relay asks the own page for the range",
		script: "fetch-rust.ts",
		args: ["--fetch", "page-join-absolute"],
		relays: ["own"],
	},
	{
		name: "own-rust asks for a Group of 7.2 MB the relay holds, reads nothing and gives up on the request stream alone; the relay stops writing by its own order",
		script: "fetch-rust.ts",
		args: ["--fetch", "page-large-request-only"],
		relays: ["own"],
	},
	{
		name: "own-rust fetches {0, 1} to {1, 2} of what moqtail's test client publishes",
		script: "fetch-rust.ts",
		args: ["--fetch", "test-standalone"],
		relays: ["moqtail", "own"],
	},
	{
		name: "own-rust subscribes and sends a Relative Joining FETCH for what moqtail's test client publishes",
		script: "fetch-rust.ts",
		args: ["--fetch", "test-join-relative"],
		relays: ["own"],
	},
	{
		name: "own-rust sends its SUBSCRIBE and a Joining FETCH at once; the relay answers the FETCH whichever it reads first",
		script: "fetch-rust.ts",
		args: ["--fetch", "page-join-at-once"],
		relays: ["own"],
	},
];

/** The arguments that name clients in one role, as in `--subscriber a --subscriber b`. */
function each(option: string, clients: string[]): string[] {
	return clients.flatMap((client) => [`--${option}`, client]);
}

/**
 * The runs in which a client leaves. What each has to show is listed in
 * docs/experiments/2026-10-10-disconnects-through-own-relay.md. They need the
 * own relay: the driver waits for lines only it prints.
 */
const DISCONNECTS: Pairing[] = [
	{
		name: "own-rust publisher is killed; own-browser and own-rust subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publisher-killed",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-browser", "own-rust"]),
		],
		mostlyWaits: true,
	},
	{
		name: "own-browser publisher's page is closed with a stream open; all four subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publisher-killed",
			"--publisher",
			"own-browser",
			...each("subscriber", CLIENTS.slice()),
		],
	},
	{
		name: "moqtail-browser publisher's page is closed; own-browser and own-rust subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publisher-killed",
			"--publisher",
			"moqtail-browser",
			...each("subscriber", ["own-browser", "own-rust"]),
		],
	},
	{
		// Without PUBLISH_DONE the Object may be lost with the session, and the
		// run passes either way; its summary says which (disconnect.ts,
		// `closedWithoutPublishDone`).
		name: "own-browser publisher closes right after its last Object; all four subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"last-object-then-close",
			"--publisher",
			"own-browser",
			...each("subscriber", CLIENTS.slice()),
		],
	},
	{
		name: "own-browser publisher closes right after its last Object and PUBLISH_DONE; all four subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"last-object-then-close",
			"--publisher",
			"own-browser",
			"--with-publish-done",
			...each("subscriber", CLIENTS.slice()),
		],
	},
	{
		name: "own-rust publisher closes right after its last Object and PUBLISH_DONE; all four subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"last-object-then-close",
			"--publisher",
			"own-rust",
			"--with-publish-done",
			...each("subscriber", CLIENTS.slice()),
		],
	},
	// moqtail-browser is left out as the publisher here: its `sendObject` queues
	// the Object and its `disconnect()` ends the PUBLISH before the Object is
	// written, so the relay is never given it.
	{
		// WHY it mostly waits: the moqtail Rust peer exits right after
		// `connection.close()`, and in 13 of 40 runs the relay learned of it by
		// its idle timeout only (12 seconds later then, 6 to 9 now). The Object
		// had arrived.
		name: "moqtail-rust publisher closes right after its last Object; own-browser and own-rust subscribe",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"last-object-then-close",
			"--publisher",
			"moqtail-rust",
			...each("subscriber", ["own-browser", "own-rust"]),
		],
		mostlyWaits: true,
	},
	{
		// moqtail-rust is not among them: it leaves "in order" by exiting, which
		// the relay saw as silence (comment above).
		name: "subscribers leave in order (own-browser and own-rust close, moqtail-browser cancels); own-rust publishes on",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subscriber-leaves",
			"--publisher",
			"own-rust",
			...each("leaver", [
				"own-browser:close",
				"own-rust:close",
				"moqtail-browser:cancel",
			]),
			...each("stayer", ["own-browser", "own-rust"]),
		],
	},
	{
		// moqtail-rust is not among them either: it has left by itself after the
		// first Object. It is killed where the leavers go before the one Object.
		name: "subscribers are killed (own-browser, own-rust, moqtail-browser); own-browser publishes on",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subscriber-leaves",
			"--publisher",
			"own-browser",
			...each("leaver", [
				"own-browser:kill",
				"own-rust:kill",
				"moqtail-browser:kill",
			]),
			...each("stayer", ["own-browser", "own-rust"]),
		],
		mostlyWaits: true,
	},
	{
		name: "subscribers leave before the one Object of moqtail-browser (own-browser closes, own-rust and moqtail-rust are killed)",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subscriber-leaves",
			"--publisher",
			"moqtail-browser",
			...each("leaver", [
				"own-browser:close",
				"own-rust:kill",
				"moqtail-rust:kill",
			]),
			...each("stayer", ["own-browser", "own-rust", "moqtail-rust"]),
		],
	},
	{
		name: "subscribers leave before the one Object of moqtail-rust (own-browser is killed, moqtail-browser cancels)",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subscriber-leaves",
			"--publisher",
			"moqtail-rust",
			...each("leaver", ["own-browser:kill", "moqtail-browser:cancel"]),
			...each("stayer", ["own-browser", "own-rust", "moqtail-browser"]),
		],
	},
	{
		name: "own-browser cancels its subscription; own-rust publishes on",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subscriber-leaves",
			"--publisher",
			"own-rust",
			...each("leaver", ["own-browser:cancel"]),
			...each("stayer", ["own-rust"]),
		],
	},
	{
		// WHY own-browser publishes: own-rust waits for the acknowledgement of
		// each stream and took 8 seconds to send what gets the subscriber cut
		// off, longer than the 6 seconds after which the relay drops a silent
		// session (9 seconds when this was measured). own-browser took 0.6
		// seconds.
		name: "a stopped own-rust subscriber is cut off (TOO_FAR_BEHIND); own-browser and own-rust get everything",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"slow-subscriber",
			"--publisher",
			"own-browser",
			...each("subscriber", ["own-browser", "own-rust"]),
		],
	},
	{
		// The publisher has sent 577 or 641 Objects, one a stream, when the relay
		// cuts the page off: 256 streams wait that the browser does not let it
		// open. The Objects are small on purpose: with one large Object a
		// stream, the page was not told that its subscription had ended
		// (docs/experiments/2026-10-10-subscription-backlog-in-chromium.md).
		name: "an own-browser page that does not read is cut off (TOO_FAR_BEHIND); own-browser and own-rust get everything",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"slow-subscriber",
			"--slow",
			"unread-own-browser",
			"--publisher",
			"own-browser",
			...each("subscriber", ["own-browser", "own-rust"]),
		],
		// The cut-off comes within 2 seconds; the 5 seconds after it are the
		// library's wait for late streams before it ends the subscription.
		mostlyWaits: true,
	}, // Two sources of one track (docs/moqt-implementation-notes.md, the rows on
	// a PUBLISH for a track that has a source, on a Subgroup from another
	// source, and on a source that goes while another is left).
	{
		name: "two own-rust publishers send the same Group; own-rust and own-browser get it once",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"same-group-twice",
			...each("publisher", ["own-rust", "own-rust"]),
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		// More Groups than the relay remembers a source for (8, provisional):
		// the second source's Group 0 is then an old one and must not be opened
		// towards the subscribers a second time (§2.2).
		name: "an own-rust publisher sends Group 0 again when another has sent ten; own-rust gets it once",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"same-group-twice",
			"--first-sends",
			"10",
			...each("publisher", ["own-rust", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
	},
	{
		// The moqtail client sends its one Object with the payload of the run,
		// as the own one does here: two sources of a Subgroup send the same.
		// WHY moqtail first: the second publisher's stream is stopped, and the
		// moqtail client 0.15.0 ends with an error when that happens before it
		// has written the stream out. So it is the source the Group is taken
		// from here, and the own client the one that is stopped.
		name: "moqtail-rust and own-rust send the same Group; own-rust and moqtail-rust get it once",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"same-group-twice",
			...each("publisher", ["moqtail-rust", "own-rust"]),
			...each("subscriber", ["own-rust", "moqtail-rust"]),
		],
	},

	{
		name: "own-browser and own-rust send the same Group; own-rust gets it once",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"same-group-twice",
			...each("publisher", ["own-browser", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
	},
	{
		name: "the first of two own-rust publishers leaves with PUBLISH_DONE; own-rust goes on to the second",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"second-source",
			"--first",
			"done",
			...each("publisher", ["own-rust", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
	},
	{
		// The source that was ahead has gone, and the track is back to one
		// source: what the subscribers have from the first must still not be
		// opened towards them again, also for Groups older than the relay
		// remembers a source for (8, provisional).
		name: "the first of two own-rust publishers sends ten Groups and leaves; the second sends them again and then the next, which own-rust gets",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"second-source",
			"--first",
			"done",
			"--first-sends",
			"10",
			"--second-resends",
			...each("publisher", ["own-rust", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
	},
	{
		name: "the first of two own-rust publishers is killed; own-rust goes on to the second",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"second-source",
			"--first",
			"kill",
			...each("publisher", ["own-rust", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
		mostlyWaits: true,
	},
	{
		name: "an own-browser publisher's page is closed with a stream open; own-rust and own-browser go on to an own-rust publisher",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"second-source",
			"--first",
			"kill-with-open-stream",
			...each("publisher", ["own-browser", "own-rust"]),
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		// The page sends on a subscription the relay asked for: `serve` of
		// `@xmoq/moqt`. It is the first to announce and the first to send, so
		// its Group is Group 0, the one a page can start at.
		name: "an own-browser page and an own-rust publisher only announce the namespace; the relay asks both, and own-rust and own-browser get a Group of each",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"ask-all",
			"--ask",
			"two-announcers",
			...each("publisher", ["own-browser", "own-rust"]),
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "two own-rust publishers only announce the namespace; the relay asks both, and own-rust and own-browser get a Group of each",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"ask-all",
			"--ask",
			"two-announcers",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "own-browser publishes and an own-rust publisher announces before the subscribers come; the relay asks the one that announced",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"ask-all",
			"--ask",
			"announcer-and-publisher",
			"--publisher",
			"own-browser",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "own-rust publishes, own-rust and own-browser subscribe, and an own-rust publisher announces then; the relay asks it at once",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"ask-all",
			"--ask",
			"late-announcer",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "an own-browser page announces and refuses what it is asked; the relay asks it once for two subscribers, and again when it announces anew",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"ask-all",
			"--ask",
			"refusing-announcer",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "an own-rust publisher sends a Group after an Object with the status END_OF_TRACK; the track ends as malformed",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"end-of-track",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "two own-rust publishers send the same Group with Publisher Priority 0 and 1; the track ends as malformed",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subgroup-priorities",
			"--priorities",
			"0,1",
			"--publisher",
			"own-rust",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "own-browser and own-rust send the same Group with Publisher Priority 0 and 1; the track ends as malformed",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subgroup-priorities",
			"--priorities",
			"0,1",
			"--publisher",
			"own-browser",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "an own-browser publisher sends a Group again with another Publisher Priority; its track ends as malformed",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subgroup-priorities",
			"--priorities",
			"0,1",
			"--publisher",
			"own-browser",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "own-rust leaves the Publisher Priority out and own-browser writes 128 for the same Group; the track goes on",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"subgroup-priorities",
			"--priorities",
			"default,128",
			"--publisher",
			"own-rust",
			"--publisher",
			"own-browser",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		// WHY it mostly waits: the first source drains for the 2 seconds the
		// relay gives the streams a PUBLISH_DONE counted.
		name: "an own-rust publisher says PUBLISH_DONE and publishes anew under another Track Alias; own-rust and own-browser stay",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publish-anew",
			"--publisher",
			"own-rust",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
		mostlyWaits: true,
	},
	{
		name: "an own-rust publisher says PUBLISH_DONE and publishes anew under the same Track Alias; own-rust and own-browser stay",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publish-anew",
			"--publisher",
			"own-rust",
			"--same-alias",
			...each("subscriber", ["own-rust", "own-browser"]),
		],
	},
	{
		name: "an own-rust publisher sends PUBLISH while the relay waits for another's answer; own-rust and own-browser are answered by it",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"publish-while-asked",
			...each("publisher", ["own-rust", "own-rust"]),
			// WHY the page first: the relay's wait for the publisher that is
			// asked starts with the first SUBSCRIBE, and the later subscribers
			// start inside it. A process starts faster than a page.
			...each("subscriber", ["own-browser", "own-rust"]),
		],
	},
	{
		name: "a second own-rust publisher joins a track that has subscribers; own-rust gets its Group",
		script: "disconnect.ts",
		args: [
			"--scenario",
			"second-source",
			"--first",
			"stays",
			"--second-joins",
			"after",
			...each("publisher", ["own-rust", "own-rust"]),
			"--subscriber",
			"own-rust",
		],
	},
];

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", multiple: true, default: ["moqtail", "own"] },
		// WHY one port per pairing: some pairings run at the same time, and a
		// pairing that is killed at its time limit can leave its relay running,
		// which must not hold the port of the pairings after it.
		//
		// WHY below 32768: from there to 60999 Linux takes the source ports of
		// outgoing sockets (`net.ipv4.ip_local_port_range`, as scripts/ci/interop.sh
		// prints it), and a client of a pairing that is running could be given
		// the port a later pairing's relay is to listen on. One pairing of a CI
		// run failed at the start of its relay with "Address already in use";
		// that this is why was not shown. macOS takes them from 49152 on.
		"base-port": { type: "string", default: "24400" },
		only: { type: "string", default: "" },
		// Provisional. A pairing takes about a second; `run.ts` gives each of
		// its own steps 20 seconds.
		"pairing-timeout-seconds": { type: "string", default: "120" },
	},
});

/** After SIGTERM, how long `run.ts` gets to stop what it started. */
const KILL_GRACE_MS = 10_000;

/** The pairings that are running, so that a signal to this process reaches them. */
const running = new Set<Bun.Subprocess>();
let interrupted = false;

// WHY forwarded: the drivers stop their relay, browser and clients when they
// get a signal. Without this, stopping the matrix leaves them running, and
// the relays keep their ports.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
	process.once(signal, () => {
		interrupted = true;
		for (const child of running) child.kill(signal);
	});
}

/** A pairing and the relay it runs through; the name begins with the relay. */
interface RelayPairing extends Pairing {
	relay: string;
}

interface Outcome {
	pairing: RelayPairing;
	ok: boolean;
	detail: string;
	seconds: number;
	log: string;
}

/** The last line of a run's log that begins with one of these words of the driver. */
function lastDriverLine(log: string, words: string): string | undefined {
	const pattern = new RegExp(`^\\[driver\\] (?:${words}) `);
	return log
		.split("\n")
		.filter((candidate) => pattern.test(candidate))
		.at(-1);
}

/** How a run says it ended. */
function ending(log: string): string {
	return (
		lastDriverLine(log, "RESULT|FAILED") ??
		"the run printed neither RESULT nor FAILED"
	);
}

/** What a run that ended well showed. */
function shown(pairing: Pairing, log: string): string {
	if (pairing.script === "run.ts") return "the payload matched";
	return (
		lastDriverLine(log, "SUMMARY")?.replace("[driver] SUMMARY ", "") ??
		"ended as the scenario expects"
	);
}

async function runPairing(
	pairing: RelayPairing,
	port: number,
	timeoutMs: number,
): Promise<Outcome> {
	const started = performance.now();
	const logFile = join(
		logDir,
		`${pairing.name.replaceAll(/[^a-z0-9]+/gi, "-").toLowerCase()}.log`,
	);
	const child = Bun.spawn(
		[
			process.execPath,
			join(here, pairing.script),
			...pairing.args,
			// `disconnect.ts` runs through the own relay only, and `catalog.ts`
			// and `video.ts` through one of their own or none; none of them
			// takes a relay.
			...(pairing.script === "disconnect.ts" ||
			pairing.script === "catalog.ts" ||
			pairing.script === "video.ts"
				? []
				: ["--relay", pairing.relay]),
			"--port",
			String(port),
		],
		{
			cwd: here,
			env: { ...process.env, ...pairing.env },
			stdout: "pipe",
			stderr: "pipe",
		},
	);
	running.add(child);
	let timedOut = false;
	let killer: ReturnType<typeof setTimeout> | undefined;
	// SIGTERM first, so that the driver stops its relay and clients itself.
	const limit = setTimeout(() => {
		timedOut = true;
		child.kill("SIGTERM");
		killer = setTimeout(() => child.kill("SIGKILL"), KILL_GRACE_MS);
	}, timeoutMs);
	const [stdout, stderr, exitCode] = await Promise.all([
		new Response(child.stdout).text(),
		new Response(child.stderr).text(),
		child.exited,
	]);
	clearTimeout(limit);
	clearTimeout(killer);
	running.delete(child);
	const log = stdout + stderr;
	writeFileSync(logFile, log);
	const seconds = (performance.now() - started) / 1000;
	const outcome = (ok: boolean, detail: string): Outcome => ({
		pairing,
		ok,
		detail,
		seconds,
		log: logFile,
	});

	// Checked first: a pairing stopped at its limit proved nothing, also where
	// a failure was the expected outcome.
	if (timedOut) {
		return outcome(
			false,
			`stopped after the limit of ${timeoutMs / 1000}s; ${ending(log)}`,
		);
	}
	if (pairing.expectedFailure === undefined) {
		// Both drivers exit 0 only after a RESULT line: `run.ts` when the
		// payloads match, `disconnect.ts` when the scenario ended as it has to.
		const passed =
			exitCode === 0 && lastDriverLine(log, "RESULT") !== undefined;
		return outcome(passed, passed ? shown(pairing, log) : ending(log));
	}
	// WHY the line of the driver and not the whole log: the same words in a
	// line of a page would pass a run that failed for another reason.
	const refused =
		exitCode !== 0 &&
		pairing.expectedFailure.test(
			pairing.reasonIsInAClientLine === true ? log : ending(log),
		);
	return outcome(
		refused,
		refused
			? `failed as expected (${pairing.expectedFailure.source})`
			: `expected a failure showing ${pairing.expectedFailure.source}; got exit ${exitCode}: ${ending(log)}`,
	);
}

/** Every pairing with the relay it runs through, in the order of the table. */
function pairingsThrough(relays: string[]): RelayPairing[] {
	const named = (relay: string, pairing: Pairing): RelayPairing => ({
		...pairing,
		name: `${relay} relay: ${pairing.name}`,
		relay,
	});
	return [
		...relays.flatMap((relay) =>
			[
				...transfers(),
				...OBJECT_PROPERTIES,
				...ANNOUNCE_ONLY,
				...ONE_TO_MANY,
				...REFUSALS,
			].map((pairing) => named(relay, pairing)),
		),
		// "fetch: " lets `--only` pick these runs alone.
		...relays.flatMap((relay) =>
			FETCHES.filter((pairing) => pairing.relays.includes(relay)).map(
				({ relays: _where, ...pairing }) =>
					named(relay, { ...pairing, name: `fetch: ${pairing.name}` }),
			),
		),
		// A publisher that is a server: no relay is in between, so the run is
		// there whichever relays were asked for.
		{
			name: "no relay: catalog: own-browser reads the MSF catalog of moqlivemock's publisher with a Joining FETCH",
			script: "catalog.ts",
			args: ["--catalog", "from-mlmpub"],
			relay: "none",
		},
		{
			name: "no relay: video: own-browser plays a video track of moqlivemock's publisher",
			script: "video.ts",
			args: ["--video", "from-mlmpub"],
			relay: "none",
		},
		{
			name: "own relay: video: an own page encodes and publishes video, and own-rust writes it as an H.264 byte stream",
			script: "video-rust.ts",
			args: [],
			relay: "own",
		},
		{
			name: "own relay: watch: an own page publishes a catalog, video and sound on one connection, and the terminal viewer finds and plays them",
			script: "watch-rust.ts",
			args: ["--watch", "page"],
			relay: "own",
		},
		{
			name: "own relay: goaway: the own relay is sent SIGTERM and sends GOAWAY naming another own relay, and the terminal viewer moves there and plays the same broadcast",
			script: "watch-rust.ts",
			args: ["--watch", "goaway"],
			relay: "own",
		},
		{
			name: "no relay: watch: the terminal viewer finds a video and an audio track in the catalog of moqlivemock's publisher and plays them",
			script: "watch-rust.ts",
			args: ["--watch", "from-mlmpub"],
			relay: "none",
		},
		// A publisher that holds its answer back: the subscriber gives its
		// SUBSCRIBE up after the time its answer gets, and its session stays.
		{
			name: "own relay: no answer: own-browser gives up a SUBSCRIBE that is not answered in time, and its session goes on",
			script: "no-answer.ts",
			args: ["--subscriber", "own-browser"],
			relay: "own",
		},
		{
			name: "own relay: no answer: own-rust gives up a SUBSCRIBE that is not answered in time, and closes its session itself",
			script: "no-answer.ts",
			args: ["--subscriber", "own-rust"],
			relay: "own",
		},
		{
			name: "own relay: no answer: the terminal viewer gives up the SUBSCRIBE of its video that is not answered in time, and closes its session itself",
			script: "no-answer.ts",
			args: ["--subscriber", "xmoq-watch"],
			relay: "own",
		},
		// Namespace discovery (§6.1, §10.18): an own-rust client asks for the
		// namespaces under a prefix and is told of one as it is announced and
		// withdrawn.
		{
			name: "moqtail relay: namespaces: own-rust is told with NAMESPACE and NAMESPACE_DONE of a namespace own-rust announces and withdraws under its prefix",
			script: "namespaces.ts",
			args: [],
			relay: "moqtail",
		},
		{
			name: "own relay: namespaces: own-rust is told with NAMESPACE and NAMESPACE_DONE of a namespace own-rust announces and withdraws under its prefix",
			script: "namespaces.ts",
			args: [],
			relay: "own",
		},
		// The gatherer of the chat (`crates/chat`): own-rust clients are the
		// participants of a room and its viewers.
		{
			name: "own relay: chat: the gatherer numbers the lines of a room as they come, serves them live and from its store, drops lines too long or too fast, numbers on after a restart, gathers one room, and ends it",
			script: "chat.ts",
			args: [],
			relay: "own",
		},
		// WHY expected to fail: moqtail's relay 0.15.0 passes on the Objects of
		// an upstream fetch stream alone, and takes an End of Range indicator
		// for no prior Location (`FetchObject::context` of moqtail-rs), where
		// §11.4.4.2 has "Prior Group ID and prior Object ID: The values from
		// the End of Range indicator". The answer the gatherer cut to its limit
		// reaches the viewer as Groups 0 to 5 and without the indicator
		// (docs/experiments/2026-10-24-chat-gatherer.md). The rest of the run
		// passes before that.
		// WHY no restart of the gatherer here (`--restart no`): moqtail's relay
		// 0.15.0 did not answer the PUBLISH of a gatherer that came again at
		// once in 2 of about 20 runs, CI's among them; its track manager takes
		// its `tracks` and `track_aliases` locks in both orders
		// (`remove_track` and `get_track_by_alias` of
		// apps/relay/src/server/track_manager.rs, read at relay@0.15.0), which
		// would wedge it. The restart runs through the own relay.
		{
			name: "moqtail relay: chat: the gatherer numbers and serves lines as through the own relay, without a restart, and a FETCH it cut to its limit reaches the viewer without its End of Unknown Range indicator",
			script: "chat.ts",
			args: ["--restart", "no"],
			relay: "moqtail",
			expectedFailure: /told of unknown lines up to: none, not 2/,
		},
		// The own clients against a peer that sends no SETUP, or answers no
		// request (`crates/quiet-peer`): what reached the peer is what counts.
		{
			name: "no relay: quiet peer: own-browser closes its session with CONTROL_MESSAGE_TIMEOUT when the peer sends no SETUP",
			script: "quiet-peer.ts",
			args: ["--case", "no-setup", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust closes its session with CONTROL_MESSAGE_TIMEOUT when the peer sends no SETUP",
			script: "quiet-peer.ts",
			args: ["--case", "no-setup", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer closes its session with CONTROL_MESSAGE_TIMEOUT when the peer sends no SETUP",
			script: "quiet-peer.ts",
			args: ["--case", "no-setup", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser closes its session with NO_ERROR at a GOAWAY and connects to the URI it names",
			script: "quiet-peer.ts",
			args: ["--case", "goaway", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser as a publisher closes its session with NO_ERROR at a GOAWAY and announces on the URI it names",
			script: "quiet-peer.ts",
			args: ["--case", "goaway-publisher", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser moves its session to the URI of a GOAWAY on its SUBSCRIBE's stream",
			script: "quiet-peer.ts",
			args: ["--case", "goaway-request", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust closes its session with NO_ERROR at a GOAWAY on its SUBSCRIBE's stream",
			script: "quiet-peer.ts",
			args: ["--case", "goaway-request", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer moves its session to the URI of a GOAWAY on its video's stream",
			script: "quiet-peer.ts",
			args: ["--case", "goaway-request", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser closes its session with INVALID_REQUEST_ID for a request of the peer with a Request ID of a client's parity",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-parity", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust closes its session with INVALID_REQUEST_ID for a request of the peer with a Request ID of a client's parity",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-parity", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer closes its session with INVALID_REQUEST_ID for a request of the peer with a Request ID of a client's parity",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-parity", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser closes its session with INVALID_REQUEST_ID for a REQUEST_UPDATE that uses the Request ID of a request it refused",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-again", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust closes its session with INVALID_REQUEST_ID for a REQUEST_UPDATE that uses the Request ID of a request it refused",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-again", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer closes its session with INVALID_REQUEST_ID for a REQUEST_UPDATE that uses the Request ID of a request it refused",
			script: "quiet-peer.ts",
			args: ["--case", "request-id-again", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust closes its session with NO_ERROR at a GOAWAY",
			script: "quiet-peer.ts",
			args: ["--case", "goaway", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer closes its session with NO_ERROR at a GOAWAY and connects to the URI it names",
			script: "quiet-peer.ts",
			args: ["--case", "goaway", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser ends a PUBLISH nobody answers with PUBLISH_DONE and a FIN",
			script: "quiet-peer.ts",
			args: ["--case", "no-publish-answer", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust ends a PUBLISH nobody answers with PUBLISH_DONE and a FIN",
			script: "quiet-peer.ts",
			args: ["--case", "no-publish-answer", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser gives up a FETCH nobody answers, and resets its request stream",
			script: "quiet-peer.ts",
			args: ["--case", "no-fetch-answer", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust gives up a FETCH nobody answers, resets its request stream, and fails",
			script: "quiet-peer.ts",
			args: ["--case", "no-fetch-answer", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser fails a SUBSCRIBE as not sent when the peer allows it no stream, and its session goes on",
			script: "quiet-peer.ts",
			args: ["--case", "no-stream", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust fails a SUBSCRIBE as not sent when the peer allows it no stream",
			script: "quiet-peer.ts",
			args: ["--case", "no-stream", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: the terminal viewer fails a SUBSCRIBE as not sent when the peer allows it no stream",
			script: "quiet-peer.ts",
			args: ["--case", "no-stream", "--client", "xmoq-watch"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-browser does not send a Subgroup the peer allows no stream for, and its session goes on",
			script: "quiet-peer.ts",
			args: ["--case", "no-subgroup-stream", "--client", "own-browser"],
			relay: "none",
		},
		{
			name: "no relay: quiet peer: own-rust does not send a Subgroup the peer allows no stream for, and ends its track with Stream Count 0",
			script: "quiet-peer.ts",
			args: ["--case", "no-subgroup-stream", "--client", "own-rust"],
			relay: "none",
		},
		{
			name: "own relay: quiet peer: the relay closes the session of a client that sends no SETUP with CONTROL_MESSAGE_TIMEOUT",
			script: "quiet-peer.ts",
			args: ["--case", "relay-no-setup", "--client", "quiet-peer"],
			relay: "own",
		},
		{
			name: "own relay: quiet peer: the relay closes a session that sent its SETUP and holds nothing, and keeps one that announced",
			script: "quiet-peer.ts",
			args: ["--case", "relay-idle-session", "--client", "quiet-peer"],
			relay: "own",
		},
		{
			name: "own relay: quiet peer: the relay refuses a session's PUBLISH_NAMESPACE requests beyond five at once and one a second with EXCESSIVE_LOAD, and closes nothing",
			script: "quiet-peer.ts",
			args: ["--case", "relay-many-announcements", "--client", "quiet-peer"],
			relay: "own",
		},
		{
			name: "own relay: quiet peer: one session that asks a publisher which answers nothing for more than its share is refused, and another viewer's SUBSCRIBEs and FETCHes reach that publisher",
			script: "quiet-peer.ts",
			args: [
				"--case",
				"relay-one-session-asks-for-much",
				"--client",
				"quiet-peer",
			],
			relay: "own",
			mostlyWaits: true,
		},
		{
			name: "own relay: quiet peer: a reader that takes a fetch stream from the relay's cache too slowly as a whole is cut with TOO_FAR_BEHIND, and one that reads gets it",
			script: "quiet-peer.ts",
			args: ["--case", "relay-slow-cached-fetch", "--client", "quiet-peer"],
			relay: "own",
			mostlyWaits: true,
		},
		{
			name: "own relay: quiet peer: a fetch stream that keeps a Group the cache has let go of, beyond its session's share, is stopped with EXCESSIVE_LOAD, and another reader is answered",
			script: "quiet-peer.ts",
			args: [
				"--case",
				"relay-group-kept-beyond-the-cache",
				"--client",
				"quiet-peer",
			],
			relay: "own",
			mostlyWaits: true,
		},
		// A SUBSCRIBE that comes before its publisher (RENDEZVOUS_TIMEOUT).
		{
			name: "own relay: rendezvous: own-browser subscribes before anybody publishes the track, is held, and is accepted when own-rust publishes",
			script: "rendezvous.ts",
			args: ["--case", "publisher-comes", "--subscriber", "own-browser"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-rust subscribes before anybody publishes the track, is held, and is accepted when own-rust publishes",
			script: "rendezvous.ts",
			args: ["--case", "publisher-comes", "--subscriber", "own-rust"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-browser's held SUBSCRIBE is refused with TIMEOUT when nobody publishes the track",
			script: "rendezvous.ts",
			args: ["--case", "nobody-comes", "--subscriber", "own-browser"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-rust's held SUBSCRIBE is refused with TIMEOUT when nobody publishes the track",
			script: "rendezvous.ts",
			args: ["--case", "nobody-comes", "--subscriber", "own-rust"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-rust's SUBSCRIBE without RENDEZVOUS_TIMEOUT is refused at once",
			script: "rendezvous.ts",
			args: ["--case", "not-asked", "--subscriber", "own-rust"],
			relay: "own",
		},
		// §10.2.13: the relay passes a subscriber's NEW_GROUP_REQUEST on to
		// the publisher of a track with DYNAMIC_GROUPS, on the stream of its
		// PUBLISH.
		...["own-rust", "own-browser"].map(
			(publisher): RelayPairing => ({
				name: `own relay: new group: own-rust subscribes with NEW_GROUP_REQUEST, and ${publisher}, which published the track with DYNAMIC_GROUPS, is asked with a REQUEST_UPDATE, answers it and goes on`,
				script: "new-group.ts",
				args: ["--publisher", publisher],
				relay: "own",
			}),
		),
		{
			name: "own relay: rendezvous: own-browser subscribes while the relay waits for the late streams of a track that ended, is held, and is accepted when own-rust publishes anew",
			script: "rendezvous.ts",
			args: ["--case", "publisher-returns", "--subscriber", "own-browser"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-rust subscribes while the relay waits for the late streams of a track that ended, is held, and is accepted when own-rust publishes anew",
			script: "rendezvous.ts",
			args: ["--case", "publisher-returns", "--subscriber", "own-rust"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-browser is held while a track that ended drains, and gets the first Object of another publisher that numbers its Groups from the time",
			script: "rendezvous.ts",
			args: [
				"--case",
				"another-publisher-comes",
				"--subscriber",
				"own-browser",
			],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: own-rust is held while a track that ended drains, and gets the first Object of another publisher that numbers its Groups from the time",
			script: "rendezvous.ts",
			args: ["--case", "another-publisher-comes", "--subscriber", "own-rust"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: a page that waits as /watch?wait=1 does is held on its session when a page ends its broadcast with PUBLISH_DONE, and plays the one another page publishes",
			script: "rendezvous.ts",
			args: ["--case", "page-publisher-returns", "--subscriber", "own-browser"],
			relay: "own",
		},
		{
			name: "own relay: rendezvous: the terminal viewer comes before a page's broadcast, is held, and plays it",
			script: "rendezvous.ts",
			args: [
				"--case",
				"viewer-before-the-broadcast",
				"--subscriber",
				"xmoq-watch",
			],
			relay: "own",
		},
		// A page encodes and publishes video, and another plays it: through
		// each relay, with each way of putting Objects onto streams.
		// `video.ts` starts the relay itself.
		...relays.flatMap((relay) =>
			(["stream-per-group", "stream-per-object"] as const).flatMap((mapping) =>
				(["bars", "noise"] as const).map((picture) => ({
					name: `${relay} relay: video: an own page encodes and publishes ${picture === "bars" ? "video" : "video of several MiB a Group"}, ${mapping === "stream-per-group" ? "a Group" : "an Object"} on a stream, and another plays it`,
					script: "video.ts" as const,
					args: [
						"--video",
						"page-to-page",
						"--through",
						relay,
						"--mapping",
						mapping,
						"--picture",
						picture,
					],
					relay,
				})),
			),
		),
		// The same with delivery timeouts in the viewer's SUBSCRIBE (transport
		// §8): no relay here acts on them, and neither is to refuse them.
		...relays.map((relay) => ({
			name: `${relay} relay: video: a viewer that sends delivery timeouts with its SUBSCRIBE plays the video of an own page`,
			script: "video.ts" as const,
			args: [
				"--video",
				"page-to-page",
				"--through",
				relay,
				"--delivery-timeout-ms",
				"10000",
			],
			relay,
		})),
		// The viewer changes its SUBGROUP_DELIVERY_TIMEOUT while it plays, with
		// a REQUEST_UPDATE (§10.9): the relay answers REQUEST_OK, and every
		// frame is played as without it. That a relay acts on the new time is
		// not seen here, where nothing is late (the own relay's tests have it).
		...relays.map((relay) => ({
			name: `${relay} relay: video: a viewer that changes its delivery timeout with a REQUEST_UPDATE goes on playing the video of an own page`,
			script: "video.ts" as const,
			args: [
				"--video",
				"page-to-page",
				"--through",
				relay,
				"--delivery-timeout-ms",
				"10000",
				"--update-delivery-timeout-ms",
				"5000",
			],
			relay,
		})),
		// moqtail's relay sends GOAWAY to every session at SIGTERM, naming the
		// URI of its --redirect-uri: here the own relay, which `video.ts`
		// starts on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET.
		...(relays.includes("moqtail")
			? [
					{
						name: "moqtail relay: goaway: moqtail's relay is sent SIGTERM and sends GOAWAY, and the own pages that publish and play move to the relay it names and go on",
						script: "video.ts" as const,
						args: ["--video", "goaway-from-moqtail"],
						relay: "moqtail",
					},
					{
						name: "moqtail relay: goaway: the viewer comes to the relay it names before the publisher, is refused there, and tries that relay again and plays",
						script: "video.ts" as const,
						args: ["--video", "goaway-from-moqtail-viewer-first"],
						relay: "moqtail",
					},
					{
						name: "moqtail relay: goaway: moqtail's relay is sent SIGTERM and sends GOAWAY, and own-rust, which subscribes, takes it and leaves with 0",
						script: "video.ts" as const,
						args: ["--video", "goaway-from-moqtail-rust"],
						relay: "moqtail",
					},
				]
			: []),
		...(relays.includes("own")
			? [
					{
						name: "own relay: goaway: the own relay is sent SIGTERM and sends GOAWAY naming another own relay, the own pages that publish and play move there and go on, and the relay ends once they have left",
						script: "video.ts" as const,
						args: ["--video", "goaway-from-own"],
						relay: "own",
					},
					{
						name: "own relay: goaway: the own relay is sent SIGTERM and sends GOAWAY, and own-rust, which subscribes, takes it and leaves with 0",
						script: "video.ts" as const,
						args: ["--video", "goaway-from-own-rust"],
						relay: "own",
					},
					{
						name: "own relay: video: an own page encodes and publishes video, and moqlivemock's subscriber takes its Groups and Objects",
						script: "video.ts" as const,
						args: ["--video", "to-mlmsub"],
						relay: "own",
					},
					{
						name: "own relay: video: a viewer that comes in the middle of a Group fetches its start from the relay and has a picture at once",
						script: "video.ts" as const,
						args: ["--video", "late-viewer"],
						relay: "own",
					},
					{
						name: "own relay: video: a viewer that comes in the middle of a Group asks for a new one, the relay passes the request on its own SUBSCRIBE, and the publishing page begins one",
						script: "video.ts" as const,
						args: ["--video", "late-viewer-new-group"],
						relay: "own",
					},
					{
						name: "own relay: audio: an own page encodes a tone as Opus and publishes it, and another decodes it",
						script: "video.ts" as const,
						args: ["--video", "audio"],
						relay: "own",
					},
					{
						name: "own relay: video: the relay is stopped and started anew, and the pages that publish and play connect again and go on",
						script: "video.ts" as const,
						args: ["--video", "reconnect"],
						relay: "own",
						// WHY: in CI a page of this run crashed now and then,
						// on Linux alone, and no reason was left
						// (docs/experiments/2026-10-14-the-page-that-crashed-when-the-relay-came-back.md).
						// playwright-core 1.64.0 passes on what Chromium prints
						// to stderr only with this; a renderer that crashes
						// prints its signal there ("Received signal 11", seen
						// on macOS with chrome://crash). WHY this run alone:
						// Chromium prints a great deal there.
						env: { DEBUG: "pw:browser" },
					},
					{
						name: "own relay: video: an own page whose key frames are all over what it sends is told so",
						script: "video.ts" as const,
						args: ["--video", "key-frames-too-large"],
						relay: "own",
					},
				]
			: []),
		// The other two runs with a catalog go through the own relay, and
		// `catalog.ts` starts it itself.
		...(relays.includes("own")
			? [
					{
						name: "own relay: catalog: an own page publishes an MSF catalog and replaces it; another reads both",
						script: "catalog.ts" as const,
						args: ["--catalog", "own"],
						relay: "own",
					},
					{
						name: "own relay: catalog: an own page that reads the catalog before the broadcast begins is held, and reads it when another page publishes",
						script: "catalog.ts" as const,
						args: ["--catalog", "reader-first"],
						relay: "own",
					},
					{
						name: "own relay: catalog: an own page publishes an MSF catalog, which moqlivemock's subscriber reads with a Joining FETCH",
						script: "catalog.ts" as const,
						args: ["--catalog", "to-mlmsub"],
						relay: "own",
					},
				]
			: []),
		// "leaving: " lets `--only` pick these runs alone.
		...(relays.includes("own")
			? DISCONNECTS.map((pairing) =>
					named("own", { ...pairing, name: `leaving: ${pairing.name}` }),
				)
			: []),
	];
}

async function main(): Promise<void> {
	const pairings = pairingsThrough(options.relay).filter((pairing) =>
		pairing.name.includes(options.only),
	);
	if (pairings.length === 0) {
		throw new Error(`no pairing contains "${options.only}"`);
	}
	// WHY the digits are checked: `Number("")` is 0.
	if (!/^[0-9]+$/.test(options["base-port"])) {
		throw new Error(`--base-port "${options["base-port"]}" is not a port`);
	}
	const basePort = Number(options["base-port"]);
	// The run with `mlmpub` takes a second port, its own plus an offset.
	const lastPort =
		basePort +
		pairings.length -
		1 +
		(pairings.some(
			(pairing) =>
				pairing.script === "catalog.ts" || pairing.script === "video.ts",
		)
			? MOQLIVEMOCK_SIDE_PORT_OFFSET
			: 0);
	if (basePort < 1 || lastPort > 65535) {
		throw new Error(
			`--base-port ${basePort} with ${pairings.length} pairings needs the ports ${basePort} to ${lastPort}, outside 1 to 65535`,
		);
	}
	const timeoutSeconds = Number(options["pairing-timeout-seconds"]);
	if (
		!/^[0-9]+(\.[0-9]+)?$/.test(options["pairing-timeout-seconds"]) ||
		timeoutSeconds <= 0
	) {
		throw new Error(
			`--pairing-timeout-seconds "${options["pairing-timeout-seconds"]}" is not a positive number`,
		);
	}
	mkdirSync(logDir, { recursive: true });
	// WHY here: the drivers that run at the same time would each create the
	// certificate when it is missing or about to expire, over one another.
	ensureCertificate();

	const started = performance.now();
	const finished = new Map<RelayPairing, Outcome>();
	const run = async (pairing: RelayPairing): Promise<void> => {
		const outcome = await runPairing(
			pairing,
			basePort + pairings.indexOf(pairing),
			timeoutSeconds * 1000,
		);
		finished.set(pairing, outcome);
		console.log(
			`${outcome.ok ? "ok  " : "FAIL"} ${pairing.name} (${outcome.seconds.toFixed(1)}s): ${outcome.detail}`,
		);
		if (!outcome.ok) {
			console.log(`---- ${outcome.log}`);
			console.log(readFileSync(outcome.log, "utf8"));
			console.log("----");
		}
	};
	// WHY the pairings one after the other: a failure is easier to read alone,
	// and a run that has the machine to itself does not fail for lack of it.
	// The runs that mostly wait are the exception. They start together when
	// the first of them is reached, which is after the pairings that move one
	// Object, and go on beside the pairings that follow.
	let waiting: Promise<void>[] = [];
	for (const pairing of pairings) {
		// A signal stops the pairings that are running; the rest is not started.
		if (interrupted) break;
		if (pairing.mostlyWaits !== true) {
			await run(pairing);
		} else if (waiting.length === 0) {
			waiting = pairings.filter((each) => each.mostlyWaits === true).map(run);
		}
	}
	await Promise.all(waiting);

	const outcomes = pairings.flatMap((pairing) => finished.get(pairing) ?? []);
	const failed = outcomes.filter((outcome) => !outcome.ok);
	const total = ((performance.now() - started) / 1000).toFixed(1);
	const table = [
		"| Pairing | Result | Seconds |",
		"|---|---|---|",
		...outcomes.map(
			(outcome) =>
				`| ${outcome.pairing.name} | ${outcome.ok ? outcome.detail : `**FAILED**: ${outcome.detail}`} | ${outcome.seconds.toFixed(1)} |`,
		),
	].join("\n");
	console.log(`\n${table}\n`);
	console.log(
		`${outcomes.length - failed.length} of ${outcomes.length} pairings ended as expected in ${total}s; logs in ${logDir}`,
	);
	// The job summary of GitHub Actions shows the same table.
	const summary = process.env.GITHUB_STEP_SUMMARY;
	if (summary !== undefined) {
		writeFileSync(summary, `${table}\n`, { flag: "a" });
	}
	if (interrupted) {
		console.log(
			`interrupted after ${outcomes.length} of ${pairings.length} pairings`,
		);
	}
	if (failed.length > 0 || interrupted) process.exitCode = 1;
}

main().catch((error: unknown) => {
	console.log(
		`FAILED ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
