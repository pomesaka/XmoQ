// Runs the room Worker locally with Miniflare. Nothing is deployed and no
// Cloudflare account is touched.
//
// Usage: node run.mjs [--port 8787]
import { Miniflare } from "miniflare";

const at = process.argv.indexOf("--port");
const port = at === -1 ? 8787 : Number(process.argv[at + 1]);
const miniflare = new Miniflare({
	modules: true,
	scriptPath: new URL("./worker.js", import.meta.url).pathname,
	compatibilityDate: "2026-07-01",
	durableObjects: { ROOMS: { className: "Room", useSQLite: true } },
	// The key the rooms share with the relay, for the tokens; none without.
	// REACTION_BATCH_MS: a trial, the reactions of that long as one message.
	// RELAY_URL and RELAY_CERT_SHA256_HEX: the relay the hello names, with a
	// broadcast and without; none without them.
	bindings: {
		AUTH_KEY_HEX: process.env.AUTH_KEY_HEX ?? "",
		REACTION_BATCH_MS: process.env.REACTION_BATCH_MS ?? "0",
		RELAY_URL: process.env.RELAY_URL ?? "",
		RELAY_CERT_SHA256_HEX: process.env.RELAY_CERT_SHA256_HEX ?? "",
	},
	host: "127.0.0.1",
	port,
});
const url = await miniflare.ready;
console.log(`rooms: ${url}`);
const stop = async () => {
	await miniflare.dispose();
	process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
