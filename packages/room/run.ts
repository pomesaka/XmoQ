// Runs the room Worker locally with Miniflare. Nothing is deployed and no
// Cloudflare account is touched.
//
// Usage: node run.ts [--port 8787]
//
// WHY node and not bun: the room ran under Node.js since the spike, and
// Miniflare under Bun was not tried. Node.js 24 runs this file by stripping
// its types. WHY `bun build` first:
// workerd runs JavaScript, and `src/worker.ts` and what it imports are
// TypeScript; the bundle is one module, in a directory of its own.
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Miniflare } from "miniflare";
import { broadcastNamespaces } from "./src/core/rules.ts";
import { keyOfSetting } from "./src/token.ts";

// Before anything is started: a key that is set and is none stops the
// rooms, as it stops the relay (`keyOfSetting`), and so do namespaces of
// broadcasts that are none (`broadcastNamespaces`).
try {
	keyOfSetting(process.env.AUTH_KEY_HEX);
	broadcastNamespaces(process.env.BROADCAST_NAMESPACES);
} catch (error) {
	console.error(String(error));
	process.exit(1);
}
const at = process.argv.indexOf("--port");
const port = at === -1 ? 8787 : Number(process.argv[at + 1]);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
	console.error(
		`--port is a port from 1 to 65535, not ${process.argv[at + 1]}`,
	);
	process.exit(1);
}
const out = mkdtempSync(join(tmpdir(), "xmoq-room-"));
// Removed however this ends.
process.on("exit", () => rmSync(out, { recursive: true, force: true }));
const built = spawnSync(
	"bun",
	[
		"build",
		new URL("./src/worker.ts", import.meta.url).pathname,
		"--target=browser",
		"--format=esm",
		`--outdir=${out}`,
	],
	{ stdio: ["ignore", "ignore", "inherit"] },
);
if (built.status !== 0) {
	console.error("the room Worker could not be built");
	process.exit(1);
}
const miniflare = new Miniflare({
	modules: true,
	scriptPath: join(out, "worker.js"),
	// The bundle is outside the directory this runs in, which workerd does
	// not let a module path leave without this.
	modulesRoot: out,
	compatibilityDate: "2026-07-01",
	durableObjects: { ROOMS: { className: "Room", useSQLite: true } },
	// The key the rooms share with the relay, for the tokens; none without.
	// REACTION_BATCH_MS: a trial, the reactions of that long as one message.
	// RELAY_URL and RELAY_CERT_SHA256_HEX: the relay the hello names, with a
	// broadcast and without; none without them.
	// BROADCAST_NAMESPACES: the namespaces of broadcasts tokens are given in,
	// separated by ","; none where it is not set.
	// TRANSCRIPT_PREFIX and MEETING_TRANSCRIPT_PREFIX: the prefixes of the
	// transcripts of a room's broadcast and of its meeting, with {room} put in
	// its name (xmoq/demo, xmoq/meet/{room}); none without each.
	bindings: {
		// Not bound where it is not set: no key, and no token.
		...(process.env.AUTH_KEY_HEX === undefined
			? {}
			: { AUTH_KEY_HEX: process.env.AUTH_KEY_HEX }),
		...(process.env.BROADCAST_NAMESPACES === undefined
			? {}
			: { BROADCAST_NAMESPACES: process.env.BROADCAST_NAMESPACES }),
		REACTION_BATCH_MS: process.env.REACTION_BATCH_MS ?? "0",
		RELAY_URL: process.env.RELAY_URL ?? "",
		RELAY_CERT_SHA256_HEX: process.env.RELAY_CERT_SHA256_HEX ?? "",
		TRANSCRIPT_PREFIX: process.env.TRANSCRIPT_PREFIX ?? "",
		MEETING_TRANSCRIPT_PREFIX: process.env.MEETING_TRANSCRIPT_PREFIX ?? "",
	},
	host: "127.0.0.1",
	port,
});
const url = await miniflare.ready;
console.log(`rooms: ${url}`);
const stop = async (): Promise<void> => {
	await miniflare.dispose();
	process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
