// The chat through two relays: the gatherer (`xmoq-chat`) at the own relay of
// the run, and the participants and the viewers at a second own relay below
// it (`--parent`), on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET. The
// relay below passes the namespaces its participants announce on to the one
// above (§9.5), where the gatherer is told of them by its
// SUBSCRIBE_NAMESPACE, and the gatherer's SUBSCRIBE for each `said` comes
// back down to the participant.
//
// Usage: bun chat-edge.ts [--case two|many] [--port N] [--verbose]
//
// What has to be seen with `--case many`, written before the first run: 7
// participants that come at the relay below at once, before it has a
// session with its parent, are all announced to the parent though the relay
// above takes 5 announcements of a session at once and one more a second:
// the relay below is refused the others (`EXCESSIVE_LOAD`), says when it
// sends them again (`ANNOUNCE_TO_PARENT_LATER ... after_ms=1000`), and does;
// the gatherer subscribes to all 7 and numbers a line of each, 0 to 6.
//
// What has to be seen with `--case two` (the default), written before the
// first run:
//
//   - The gatherer is told of `a` and `b`, both at the relay below, and
//     subscribes to each one's `said` (`SUBSCRIBED`); each participant serves
//     the subscription (`SERVING`).
//   - The relay below announces each namespace to its parent once
//     (`OPEN_REQUEST ... PUBLISH_NAMESPACE`, two in all).
//   - Lines of `a` and `b`, one after the other, are numbered 0 to 3, and a
//     viewer at the relay below subscribed to `lines` gets them live: Groups
//     0 to 3, whose JSON names a, b, a, b.
//   - A viewer at the relay below that comes then FETCHes 0 to 100 and gets
//     Groups 0 to 3, with `FETCH_OK` ending at {3, 1}.
//   - When `a` leaves, the relay below withdraws its namespace from its parent
//     (`CANCEL_OWN_REQUEST`), and the relay above tells the gatherer
//     (`NAMESPACE_DONE`).

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	captureGroup,
	type LoggedProcess,
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	pollFor,
	type RunContext,
	runCleanups,
	spawnChatGatherer,
	spawnRustClient,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		// The relay above: the own one alone, whose lines the run reads.
		relay: { type: "string", default: "own" },
		case: { type: "string", default: "two" },
		port: { type: "string", default: "24397" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const ROOM = "broadcast/r";

/** What a participant says: the JSON of `said`, in hex for `--payload-hex`. */
function said(text: string): string {
	const json = JSON.stringify({ text, sentAt: 1_761_300_000_000 });
	return Buffer.from(json, "utf8").toString("hex");
}

const subscribed = (name: string): RegExp =>
	new RegExp(`^SUBSCRIBED room="${ROOM}" name="${name}"$`);

/** A participant of the room at `below`, which says `text` for every line of its input. */
async function participant(
	below: RunContext,
	gatherer: LoggedProcess,
	name: string,
	text: string,
): Promise<LoggedProcess> {
	const spawned = spawnRustClient(
		below,
		`said:${name}`,
		"own-rust",
		"publish-namespace",
		"said",
		[
			"--namespace",
			`xmoq/chat/${ROOM}/${name}`,
			"--payload-hex",
			said(text),
			"--timeout-secs",
			"60",
		],
	);
	await spawned.waitForLine(/^NAMESPACE_PUBLISHED/);
	await spawned.waitForLine(/^SERVING /);
	await gatherer.waitForLine(subscribed(name));
	return spawned;
}

/** Has `who` say one line, and waits for the gatherer to number it `number`. */
async function sayOne(
	gatherer: LoggedProcess,
	who: LoggedProcess,
	name: string,
	number: number,
): Promise<void> {
	const taken = new RegExp(
		`^LINE room="${ROOM}" number=(\\d+) name="${name}" `,
	);
	const before = gatherer.matches(taken).length;
	who.child.stdin.write("line\n");
	const numbered = await pollFor(
		() => gatherer.matches(taken)[before],
		`the gatherer to take a line of ${name}`,
	);
	const got = Number(captureGroup(numbered, 1));
	if (got !== number) {
		throw new Error(`the line of ${name} was numbered ${got}, not ${number}`);
	}
}

/** Participants that come at once in `--case many`: more than the 5 announcements the relay above takes of a session at once. */
const MANY = 7;

async function many(
	below: RunContext,
	gatherer: LoggedProcess,
	edge: RunContext["relay"],
): Promise<string> {
	const names = Array.from({ length: MANY }, (_, index) => `p${index}`);
	const participants = await Promise.all(
		names.map((name) => participant(below, gatherer, name, `from ${name}`)),
	);
	const [later] = await edge.waitForLines(
		/^ANNOUNCE_TO_PARENT_LATER session=\d+ after_ms=(\d+)$/,
		1,
	);
	if (later === undefined)
		throw new Error("the relay below printed no ANNOUNCE_TO_PARENT_LATER");
	for (const [index, name] of names.entries()) {
		const who = participants[index];
		if (who === undefined) throw new Error(`no participant ${name}`);
		await sayOne(gatherer, who, name, index);
	}
	return `${MANY} participants came at the relay below at once; the relay above refused it the announcements beyond its 5 at once, the relay below sent them again after ${captureGroup(later, 1)} ms one at a time, and the gatherer subscribed to all ${MANY} and numbered a line of each, 0 to ${MANY - 1}`;
}

/** The sender a line's payload names. */
function nameIn(hex: string): string {
	const text = Buffer.from(hex, "hex").toString("utf8");
	const name = /^\{"name":"([^"]*)"/.exec(text)?.[1];
	if (name === undefined) throw new Error(`no name in ${text}`);
	return name;
}

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-chat-edge-"));
	let summary: string;
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: ["own-rust"],
			verbose: options.verbose,
		});
		const edge = await startSecondOwnRelay(
			String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
			context,
		);
		const below: RunContext = { ...context, relayUrl: edge.url };
		const gatherer = spawnChatGatherer(
			context,
			"gatherer",
			join(directory, "chat.sqlite3"),
			[],
		);
		await gatherer.waitForLine(/^WATCHING prefix=xmoq\/chat$/);

		summary =
			options.case === "many"
				? await many(below, gatherer, edge.relay)
				: await two(context, edge, below, gatherer);
	} finally {
		await runCleanups();
		rmSync(directory, { recursive: true, force: true });
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ chat: "through-edge", case: options.case })}`,
	);
	log("driver", `SUMMARY ${summary}`);
}

async function two(
	context: RunContext,
	edge: Awaited<ReturnType<typeof startSecondOwnRelay>>,
	below: RunContext,
	gatherer: LoggedProcess,
): Promise<string> {
	if (options.case !== "two") {
		throw new Error(`--case is two or many, not ${options.case}`);
	}
	const a = await participant(below, gatherer, "a", "from a");
	const b = await participant(below, gatherer, "b", "from b");
	await edge.relay.waitForLines(
		/^OPEN_REQUEST session=\d+ own=\d+ PUBLISH_NAMESPACE/,
		2,
	);

	await gatherer.waitForLine(
		new RegExp(`^ROOM_PUBLISHED room="${ROOM}" forward=true`),
	);
	const live = spawnRustClient(
		below,
		"live",
		"own-rust",
		"subscribe",
		"lines",
		["--namespace", `xmoq/chat/${ROOM}`, "--objects", "4"],
	);
	await live.waitForLine(/^SUBSCRIBED /);
	const turns: [LoggedProcess, string, number][] = [
		[a, "a", 0],
		[b, "b", 1],
		[a, "a", 2],
		[b, "b", 3],
	];
	for (const [who, name, number] of turns) {
		await sayOne(gatherer, who, name, number);
	}
	const liveCode = await withTimeout(live.exited, "the live viewer to leave");
	if (liveCode !== 0) throw new Error(`the live viewer left with ${liveCode}`);
	// WHY in the order of the Groups and not of their arrival: each Group
	// comes on a subgroup stream of its own, and streams reach the viewer
	// in no order; that the lines are numbered in the order said is
	// checked as the gatherer takes each (`sayOne`).
	const seenLive = live
		.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
		.map((match) => ({
			group: Number(captureGroup(match, 1)),
			name: nameIn(captureGroup(match, 2)),
		}))
		.sort((one, other) => one.group - other.group)
		.map(({ group, name }) => `${group}:${name}`)
		.join(" ");
	expect("the live viewer got", seenLive, "0:a 1:b 2:a 3:b");

	const late = spawnRustClient(below, "late", "own-rust", "fetch", "lines", [
		"--namespace",
		`xmoq/chat/${ROOM}`,
		"--fetch-start",
		"0,0",
		"--fetch-end",
		"100,0",
	]);
	const [ok] = await late.waitForLine(
		/^FETCH_OK request_id=\d+ end_group=(\d+) end_object=(\d+)/,
	);
	const lateCode = await withTimeout(late.exited, "the late viewer to leave");
	if (lateCode !== 0) throw new Error(`the late viewer left with ${lateCode}`);
	const end = ok.replace(/^.*end_group=(\d+) end_object=(\d+).*$/, "{$1, $2}");
	expect("the late viewer's FETCH_OK", end, "{3, 1}");
	const fetched = late
		.matches(/^FETCH_OBJECT request_id=\d+ group=(\d+) /)
		.map((match) => captureGroup(match, 1))
		.join(",");
	expect("the late viewer fetched", fetched, "0,1,2,3");

	a.child.kill("SIGTERM");
	await edge.relay.waitForLine(/^CANCEL_OWN_REQUEST session=\d+ own=\d+/);
	await context.relay.waitForLine(
		/^SEND session=\d+ stream=\d+ NAMESPACE_DONE /,
	);

	return `the gatherer at the relay above was told of a and b at the relay below, which announced their namespaces to it once each, and subscribed to them through it; it numbered their lines 0 to 3, which a viewer at the relay below got live (${seenLive}) and a late one fetched (${fetched}, FETCH_OK ${end}); when a left, the relay below withdrew its namespace from it and the gatherer was told`;
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED chat through an edge: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
