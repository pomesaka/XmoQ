// The gatherer of the chat (`crates/chat`, `xmoq-chat`) through a relay:
// own-rust clients are the participants of one room, each announcing
// `xmoq/chat/r/<name>` and sending a line on its track `said` for every line
// of its input; own-rust clients are the viewers of the room's `lines`.
//
// Usage: bun chat.ts --relay own|moqtail [--restart yes|no] [--port N] [--verbose]
//
// What has to be seen, written before the first run:
//
//   - The gatherer is told of every participant by its SUBSCRIBE_NAMESPACE
//     for `xmoq/chat`, and subscribes to each one's `said` (`SUBSCRIBED`).
//   - Lines of `a` and `b`, one after the other, are numbered 0 to 3 in the
//     order they reach the gatherer, and a viewer subscribed to `lines`
//     gets them live: Groups 0 to 3, whose JSON names a, b, a, b.
//   - `c` says a line of 513 bytes: dropped as `too-long`.
//   - `d` says 8 lines at once: 5 are taken (4 to 8) and 3 dropped as
//     `too-fast`.
//   - A viewer that comes then FETCHes 0 to 100 and gets Groups 0 to 8 from
//     the gatherer's store, with `FETCH_OK` ending at {8, 1} (§10.13).
//   - The gatherer is stopped and started again on the same store, with
//     `--fetch-lines 3`: it publishes the room with the largest line it holds
//     (8), follows the participants that are still there, and the line of a
//     new participant `e` is numbered 9.
//   - A viewer then FETCHes 0 to 100 and gets an End of Unknown Range up to
//     Group 6, then Groups 7 to 9 (the limit of 3), and `FETCH_OK` ends at
//     {9, 1}.
//
// Written before the runs after the review (2026-10-24):
//
//   - The second gatherer gathers room r alone (`--room r`): it watches
//     `xmoq/chat/r`, and a participant of room s is never asked for its
//     track.
//   - When every participant of r has left, the gatherer ends the room's
//     PUBLISH with PUBLISH_DONE TRACK_ENDED, counting the one stream it
//     opened (`ROOM_DONE room="r" status=0x2 streams=1`), and a FETCH for
//     the room is refused by the own relay with DOES_NOT_EXIST (0x10): no
//     publisher of it is left.
//   - `--restart no` (the run through moqtail's relay): no restart; the
//     gatherer runs with `--fetch-lines 6`, and the late viewer gets an End
//     of Unknown Range up to Group 2 and then Groups 3 to 8. WHY the run
//     through moqtail's relay has no restart: moqtail's relay 0.15.0 did not
//     answer the PUBLISH of a gatherer that came again at once, now and
//     then (docs/experiments/2026-10-24-chat-gatherer.md).
//
// Written before the run that asks again (2026-10-24):
//
//   - A participant whose subscription its publisher ends with PUBLISH_DONE
//     while its namespace stays announced is subscribed to again by the
//     gatherer (`ASKING_AGAIN ... in_ms=1000`, then a second `SUBSCRIBED`),
//     and its next line is taken: through moqtail's relay `b`'s, numbered 9,
//     before the late viewer, who then gets an End of Unknown Range up to
//     Group 3 and Groups 4 to 9; through the own relay `a`'s after the
//     restart, numbered 10. The room's
//     PUBLISH_DONE then counts the two streams the second gatherer opened
//     (`streams=2`).

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import {
	captureGroup,
	type LoggedProcess,
	log,
	pollFor,
	type RunContext,
	runCleanups,
	spawnChatGatherer,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24399" },
		restart: { type: "string", default: "yes" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const ROOM = "r";

/** What a participant says: the JSON of `said`, in hex for `--payload-hex`. */
function said(text: string): string {
	const json = JSON.stringify({ text, sentAt: 1_761_300_000_000 });
	return Buffer.from(json, "utf8").toString("hex");
}

/** A participant of the room, which says `text` for every line of its input. */
async function participant(
	context: RunContext,
	gatherer: LoggedProcess,
	name: string,
	text: string,
): Promise<LoggedProcess> {
	const spawned = spawnRustClient(
		context,
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
			"120",
		],
	);
	await spawned.waitForLine(/^NAMESPACE_PUBLISHED/);
	// The relay subscribes for the gatherer once the gatherer has.
	await spawned.waitForLine(/^SERVING /);
	await gatherer.waitForLine(subscribed(name));
	return spawned;
}

const subscribed = (name: string): RegExp =>
	new RegExp(`^SUBSCRIBED room="${ROOM}" name="${name}"$`);

const taken = (name: string): RegExp =>
	new RegExp(`^LINE room="${ROOM}" number=(\\d+) name="${name}" `);

/** Has `who` say one line, and waits for the gatherer to number it. */
async function sayOne(
	gatherer: LoggedProcess,
	who: LoggedProcess,
	name: string,
	number: number,
): Promise<void> {
	const before = gatherer.matches(taken(name)).length;
	who.child.stdin.write("line\n");
	const numbered = await pollFor(
		() => gatherer.matches(taken(name))[before],
		`the gatherer to take a line of ${name}`,
	);
	const got = Number(captureGroup(numbered, 1));
	if (got !== number) {
		throw new Error(`the line of ${name} was numbered ${got}, not ${number}`);
	}
}

/** The Groups and the senders an `OBJECT` line or a `FETCH_OBJECT` line names. */
function lineOf(match: RegExpMatchArray): { group: number; name: string } {
	const group = Number(captureGroup(match, 1));
	const hex = captureGroup(match, 2);
	// A FETCH_OBJECT shows the first 64 bytes alone: the name is in them.
	const text = Buffer.from(hex, "hex").toString("utf8");
	const name = /^\{"name":"([^"]*)"/.exec(text)?.[1];
	if (name === undefined) throw new Error(`no name in ${text}`);
	return { group, name };
}

