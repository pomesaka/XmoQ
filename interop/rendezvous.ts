// A SUBSCRIBE that comes before its publisher: with RENDEZVOUS_TIMEOUT the
// relay holds it (§10.2.6 of draft-ietf-moq-transport-18).
//
// Usage: bun rendezvous.ts --case <case> --subscriber own-browser|own-rust [--relay own] [--port N]
//
// Through the own relay. (The row "A SUBSCRIBE for a track nobody
// publishes" of docs/moqt-implementation-notes.md.)
//
//   publisher-comes   The subscriber sends its SUBSCRIBE with a
//                     RENDEZVOUS_TIMEOUT of 20 seconds while nobody
//                     publishes the track. The relay holds it
//                     (`HOLD_REQUEST session=0 stream=S SUBSCRIBE`) and
//                     answers nothing. Two seconds later an own-rust
//                     publisher sends PUBLISH for the track and one Object:
//                     the relay answers SUBSCRIBE_OK, and the subscriber has
//                     the Object.
//   nobody-comes      The same SUBSCRIBE with 1.5 seconds, and no publisher.
//                     The relay answers REQUEST_ERROR with TIMEOUT (0x2)
//                     between 1.5 and 4 seconds after the SUBSCRIBE, and
//                     not before. The session is not closed over it.
//   viewer-before-the-broadcast
//                     The terminal viewer (`xmoq-watch
//                     --rendezvous-timeout-ms 20000`) comes first. The
//                     relay holds its SUBSCRIBE for the catalog and answers
//                     it nothing. Two seconds later an own page begins to
//                     publish a catalog, video and sound: the viewer finds
//                     the tracks and plays them (100 frames or more drawn,
//                     100 packets of sound or more) and ends of itself.
//   not-asked         The SUBSCRIBE without the parameter: DOES_NOT_EXIST
//                     (0x10) at once, as before.

import { parseArgs } from "node:util";
import {
	log,
	NAMESPACE,
	openConnectedPage,
	type RunContext,
	runCleanups,
	spawnRustClient,
	spawnTerminalViewer,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		case: { type: "string", default: "" },
		subscriber: { type: "string", default: "own-rust" },
		port: { type: "string", default: "24398" },
		relay: { type: "string", default: "own" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const PAYLOAD_HEX = "72656e64657a766f7573";
const wait = (ms: number): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, ms));

/** A subscriber that has sent its SUBSCRIBE: how that ended, and the payload of the Object it got. */
interface Asked {
	/** "ok", or what the subscriber said of the refusal. */
	answer: Promise<string>;
	object(): Promise<string>;
}

async function subscribe(
	context: RunContext,
	subscriber: string,
	rendezvousMs: number | undefined,
): Promise<Asked> {
	if (subscriber === "own-browser") {
		const page = await openConnectedPage(
			context,
			"sub:own-browser",
			"own-browser",
		);
		const answer = page
			.evaluate(
				([namespace, track, rendezvous]) =>
					window.xmoq.subscribe(namespace, track, rendezvous),
				[NAMESPACE, TRACK, rendezvousMs] satisfies [
					string,
					string,
					number | undefined,
				],
			)
			.then(
				() => "ok",
				(error: unknown) => String(error),
			);
		return {
			answer,
			object: async () =>
				(await page.evaluate(() => window.xmoq.firstObject())).payloadHex,
		};
	}
	if (subscriber !== "own-rust") {
		throw new Error(
			`--subscriber is own-browser or own-rust, not ${subscriber}`,
		);
	}
	const rust = spawnRustClient(
		context,
		"sub:own-rust",
		"own-rust",
		"subscribe",
		TRACK,
		[
			...(rendezvousMs === undefined
				? []
				: ["--rendezvous-timeout-ms", String(rendezvousMs)]),
			"--timeout-secs",
			"40",
		],
	);
	const answer = Promise.race([
		rust.waitForLine(/^SUBSCRIBED /, 40_000).then(() => "ok"),
		rust.exited.then(
			() =>
				rust.matches(/^.*(?:rejected|refused|error_code).*$/i)[0]?.[0] ??
				"it left",
		),
	]);
	return {
		answer,
		object: async () => {
			const [, payload] = await rust.waitForLine(
				/^OBJECT .*payload_hex=([0-9a-f]*)$/,
			);
			return payload ?? "";
		},
	};
}

/** The second session of the run is the subscriber's where a publisher is there first; here the subscriber is the first. */
const HELD = /^HOLD_REQUEST session=0 stream=(\d+) SUBSCRIBE$/;
const answered = (stream: string): RegExp =>
	new RegExp(
		`^SEND session=0 stream=${stream} (SUBSCRIBE_OK|REQUEST_ERROR.*)$`,
	);

async function publisherComes(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const asked = await subscribe(context, subscriber, 20_000);
	const [, stream = ""] = await context.relay.waitForLine(HELD);
	await wait(2000);
	if (context.relay.hasPrinted(answered(stream))) {
		throw new Error(
			"the relay was to answer nothing while nobody published the track",
		);
	}
	const publisher = spawnRustClient(
		context,
		"pub:own-rust",
		"own-rust",
		"publish",
		TRACK,
		["--payload-hex", PAYLOAD_HEX, "--timeout-secs", "40"],
	);
	await publisher.waitForLine(/^PUBLISHED /);
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be answered",
	);
	if (answer !== "ok") {
		throw new Error(
			`the held SUBSCRIBE was to be accepted when the publisher came: ${answer}`,
		);
	}
	await context.relay.waitForLine(
		new RegExp(`^SEND session=0 stream=${stream} SUBSCRIBE_OK`),
	);
	publisher.child.stdin.write("send\n");
	const payload = await withTimeout(
		asked.object(),
		"the Object of the publisher that came later",
	);
	if (payload !== PAYLOAD_HEX) {
		throw new Error(
			`the subscriber was to get the publisher's Object, not ${payload}`,
		);
	}
	return `${subscriber} subscribed 2 s before anybody published the track, with a RENDEZVOUS_TIMEOUT of 20 s: the relay held the SUBSCRIBE, answered SUBSCRIBE_OK when own-rust sent its PUBLISH, and the subscriber got the Object`;
}

