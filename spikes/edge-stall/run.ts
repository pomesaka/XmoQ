// How long a viewer's picture stands still when the relay above its relay goes and comes back,
// and where the time goes. A page publishes the bars at the relay of the run (keepPublishing),
// another plays them (keepPlaying) at an own relay below it (`--upstream`, the run's port plus
// MOQLIVEMOCK_SIDE_PORT_OFFSET), or, with `--through origin`, at the relay of the run itself, to
// compare. Once the viewer plays, the relay of the run is stopped (`--stop sigint`, which sends
// GOAWAY and closes, or `--stop sigkill`, which vanishes), kept down for `--down-ms`, and started
// anew on its port; the run ends when the viewer has drawn 25 frames after the stop.
//
//   bun spikes/edge-stall/run.ts --through edge|origin --stop sigint|sigkill --down-ms N [--port N]
//
// Prints one line, STALL {...}: every time is in milliseconds from the stop.
//   stall_ms: the longest time between two frames the viewer drew, from a second before the stop on
//   last_frame_ms, first_frame_ms: the frames on either side of it
//   down_ms, up_ms: when the relay was gone, and when it printed that it listens again
//   edge: when the relay below printed that its session upstream ended, each CONNECT_UPSTREAM,
//     UPSTREAM_FAILED and UPSTREAM_CONNECTED after the stop
//   states: when each state of the two pages first showed (keepPublishing, keepPlaying), as seen
//     by asking them every 50 ms
// A throwaway: not part of the interop run.

import { parseArgs } from "node:util";
import {
	log,
	MOQLIVEMOCK_SIDE_PORT_OFFSET,
	openConnectedPage,
	runCleanups,
	startRun,
	startSecondOwnRelay,
	stopEverythingOnExit,
	withTimeout,
} from "../../interop/harness.ts";
import type {
	OwnPageExtras,
	PageKeptReport,
	PageVideoSetup,
	XmoqPage,
} from "../../interop/xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		through: { type: "string", default: "edge" },
		stop: { type: "string", default: "sigint" },
		"down-ms": { type: "string", default: "0" },
		port: { type: "string", default: "34500" },
	},
});

stopEverythingOnExit();

const NAMESPACE = "xmoq/video";
/** The bars of `interop/video.ts`: 320x180, 25 frames a second, a key frame a second. */
const BARS: PageVideoSetup = {
	trackName: "video",
	codec: "avc1.42E01F",
	catalogCodec: "avc3.42E01F",
	width: 320,
	height: 180,
	framerate: 25,
	bitrate: 500_000,
	keyFrameIntervalMs: 1000,
	firstTimestamp: 5_000_000,
	noise: false,
	mapping: "stream-per-group",
};

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

