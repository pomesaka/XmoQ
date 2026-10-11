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