async function nobodyComes(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const began = Date.now();
	const asked = await subscribe(context, subscriber, 1500);
	const [, stream = ""] = await context.relay.waitForLine(HELD);
	const answer = await withTimeout(
		asked.answer,
		"the held SUBSCRIBE to be refused",
	);
	const after = Date.now() - began;
	const [, sent = ""] = await context.relay.waitForLine(answered(stream));
	if (!/^REQUEST_ERROR error_code=0x2\b/.test(sent)) {
		throw new Error(
			`the relay was to answer REQUEST_ERROR with TIMEOUT (0x2), not ${sent}`,
		);
	}
	if (answer === "ok" || after < 1500 || after > 5000) {
		throw new Error(
			`the subscriber was to be refused between 1.5 and 5 s, not "${answer}" after ${after} ms`,
		);
	}
	if (context.relay.hasPrinted(/^CLOSE_SESSION /)) {
		throw new Error("the relay was to close no session over it");
	}
	return `${subscriber} subscribed to a track nobody publishes with a RENDEZVOUS_TIMEOUT of 1.5 s: the relay held the SUBSCRIBE and refused it with TIMEOUT (0x2) ${after} ms after the subscriber began: ${answer}`;
}

async function notAsked(
	context: RunContext,
	subscriber: string,
): Promise<string> {
	const asked = await subscribe(context, subscriber, undefined);
	const answer = await withTimeout(asked.answer, "the SUBSCRIBE to be refused");
	const [, sent = ""] = await context.relay.waitForLine(
		/^SEND session=0 stream=\d+ (SUBSCRIBE_OK|REQUEST_ERROR.*)$/,
	);
	if (!/^REQUEST_ERROR error_code=0x10\b/.test(sent) || answer === "ok") {
		throw new Error(
			`the relay was to answer DOES_NOT_EXIST (0x10) at once, not ${sent}`,
		);
	}
	if (context.relay.hasPrinted(/^HOLD_REQUEST /)) {
		throw new Error("the relay was to hold nothing");
	}
	return `${subscriber} subscribed to a track nobody publishes without RENDEZVOUS_TIMEOUT, and was refused with DOES_NOT_EXIST (0x10) at once: ${answer}`;
}

/**
 * The terminal viewer comes before the broadcast: it is held on the
 * SUBSCRIBE for the catalog, and plays when a page begins to publish.
 */
async function viewerBeforeTheBroadcast(context: RunContext): Promise<string> {
	const namespace = "xmoq/rendezvous";
	const seconds = 14;
	const viewer = spawnTerminalViewer(context, "viewer:xmoq-watch", [
		"--namespace",
		namespace,
		"--rendezvous-timeout-ms",
		"20000",
		"--seconds",
		String(seconds),
		"--sound",
		"off",
		"--out",
		"/dev/null",
		"--medium",
		"direct",
		"--fit",
		"none",
	]);
	await context.relay.waitForLine(HELD);
	await wait(2000);
	if (context.relay.hasPrinted(/^SEND session=0 stream=\d+ /)) {
		throw new Error(
			"the relay was to answer the viewer nothing before the broadcast",
		);
	}
	const publisher = await openConnectedPage(
		context,
		"publisher",
		"own-browser",
	);
	await withTimeout(
		publisher.evaluate(([name]) => window.xmoqOwn.publishBroadcast(name), [
			namespace,
		] satisfies [string]),
		"the page to publish its broadcast",
	);
	const [line] = await viewer.waitForLine(/^AV_PLAYED .*$/, 40_000);
	const field = (name: string): number =>
		Number(new RegExp(` ${name}=([0-9.]+)`).exec(line)?.[1] ?? Number.NaN);
	const drawn = field("video_drawn");
	const packets = field("audio_packets");
	if (!(drawn >= 100) || !(packets >= 100)) {
		throw new Error(
			`the viewer was to play the broadcast that came after it: ${drawn} frames drawn, ${packets} packets of sound. ${line}`,
		);
	}
	return `xmoq-watch came 2 s before the broadcast, with a RENDEZVOUS_TIMEOUT of 20 s on its SUBSCRIBEs: the relay held the one for the catalog, and when a page began to publish the viewer found the tracks and played them (${drawn} frames drawn, ${packets} packets of sound)`;
}

const CASES: Record<
	string,
	(context: RunContext, subscriber: string) => Promise<string>
> = {
	"publisher-comes": publisherComes,
	"nobody-comes": nobodyComes,
	"not-asked": notAsked,
	"viewer-before-the-broadcast": (context) => viewerBeforeTheBroadcast(context),
};

async function main(): Promise<void> {
	const run = CASES[options.case];
	if (run === undefined) throw new Error(`unknown --case ${options.case}`);
	let summary: string;
	try {
		const context = await startRun({
			relay: options.relay,
			port: options.port,
			clients:
				options.subscriber === "own-browser" ||
				options.case === "viewer-before-the-broadcast"
					? ["own-browser", "own-rust"]
					: ["own-rust"],
			verbose: options.verbose,
		});
		summary = await run(context, options.subscriber);
	} finally {
		await runCleanups();
	}
	log(
		"driver",
		`RESULT ${JSON.stringify({ case: options.case, subscriber: options.subscriber })}`,
	);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED rendezvous ${options.case}: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