async function main(): Promise<void> {
	const throughEdge = options.through === "edge";
	const downMs = Number(options["down-ms"]);
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: false,
	});
	const edge = throughEdge
		? await startSecondOwnRelay(
				String(Number(options.port) + MOQLIVEMOCK_SIDE_PORT_OFFSET),
				context,
			)
		: undefined;
	const viewerUrl = edge?.url ?? context.relayUrl;
	const publisher = await openConnectedPage(context, "publisher", "own-browser");
	const viewer = await openConnectedPage(context, "viewer", "own-browser");
	await publisher.evaluate(
		([url, hash, namespace, setup]) => window.xmoqOwn.keepVideoPublished(url, hash, namespace, setup),
		[context.relayUrl, context.certSha256Hex, NAMESPACE, BARS] satisfies [
			string,
			string,
			string,
			PageVideoSetup,
		],
	);
	await viewer.evaluate(
		([url, hash, namespace, name, codec]) =>
			window.xmoqOwn.keepVideoPlayed(url, hash, namespace, name, codec),
		[viewerUrl, context.certSha256Hex, NAMESPACE, BARS.trackName, BARS.catalogCodec] satisfies [
			string,
			string,
			string,
			string,
			string,
		],
	);
	const report = async (): Promise<{ publishing: string[]; playing: string[]; shown: PageKeptReport["shown"] }> => {
		const [published, played] = await Promise.all([
			publisher.evaluate(() => window.xmoqOwn.keptReport()),
			viewer.evaluate(() => window.xmoqOwn.keptReport()),
		]);
		return { publishing: published.publishing, playing: played.playing, shown: played.shown };
	};
	// Playing for 3 seconds before the stop.
	for (;;) {
		const now = await report();
		if (now.shown.length >= 75) break;
		await sleep(100);
	}
	const before = await report();

	const edgeTimes: Record<string, number> = {};
	const stamp = (name: string, promise: Promise<unknown>): void => {
		promise.then(
			() => {
				edgeTimes[name] = Date.now() - stopAt;
			},
			() => {},
		);
	};
	let upstreamSession = "";
	if (edge !== undefined) {
		const [connected] = await edge.relay.waitForLines(/^UPSTREAM_CONNECTED session=(\d+)/, 1);
		upstreamSession = connected?.[1] ?? "";
	}
	const states: { at: number; who: string; state: string }[] = [];
	let seen = { publishing: before.publishing.length, playing: before.playing.length };
	let watching = true;

	const stopAt = Date.now();
	if (edge !== undefined) {
		const long = 60_000;
		stamp("upstream_session_ended", edge.relay.waitForLines(new RegExp(`^SESSION_CLOSED session=${upstreamSession}$`), 1, long));
		for (const nth of [2, 3, 4]) {
			stamp(`connect_upstream_${nth}`, edge.relay.waitForLines(/^CONNECT_UPSTREAM/, nth, long));
		}
		for (const nth of [1, 2]) {
			stamp(`upstream_failed_${nth}`, edge.relay.waitForLines(/^UPSTREAM_FAILED/, nth, long));
		}
		stamp("upstream_connected_2", edge.relay.waitForLines(/^UPSTREAM_CONNECTED/, 2, long));
	}
	const watcher = (async () => {
		while (watching) {
			const now = await report().catch(() => undefined);
			if (now !== undefined) {
				const at = Date.now() - stopAt;
				for (const state of now.publishing.slice(seen.publishing)) states.push({ at, who: "publisher", state });
				for (const state of now.playing.slice(seen.playing)) states.push({ at, who: "viewer", state });
				seen = { publishing: now.publishing.length, playing: now.playing.length };
			}
			await sleep(50);
		}
	})();
	if (options.stop === "sigkill") {
		context.relay.child.kill("SIGKILL");
		await context.relay.exited;
	} else {
		await context.stopRelay();
	}
	const downAt = Date.now() - stopAt;
	await sleep(downMs);
	await withTimeout(context.startRelayAnew(), "the relay to start anew");
	const upAt = Date.now() - stopAt;
	let after: Awaited<ReturnType<typeof report>>;
	for (;;) {
		after = await report();
		if (after.shown.filter((frame) => frame.at > stopAt).length >= 25) break;
		if (Date.now() - stopAt > 90_000) throw new Error("the viewer drew no 25 frames within 90 s of the stop");
		await sleep(100);
	}
	watching = false;
	await watcher;
	const frames = after.shown.filter((frame) => frame.at >= stopAt - 1000);
	let stall = { ms: 0, last: 0, first: 0 };
	for (let index = 1; index < frames.length; index++) {
		const gap = (frames[index]?.at ?? 0) - (frames[index - 1]?.at ?? 0);
		if (gap > stall.ms) {
			stall = { ms: gap, last: (frames[index - 1]?.at ?? 0) - stopAt, first: (frames[index]?.at ?? 0) - stopAt };
		}
	}
	const result = {
		through: options.through,
		stop: options.stop,
		down_ms_asked: downMs,
		stall_ms: stall.ms,
		last_frame_ms: stall.last,
		first_frame_ms: stall.first,
		down_ms: downAt,
		up_ms: upAt,
		edge: edgeTimes,
		states,
	};
	log("driver", `STALL ${JSON.stringify(result)}`);
}

main()
	.catch((error: unknown) => {
		log("driver", `FAILED ${error instanceof Error ? error.message : String(error)}`);
		process.exitCode = 1;
	})
	.finally(() => runCleanups());