/** Has a viewer FETCH the whole history, and says what it got. */
async function fetchAll(
	context: RunContext,
	label: string,
): Promise<{ end: string; groups: number[]; unknownThrough: string }> {
	const viewer = spawnRustClient(context, label, "own-rust", "fetch", "lines", [
		"--namespace",
		`xmoq/chat/${ROOM}`,
		"--fetch-start",
		"0,0",
		"--fetch-end",
		"100,0",
	]);
	const [ok] = await viewer.waitForLine(
		/^FETCH_OK request_id=\d+ end_group=(\d+) end_object=(\d+)/,
	);
	const [ended] = await viewer.waitForLine(/^FETCH_ENDED .*how=(\S+)/);
	if (!ended.includes("how=fin")) throw new Error(`the FETCH ended: ${ended}`);
	const code = await withTimeout(viewer.exited, `${label} to leave`);
	if (code !== 0) throw new Error(`${label} left with ${code}, not 0`);
	const groups = viewer
		.matches(/^FETCH_OBJECT request_id=\d+ group=(\d+) .*payload_hex=(\S*)$/)
		.map((match) => lineOf(match).group);
	const unknown = viewer.matches(
		/^FETCH_RANGE_END request_id=\d+ kind=unknown group=(\d+) object=0$/,
	);
	return {
		end: ok.replace(/^.*end_group=(\d+) end_object=(\d+).*$/, "{$1, $2}"),
		groups,
		unknownThrough:
			unknown[0] === undefined ? "none" : captureGroup(unknown[0], 1),
	};
}

/**
 * Has `who` end the gatherer's subscription with PUBLISH_DONE while its
 * namespace stays announced, waits for the gatherer to subscribe again, and
 * has `who` say a line that the gatherer numbers `number`. Says how long the
 * gatherer said it would wait.
 */
async function endAndGatherAgain(
	gatherer: LoggedProcess,
	who: LoggedProcess,
	name: string,
	number: number,
): Promise<string> {
	const served = who.matches(/^SERVING /).length;
	const subscriptions = gatherer.matches(subscribed(name)).length;
	who.child.stdin.write("publish-done\n");
	await who.waitForLine(/^PUBLISH_DONE_SENT$/);
	const asking = await gatherer.waitForLine(
		new RegExp(`^ASKING_AGAIN room="${ROOM}" name="${name}" in_ms=(\\d+)$`),
	);
	await pollFor(
		() => (who.matches(/^SERVING /).length > served ? true : undefined),
		`${name} to be asked for its track again`,
	);
	await pollFor(
		() =>
			gatherer.matches(subscribed(name)).length > subscriptions
				? true
				: undefined,
		`the gatherer to subscribe to ${name} again`,
	);
	await sayOne(gatherer, who, name, number);
	return captureGroup(asking, 1);
}

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

