// Namespace discovery through a relay (draft 18 §6.1, §10.16 to §10.18): a
// watcher sends SUBSCRIBE_NAMESPACE for the prefix (xmoq) and is told of the
// namespace (xmoq, phase0) when an own-rust publisher announces it, and of
// its end when the publisher goes. The watcher is own-rust, or with
// `--watcher own-browser` a page (`MoqtConnection.subscribeNamespace`).
//
// Usage: bun namespaces.ts --relay own|moqtail [--watcher own-rust|own-browser] [--port N]
//
// What has to be seen: the watcher prints `NAMESPACES_SUBSCRIBED` (the
// relay's REQUEST_OK), then `NAMESPACE namespace=xmoq/phase0` once the
// publisher has announced, then `NAMESPACE_DONE namespace=xmoq/phase0` once
// the publisher's session is gone, and leaves with 0 when its input ends.
//
// With `--watcher own-browser`, written before its first run (2026-10-25):
// the page's lines are the same three, in that order and no others, and
// after it cancels the request (`stopWatchingNamespaces`) the fourth is
// `NAMESPACES_ENDED cancelled`. A second SUBSCRIBE_NAMESPACE for the same
// prefix on the same connection is then answered: the relay let go of the
// first, which it would otherwise refuse the second for (PREFIX_OVERLAP,
// §10.18).

import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	log,
	openConnectedPage,
	pollFor,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type { OwnPageExtras, XmoqPage } from "./xmoq-page.ts";

declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		watcher: { type: "string", default: "own-rust" },
		port: { type: "string", default: "24398" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

/** What a watcher is asked for, the same for a Rust client and a page. */
interface Watcher {
	/** Resolves with the first line that matches, among those printed so far and later. */
	line(pattern: RegExp): Promise<string>;
	/** Ends the watcher; what it said about its end. */
	leave(): Promise<string>;
}

async function rustWatcher(
	context: Awaited<ReturnType<typeof startRun>>,
): Promise<Watcher> {
	// The prefix is the first field of the publisher's namespace.
	const watcher = spawnRustClient(
		context,
		"watch:own-rust",
		"own-rust",
		"subscribe-namespace",
		TRACK,
		["--namespace", "xmoq"],
	);
	return {
		line: async (pattern) => (await watcher.waitForLine(pattern))[0],
		leave: async () => {
			watcher.child.stdin.end();
			const code = await withTimeout(watcher.exited, "the watcher to leave");
			if (code !== 0) throw new Error(`the watcher left with ${code}, not 0`);
			return "it left with 0";
		},
	};
}

async function pageWatcher(
	context: Awaited<ReturnType<typeof startRun>>,
): Promise<Watcher> {
	const page: Page = await openConnectedPage(
		context,
		"watch:own-browser",
		"own-browser",
	);
	await withTimeout(
		page.evaluate(() => window.xmoqOwn.watchNamespaces("xmoq")),
		"the page's SUBSCRIBE_NAMESPACE to be answered",
	);
	const lines = (): Promise<string[]> =>
		page.evaluate(() => window.xmoqOwn.namespaceLines());
	return {
		line: (pattern) =>
			pollFor(
				async () => (await lines()).find((line) => pattern.test(line)),
				`the page to have a line ${pattern}`,
			),
		leave: async () => {
			await page.evaluate(() => window.xmoqOwn.stopWatchingNamespaces());
			const all = await pollFor(async () => {
				const now = await lines();
				return now.length >= 4 ? now : undefined;
			}, "the page's request to end");
			const wanted = [
				"NAMESPACES_SUBSCRIBED",
				"NAMESPACE namespace=xmoq/phase0",
				"NAMESPACE_DONE namespace=xmoq/phase0",
				"NAMESPACES_ENDED cancelled",
			];
			if (JSON.stringify(all) !== JSON.stringify(wanted)) {
				throw new Error(`the page's lines: ${JSON.stringify(all)}`);
			}
			await withTimeout(
				page.evaluate(() => window.xmoqOwn.watchNamespaces("xmoq")),
				"the page's second SUBSCRIBE_NAMESPACE to be answered",
			);
			return `its lines were ${wanted.length}, the last "NAMESPACES_ENDED cancelled", and a second SUBSCRIBE_NAMESPACE (xmoq) was answered after it`;
		},
	};
}

async function main(): Promise<void> {
	const { relay, watcher: kind } = options;
	if (relay !== "own" && relay !== "moqtail") {
		throw new Error(`--relay is own or moqtail, not ${relay}`);
	}
	if (kind !== "own-rust" && kind !== "own-browser") {
		throw new Error(`--watcher is own-rust or own-browser, not ${kind}`);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay,
			port: options.port,
			clients:
				kind === "own-browser" ? ["own-rust", "own-browser"] : ["own-rust"],
			verbose: options.verbose,
		});
		const watcher =
			kind === "own-browser"
				? await pageWatcher(context)
				: await rustWatcher(context);
		await watcher.line(/^NAMESPACES_SUBSCRIBED$/);
		const publisher = spawnRustClient(
			context,
			"pub:own-rust",
			"own-rust",
			"publish-namespace",
			TRACK,
			["--payload-hex", "00"],
		);
		await publisher.waitForLine(/^NAMESPACE_PUBLISHED/);
		const told = await watcher.line(/^NAMESPACE namespace=xmoq\/phase0$/);
		// Its input ends: it leaves, and its announcement with its session.
		publisher.child.stdin.end();
		await withTimeout(publisher.exited, "the publisher to leave");
		const done = await watcher.line(/^NAMESPACE_DONE namespace=xmoq\/phase0$/);
		const left = await watcher.leave();
		summary = `through the ${relay} relay, ${kind}'s SUBSCRIBE_NAMESPACE (xmoq) was answered, told "${told}" when the publisher announced and "${done}" when it left; ${left}`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ relay, watcher: kind })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED namespaces: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
