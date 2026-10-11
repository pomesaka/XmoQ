// Starts the own relay and a page of Chromium that encodes and publishes
// video (the page of interop/video.ts, bars, 320x180 at 25 frames a second),
// prints where the relay is, and stays until it is stopped.
//
// Usage: bun spikes/kitty-graphics/publish-page.ts [--port N]

import { parseArgs } from "node:util";
import {
	log,
	openConnectedPage,
	pollFor,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "../../interop/harness.ts";
import type {
	OwnPageExtras,
	PageVideoSetup,
} from "../../interop/xmoq-page.ts";

declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: { port: { type: "string", default: "48850" } },
});

stopEverythingOnExit();

const NAMESPACE = "xmoq/video";
const VIDEO: PageVideoSetup = {
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

const context = await startRun({
	relay: "own",
	port: options.port,
	clients: ["own-browser", "own-rust"],
	verbose: false,
});
const publisher = await openConnectedPage(context, "publisher", "own-browser");
await withTimeout(
	publisher.evaluate(
		([namespace, setup]) => window.xmoqOwn.publishVideo(namespace, setup),
		[NAMESPACE, VIDEO] satisfies [string, PageVideoSetup],
	),
	"the page to publish its video",
);
await pollFor(
	async () =>
		(await publisher.evaluate(() => window.xmoqOwn.videoPublishReport()))
			.painted >= 30
			? true
			: undefined,
	"the publisher to paint 30 frames",
);
log(
	"driver",
	`READY url=${context.relayUrl} cert=${context.certSha256Hex} namespace=${NAMESPACE} track=${VIDEO.trackName}`,
);
// Until a signal: `stopEverythingOnExit` stops the page and the relay.
setInterval(() => {}, 60_000);
