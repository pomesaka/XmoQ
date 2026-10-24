// Namespace discovery through a relay (draft 18 §6.1, §10.16 to §10.18): an
// own-rust client sends SUBSCRIBE_NAMESPACE for the prefix (xmoq) and is told
// of the namespace (xmoq, phase0) when an own-rust publisher announces it, and
// of its end when the publisher goes.
//
// Usage: bun namespaces.ts --relay own|moqtail [--port N]
//
// What has to be seen: the watcher prints `NAMESPACES_SUBSCRIBED` (the
// relay's REQUEST_OK), then `NAMESPACE namespace=xmoq/phase0` once the
// publisher has announced, then `NAMESPACE_DONE namespace=xmoq/phase0` once
// the publisher's session is gone, and leaves with 0 when its input ends.

import { parseArgs } from "node:util";
import {
	log,
	runCleanups,
	spawnRustClient,
	startRun,
	stopEverythingOnExit,
	TRACK,
	withTimeout,
} from "./harness.ts";

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24398" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

async function main(): Promise<void> {
	const relay = options.relay;
	if (relay !== "own" && relay !== "moqtail") {
		throw new Error(`--relay is own or moqtail, not ${relay}`);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay,
			port: options.port,
			clients: ["own-rust"],
			verbose: options.verbose,
		});
		// The prefix is the first field of the publisher's namespace.
		const watcher = spawnRustClient(
			context,
			"watch:own-rust",
			"own-rust",
			"subscribe-namespace",
			TRACK,
			["--namespace", "xmoq"],
		);
		await watcher.waitForLine(/^NAMESPACES_SUBSCRIBED$/);
		const publisher = spawnRustClient(
			context,
			"pub:own-rust",
			"own-rust",
			"publish-namespace",
			TRACK,
			["--payload-hex", "00"],
		);
		await publisher.waitForLine(/^NAMESPACE_PUBLISHED/);
		const [told] = await watcher.waitForLine(
			/^NAMESPACE namespace=xmoq\/phase0$/,
		);
		// Its input ends: it leaves, and its announcement with its session.
		publisher.child.stdin.end();
		await withTimeout(publisher.exited, "the publisher to leave");
		const [done] = await watcher.waitForLine(
			/^NAMESPACE_DONE namespace=xmoq\/phase0$/,
		);
		watcher.child.stdin.end();
		const code = await withTimeout(watcher.exited, "the watcher to leave");
		if (code !== 0) throw new Error(`the watcher left with ${code}, not 0`);
		summary = `through the ${relay} relay, SUBSCRIBE_NAMESPACE (xmoq) was answered, told "${told}" when the publisher announced and "${done}" when it left`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ relay })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED namespaces: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
