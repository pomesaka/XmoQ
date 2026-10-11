// Interop driver: the MSF catalog of a broadcast is published and read
// through `@xmoq/msf`, with a side that is not ours where there is one.
//
// Usage: bun catalog.ts --catalog <case> [--port N] [--verbose]
//
// What has to be seen in each case was written before the runs.
//
//   from-mlmpub   An own page reads the catalog of `mlmpub`, the publisher
//                 of moqlivemock, which is a server: the page connects to
//                 it, and no relay is in between. Its side port is the port
//                 plus 1000.
//
//     - One state, and no second one within a second: `mlmpub` sends its
//       one catalog Object, {0, 0}, in answer to the FETCH and on the
//       subscription as well (`internal/pub/pub.go` of v0.16.1: "for
//       backward compatibility with subscribe-only clients"), and the
//       Object is taken once.
//     - The catalog of that state has the tracks of
//       `fixtures/msf-01/catalog-moqlivemock.json`, which was taken from
//       the same publisher, field for field as `@xmoq/msf` reads them, at
//       {0, 0}, and is not outdated.
//     - The page says that its Joining FETCH was served and that the fetch
//       stream had one Object, and `mlmpub` says that it served the catalog
//       with a Joining FETCH: the catalog did not come by the subscription
//       alone. (The line of `mlmpub` has `joining=true` and no more; that
//       the FETCH is a Relative one with Joining Start 0 is in the tests of
//       `@xmoq/msf`.)
//
//   own           An own page publishes a catalog (`createCatalogPublisher`)
//                 and another reads it (`followCatalog`), through the own
//                 relay. Then the publisher replaces the catalog.
//
//     - The publisher's catalog is at Object 0 of a Group that is the time
//       in milliseconds (§6.1 of MSF: the driver's clock before and after).
//     - The reader has that catalog, at that Location, and its Joining
//       FETCH was served with one Object: the relay, which keeps nothing,
//       asked the publisher, and the publisher answered from what it holds.
//     - After the update, the reader has the new catalog at Object 0 of the
//       next Group, not outdated: it came on the subscription.
//
//   to-mlmsub     An own page publishes a catalog through the own relay,
//                 and `mlmsub` of moqlivemock reads it. `mlmsub` is built
//                 with a patch of one line, without which it connects to
//                 neither relay (peers/moqlivemock/README.md).
//
//     - `mlmsub` says that it fetched the catalog with a Joining FETCH, at
//       the Location the publisher has it.
//     - What it writes with `-catalogout` is read by `@xmoq/msf` as the
//       catalog that was published. (`mlmsub` writes the catalog anew; it
//       is not the bytes it got.)
//     - It then finds no track to play (it looks for names with `_avc`),
//       which is no part of this run; the driver stops it.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
// The entry of the package without its connection part, which needs the
// types of a browser.
import { type Catalog, readCatalog } from "@xmoq/msf/catalog";
import {
	captureGroup,
	log,
	NAMESPACE,
	openConnectedPage,
	pollFor,
	type RunContext,
	runCleanups,
	spawnMlmsub,
	startMoqlivemockPublisher,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, PageCatalogReport } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		catalog: { type: "string", default: "from-mlmpub" },
		port: { type: "string", default: "24380" },
		// The matrix gives every driver a relay; each case here has its own.
		relay: { type: "string", default: "" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The namespace of `mlmpub`'s LOC tracks and their MSF catalog. */
const MLMPUB_NAMESPACE = "mlm/msf/clear";

/**
 * The catalog an own page publishes: one LOC video track. The codec string
 * is provisional (open question on the codec); the catalog only carries it.
 */
const VIDEO = {
	name: "video",
	packaging: "loc",
	isLive: true,
	role: "video",
	codec: "avc3.42E01F",
	bitrate: 1000000,
	width: 1280,
	height: 720,
	framerate: 30,
};
const FIRST: Catalog = { version: "draft-01", tracks: [VIDEO] };
/** What replaces it: a second track. */
const SECOND: Catalog = {
	version: "draft-01",
	tracks: [VIDEO, { ...VIDEO, name: "video-low", bitrate: 300000 }],
};
/** What ends the broadcast (MSF §11.3): complete, and no tracks. */
const LAST: Catalog = { version: "draft-01", isComplete: true, tracks: [] };

type State = PageCatalogReport["states"][number];

/** Throws unless the state has this catalog at this Location, and is not outdated. */
function expectState(
	state: State | undefined,
	catalog: Catalog,
	at: [string, string],
	seen: string,
): void {
	if (
		state === undefined ||
		state.catalog === null ||
		state.at?.join(",") !== at.join(",") ||
		state.outdated !== null
	) {
		throw new Error(
			`the page has no catalog at {${at.join(", ")}} that is not outdated: ${seen}`,
		);
	}
	if (!Bun.deepEquals(state.catalog, catalog, true)) {
		throw new Error(
			`the page read another catalog than the one published: ${seen}`,
		);
	}
}

function follow(
	page: Awaited<ReturnType<typeof openConnectedPage>>,
	namespace: string,
	states: number,
): Promise<PageCatalogReport> {
	return withTimeout(
		page.evaluate(
			async ([name, count]) => {
				await window.xmoqOwn.followCatalog(name);
				return window.xmoqOwn.catalogReport(count);
			},
			[namespace, states] satisfies [string, number],
		),
		"the page to read the catalog",
	);
}

async function fromMlmpub(): Promise<string> {
	const captured = readCatalog(
		readFileSync(
			join(
				import.meta.dir,
				"..",
				"fixtures",
				"msf-01",
				"catalog-moqlivemock.json",
			),
		),
	);
	if (captured.type !== "catalog") {
		throw new Error(`the captured catalog is not read: ${captured.type}`);
	}
	const context = await startMoqlivemockPublisher({
		port: options.port,
		verbose: options.verbose,
	});
	const page = await openConnectedPage(context, "reader", "own-browser");
	await follow(page, MLMPUB_NAMESPACE, 1);
	// A second state would come at once: the same Object on the subscription.
	await new Promise((resolve) => setTimeout(resolve, 1000));
	const report = await page.evaluate(() => window.xmoqOwn.catalogReport(1));
	const seen = JSON.stringify(report).slice(0, 900);
	log("driver", `REPORT ${seen}`);
	const [state, ...more] = report.states;
	if (state === undefined || more.length > 0) {
		throw new Error(
			`the page had ${report.states.length} states of the catalog, not 1: ${seen}`,
		);
	}
	// `generatedAt` is the time the publisher made the catalog: a number,
	// and another one in every run.
	if (typeof state.catalog?.generatedAt !== "number") {
		throw new Error(`the catalog has no generatedAt: ${seen}`);
	}
	expectState(
		state,
		{ ...captured.catalog, generatedAt: state.catalog.generatedAt },
		["0", "0"],
		seen,
	);
	// The page's side of the FETCH: it read the fetch stream to its end,
	// and the stream had the one Object. Without this, a page that could
	// not read the stream would pass, as the same Object comes on the
	// subscription.
	if (report.joiningFetch !== "served 1") {
		throw new Error(
			`the Joining FETCH of the page was not served with one Object: ${report.joiningFetch}`,
		);
	}
	const served = await context.relay.waitForLine(
		/served catalog via FETCH.*joining=true/,
	);
	log("driver", `PUBLISHER ${served[0]}`);
	return `the catalog of ${captured.catalog.tracks.length} tracks at {0, 0}, read once; the page's Joining FETCH was served with one Object, and mlmpub says so too`;
}

/** Has an own page publish `FIRST` through the own relay, and gives where it is. */
async function publishFromAPage(context: RunContext): Promise<{
	page: Awaited<ReturnType<typeof openConnectedPage>>;
	at: [string, string];
}> {
	const page = await openConnectedPage(context, "publisher", "own-browser");
	const before = Date.now();
	const at = await withTimeout(
		page.evaluate(
			([namespace, catalog]) =>
				window.xmoqOwn.publishCatalog(namespace, catalog),
			[NAMESPACE, FIRST] satisfies [string, Catalog],
		),
		"the page to publish its catalog",
	);
	const group = Number(at[0]);
	if (at[1] !== "0" || group < before || group > Date.now()) {
		throw new Error(
			`the catalog is at {${at.join(", ")}}, which is not Object 0 of a Group that is the time in milliseconds (${before} to ${Date.now()})`,
		);
	}
	return { page, at };
}

async function own(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const publisher = await publishFromAPage(context);
	const reader = await openConnectedPage(context, "reader", "own-browser");
	const first = await follow(reader, NAMESPACE, 1);
	let seen = JSON.stringify(first).slice(0, 900);
	log("driver", `REPORT ${seen}`);
	expectState(first.states.at(0), FIRST, publisher.at, seen);
	if (first.states.length !== 1 || first.joiningFetch !== "served 1") {
		throw new Error(
			`the reader has ${first.states.length} states and a Joining FETCH that was ${first.joiningFetch}, not 1 and "served 1": ${seen}`,
		);
	}
	const next = await withTimeout(
		publisher.page.evaluate(
			([catalog]) => window.xmoqOwn.updateCatalog(catalog),
			[SECOND] satisfies [Catalog],
		),
		"the page to publish the catalog that replaces the first",
	);
	if (next.join(",") !== `${BigInt(publisher.at[0]) + 1n},0`) {
		throw new Error(
			`the catalog that replaces the one at {${publisher.at.join(", ")}} is at {${next.join(", ")}}, not at Object 0 of the next Group`,
		);
	}
	const second = await withTimeout(
		reader.evaluate(() => window.xmoqOwn.catalogReport(2)),
		"the reader to have the catalog that replaced the first",
	);
	seen = JSON.stringify(second).slice(0, 1200);
	log("driver", `REPORT ${seen}`);
	if (second.states.length !== 2) {
		throw new Error(
			`the reader has ${second.states.length} states, not 2: ${seen}`,
		);
	}
	expectState(second.states.at(1), SECOND, next, seen);
	// The last catalog and the end of the track, one right after the other.
	const last = await withTimeout(
		publisher.page.evaluate(([catalog]) => window.xmoqOwn.endCatalog(catalog), [
			LAST,
		] satisfies [Catalog]),
		"the page to publish its last catalog and end the catalog track",
	);
	if (last.join(",") !== `${BigInt(publisher.at[0]) + 2n},0`) {
		throw new Error(
			`the last catalog is at {${last.join(", ")}}, not at Object 0 of the Group after {${next.join(", ")}}`,
		);
	}
	const ended = await withTimeout(
		reader.evaluate(() => window.xmoqOwn.catalogEnded()),
		"the reader's subscription to the catalog track to end",
	);
	const third = await withTimeout(
		reader.evaluate(() => window.xmoqOwn.catalogReport(3)),
		"the reader to have the last catalog",
	);
	seen = JSON.stringify(third).slice(0, 1500);
	log("driver", `REPORT ${seen} ended=${ended}`);
	if (third.states.length !== 3) {
		throw new Error(
			`the reader has ${third.states.length} states, not 3: ${seen}`,
		);
	}
	expectState(third.states.at(2), LAST, last, seen);
	if (ended !== "publish-done 2") {
		throw new Error(
			`the reader's subscription ended with ${ended}, not with PUBLISH_DONE and TRACK_ENDED`,
		);
	}
	return `the catalog at {${publisher.at.join(", ")}} read with a Joining FETCH that was served with one Object; its replacement at {${next.join(", ")}} read from the subscription; the last one at {${last.join(", ")}}, published right before the end of the track, read before the subscription ended with TRACK_ENDED`;
}

async function toMlmsub(): Promise<string> {
	const context = await startRun({
		relay: "own",
		port: options.port,
		clients: ["own-browser"],
		verbose: options.verbose,
	});
	const publisher = await publishFromAPage(context);
	const { process: mlmsub, catalogOut } = spawnMlmsub(context, NAMESPACE);
	const fetched = await mlmsub.waitForLine(
		/fetched catalog via joining fetch.* groupID=(\d+) objectID=(\d+)/,
	);
	const at = [captureGroup(fetched, 1), captureGroup(fetched, 2)];
	if (at.join(",") !== publisher.at.join(",")) {
		throw new Error(
			`mlmsub fetched the catalog at {${at.join(", ")}}, not at {${publisher.at.join(", ")}}`,
		);
	}
	// It writes the file after that line.
	const written = await pollFor(() => {
		try {
			const bytes = readFileSync(catalogOut);
			// WHY read as a catalog here: the file can be seen while it is
			// being written.
			return readCatalog(bytes).type === "catalog" ? bytes : undefined;
		} catch {
			return undefined;
		}
	}, "mlmsub to write the catalog it read");
	const read = readCatalog(written);
	if (read.type !== "catalog" || !Bun.deepEquals(read.catalog, FIRST, true)) {
		throw new Error(
			`what mlmsub wrote is not the catalog that was published: ${new TextDecoder().decode(written).slice(0, 600)}`,
		);
	}
	return `mlmsub fetched the catalog at {${at.join(", ")}} with a Joining FETCH and wrote it out as it was published`;
}

const CASES: Record<string, () => Promise<string>> = {
	"from-mlmpub": fromMlmpub,
	own,
	"to-mlmsub": toMlmsub,
};

async function main(): Promise<void> {
	const run = CASES[options.catalog];
	if (run === undefined) {
		throw new Error(
			`--catalog "${options.catalog}" is not one of ${Object.keys(CASES).join(", ")}`,
		);
	}
	let summary: string;
	try {
		summary = await run();
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ catalog: options.catalog })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED ${options.catalog} catalog: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