async function main(): Promise<void> {
	const relay = options.relay;
	if (relay !== "own" && relay !== "moqtail") {
		throw new Error(`--relay is own or moqtail, not ${relay}`);
	}
	const restart = options.restart;
	if (restart !== "yes" && restart !== "no") {
		throw new Error(`--restart is yes or no, not ${restart}`);
	}
	const directory = mkdtempSync(join(tmpdir(), "xmoq-chat-"));
	const store = join(directory, "chat.sqlite3");
	let summary: string;
	try {
		const context = await startRun({
			relay,
			port: options.port,
			clients: ["own-rust"],
			verbose: options.verbose,
		});
		const first = spawnChatGatherer(
			context,
			"gatherer",
			store,
			restart === "no" ? ["--fetch-lines", "6"] : [],
		);
		await first.waitForLine(/^WATCHING prefix=xmoq\/chat$/);

		const a = await participant(context, first, "a", "from a");
		const b = await participant(context, first, "b", "from b");
		await first.waitForLine(
			new RegExp(`^ROOM_PUBLISHED room="${ROOM}" forward=true`),
		);
		const live = spawnRustClient(
			context,
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
			await sayOne(first, who, name, number);
		}
		const liveCode = await withTimeout(live.exited, "the live viewer to leave");
		if (liveCode !== 0)
			throw new Error(`the live viewer left with ${liveCode}`);
		// WHY in the order of the Groups and not of their arrival: each Group
		// comes on a subgroup stream of its own, and streams reach the viewer
		// in no order; that the lines are numbered in the order said is
		// checked as the gatherer takes each (`sayOne`).
		const seenLive = live
			.matches(/^OBJECT group=(\d+) object=0 .*payload_hex=(\S*)$/)
			.map(lineOf)
			.sort((one, other) => one.group - other.group)
			.map(({ group, name }) => `${group}:${name}`)
			.join(" ");
		expect("the live viewer got", seenLive, "0:a 1:b 2:a 3:b");

		const c = await participant(context, first, "c", "x".repeat(513));
		c.child.stdin.write("line\n");
		await first.waitForLine(
			new RegExp(`^LINE_DROPPED room="${ROOM}" name="c" why=too-long `),
		);

		const d = await participant(context, first, "d", "fast");
		d.child.stdin.write("line\n".repeat(8));
		await pollFor(
			() =>
				first.matches(taken("d")).length +
					first.matches(/^LINE_DROPPED room="r" name="d" /).length >=
				8
					? true
					: undefined,
			"the gatherer to take or drop the 8 lines of d",
		);
		const ofD = first
			.matches(taken("d"))
			.map((match) => captureGroup(match, 1));
		expect("the lines of d that were taken", ofD.join(","), "4,5,6,7,8");
		expect(
			"the lines of d that were dropped as too fast",
			String(
				first.matches(/^LINE_DROPPED room="r" name="d" why=too-fast /).length,
			),
			"3",
		);

		// WHY before the late viewer through moqtail's relay: that run is
		// expected to fail at the late viewer's FETCH (`matrix.ts`).
		const pauseOfB =
			restart === "no" ? await endAndGatherAgain(first, b, "b", 9) : "1000";
		expect("the pause before b was asked again", pauseOfB, "1000");

		const before = await fetchAll(context, "late");
		expect(
			"the late viewer's FETCH_OK ends at",
			before.end,
			restart === "no" ? "{9, 1}" : "{8, 1}",
		);
		expect(
			"the late viewer was told of unknown lines up to",
			before.unknownThrough,
			restart === "no" ? "3" : "none",
		);
		expect(
			"the late viewer got",
			before.groups.join(","),
			restart === "no" ? "4,5,6,7,8,9" : "0,1,2,3,4,5,6,7,8",
		);
		if (restart === "no") {
			summary = `through the ${relay} relay, the gatherer numbered the lines of a and b 0 to 3 in the order they came and a viewer got them live (${seenLive}); a line of 513 bytes was dropped as too long; of 8 lines at once 5 were taken (${ofD.join(",")}) and 3 dropped as too fast; a late viewer's FETCH, cut to 6 lines, got an unknown range up to ${before.unknownThrough} and Groups ${before.groups.join(",")} (FETCH_OK ${before.end}), after b ended the gatherer's subscription with PUBLISH_DONE, was asked again after ${pauseOfB} ms, and its next line was numbered 9`;
			return;
		}

		await first.stop();
		const second = spawnChatGatherer(context, "gatherer-2", store, [
			"--fetch-lines",
			"3",
			"--room",
			ROOM,
		]);
		await second.waitForLine(/^WATCHING prefix=xmoq\/chat\/r$/);
		await second.waitForLine(/^STORE .* rooms=1$/);
		await second.waitForLine(
			new RegExp(`^ROOM_PUBLISHED room="${ROOM}" forward=true largest=8$`),
		);
		for (const name of ["a", "b", "c", "d"]) {
			await second.waitForLine(subscribed(name));
		}
		const elsewhere = spawnRustClient(
			context,
			"said:s/x",
			"own-rust",
			"publish-namespace",
			"said",
			[
				"--namespace",
				"xmoq/chat/s/x",
				"--payload-hex",
				said("in another room"),
				"--timeout-secs",
				"120",
			],
		);
		await elsewhere.waitForLine(/^NAMESPACE_PUBLISHED/);
		const e = await participant(context, second, "e", "after the restart");
		await sayOne(second, e, "e", 9);

		const after = await fetchAll(context, "after-restart");
		expect("the FETCH_OK after the restart ends at", after.end, "{9, 1}");
		expect(
			"the viewer after the restart was told of unknown lines up to",
			after.unknownThrough,
			"6",
		);
		expect("the viewer after the restart got", after.groups.join(","), "7,8,9");
		const pause = await endAndGatherAgain(second, a, "a", 10);
		expect("the pause before a was asked again", pause, "1000");

		for (const who of [a, b, c, d, e]) who.child.stdin.end();
		const [done] = await second.waitForLine(
			/^ROOM_DONE room="r" status=0x2 streams=\d+$/,
		);
		expect(
			"the room's PUBLISH_DONE",
			done,
			'ROOM_DONE room="r" status=0x2 streams=2',
		);
		const empty = spawnRustClient(
			context,
			"empty",
			"own-rust",
			"fetch",
			"lines",
			[
				"--namespace",
				`xmoq/chat/${ROOM}`,
				"--fetch-start",
				"0,0",
				"--fetch-end",
				"100,0",
			],
		);
		const refused = await empty.waitForLine(
			/^FETCH_REFUSED request_id=\d+ error_code=(\S+)/,
		);
		expect(
			"the FETCH for the room nobody is in was refused with",
			captureGroup(refused, 1),
			"0x10",
		);
		const askedElsewhere = elsewhere.matches(/^SERVING /).length;
		expect(
			"the participant of room s was asked for its track",
			String(askedElsewhere),
			"0",
		);
		if (second.matches(/room="s"/).length > 0) {
			throw new Error("the gatherer of room r took something of room s");
		}

		summary = `through the ${relay} relay, the gatherer numbered the lines of a and b 0 to 3 in the order they came and a viewer got them live (${seenLive}); a line of 513 bytes was dropped as too long; of 8 lines at once 5 were taken (${ofD.join(",")}) and 3 dropped as too fast; a late viewer fetched Groups ${before.groups.join(",")} (FETCH_OK ${before.end}); after a restart on the same store the next line was numbered 9, and a FETCH with a limit of 3 got an unknown range up to ${after.unknownThrough} and Groups ${after.groups.join(",")} (FETCH_OK ${after.end}); a ended the gatherer's subscription with PUBLISH_DONE, was asked again after ${pause} ms, and its next line was numbered 10; the gatherer of room r alone was never told of room s; when everybody left, it ended the room (${done}), and a FETCH for it was refused with ${captureGroup(refused, 1)}`;
	} finally {
		await runCleanups();
		rmSync(directory, { recursive: true, force: true });
	}
	log("driver", `RESULT ${JSON.stringify({ relay })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED chat: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
