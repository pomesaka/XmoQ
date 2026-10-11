// Interop driver: an own page reads the catalog of a publisher that is not
// ours, with `followCatalog` of @xmoq/msf: a SUBSCRIBE to the catalog track
// and a Joining FETCH with offset 0 (draft-ietf-moq-msf-01 §5).
//
// Usage: bun catalog.ts [--port N] [--verbose]
//
// The publisher is `mlmpub` of moqlivemock, which is a server: the page
// connects to it, and no relay is in between. Its side port is the port
// plus 1000.
//
// What has to be seen, written before the run:
//
// - One state, and no second one within a second: `mlmpub` sends its one
//   catalog Object, {0, 0}, in answer to the FETCH and on the subscription
//   as well (`internal/pub/pub.go` of v0.16.1: "for backward compatibility
//   with subscribe-only clients"), and the Object is taken once.
// - The catalog of that state has the tracks of
//   `fixtures/msf-01/catalog-moqlivemock.json`, which was taken from the
//   same publisher, field for field as `@xmoq/msf` reads them, at {0, 0},
//   and is not outdated.
// - The page says that its Joining FETCH was served and that the fetch
//   stream had one Object, and `mlmpub` says that it served the catalog
//   with a Joining FETCH: the catalog did not come by the subscription
//   alone. (The line of `mlmpub` has `joining=true` and no more; that the
//   FETCH is a Relative one with Joining Start 0 is in the tests of
//   `@xmoq/msf`.)

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
// The entry of the package without its connection part, which needs the
// types of a browser.
import { readCatalog } from "@xmoq/msf/catalog";
import {
	log,
	openConnectedPage,
	runCleanups,
	startMoqlivemockPublisher,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras } from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "44380" },
		// The matrix gives every driver a relay; this one runs through none.
		relay: { type: "string", default: "none" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** The namespace of `mlmpub`'s LOC tracks and their MSF catalog. */
const NAMESPACE = "mlm/msf/clear";

async function main(): Promise<void> {
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
	let summary: string;
	try {
		const context = await startMoqlivemockPublisher({
			port: options.port,
			verbose: options.verbose,
		});
		const page = await openConnectedPage(context, "reader", "own-browser");
		const report = await withTimeout(
			page.evaluate(
				([namespace]) => window.xmoqOwn.followCatalog(namespace, 1000),
				[NAMESPACE] satisfies [string],
			),
			"the page to read the catalog",
		);
		const seen = JSON.stringify(report).slice(0, 900);
		log("driver", `REPORT ${seen}`);
		const [state, ...more] = report.states;
		if (state === undefined || more.length > 0) {
			throw new Error(
				`the page had ${report.states.length} states of the catalog, not 1: ${seen}`,
			);
		}
		const { catalog } = state;
		if (
			catalog === null ||
			state.at?.join(",") !== "0,0" ||
			state.outdated !== null
		) {
			throw new Error(
				`the page has no catalog at {0, 0} that is not outdated: ${seen}`,
			);
		}
		// `generatedAt` is the time the publisher made the catalog: a number,
		// and another one in every run.
		if (typeof catalog.generatedAt !== "number") {
			throw new Error(`the catalog has no generatedAt: ${seen}`);
		}
		const wanted = { ...captured.catalog, generatedAt: catalog.generatedAt };
		if (!Bun.deepEquals(catalog, wanted, true)) {
			throw new Error(
				`the page read another catalog than the one captured from this publisher: ${seen}`,
			);
		}
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
		summary = `the catalog of ${captured.catalog.tracks.length} tracks at {0, 0}, read once; the page's Joining FETCH was served with one Object, and mlmpub says so too`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ publisher: "mlmpub" })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED own-browser reads the catalog of mlmpub: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
