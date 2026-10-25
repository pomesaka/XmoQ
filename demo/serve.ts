// Starts the demo: the own relay, and a web server with the two pages.
//
// Usage: bun demo/serve.ts [--relay-port 4443] [--port 8080] [--rooms-port 8787] [--auth] [--relay-log FILE] [--congestion cubic|newreno|bbr] [--delivery-patience-ms N] [--stats-every-ms N] [--chat-store FILE]
//
// `--auth` (an experiment; how the relay authorizes is undecided): a key
// is made for this run and given to the relay and to the rooms. The relay
// then refuses a request without a token of the rooms, and the pages ask
// the room for one; `?token=no` on a page asks for none. The relay looks
// at the token in SETUP, and closes a session without one before any
// request.
//
// The rooms (a count, where the broadcast is, and reactions) are the spike in
// spikes/durable-objects-room, run locally with Miniflare. They start when
// that directory has its dependencies (`npm install` there); without them
// the demo runs without a room.
//
// The gatherer of the chat (`xmoq-chat`) starts beside the relay, where it
// is built, and keeps the lines in `--chat-store` (demo/.chat/chat.sqlite3,
// which is not committed). The pages chat over its tracks, and have no chat
// without it. With `--auth`, the gatherer is given the relay's key
// (`--auth-key-hex`) and makes its own tokens, and each page asks the room
// for two whenever it connects for the chat (`demo/shared.ts`). WHY the key
// for the gatherer, and not a token of the room: the room gives a token to
// whoever asks, and the gatherer needs one for every namespace under
// `xmoq/chat`. A gatherer built without that flag exits at once with
// "unknown flag", which is shown as any other exit of the gatherer.
//
// Open http://localhost:8080/publish in Chromium, then /watch in one or
// more other tabs. Ctrl-C stops the relay as well.

import { execFileSync, spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { existsSync, mkdirSync, openSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";

const { values: options } = parseArgs({
	options: {
		"relay-port": { type: "string", default: "4443" },
		"rooms-port": { type: "string", default: "8787" },
		auth: { type: "boolean", default: false },
		// Another relay than the one of this tree, for trying one out.
		"relay-binary": { type: "string" },
		// The relay's congestion control: cubic (its default), newreno or bbr.
		congestion: { type: "string" },
		// An experiment of the relay: a Group a viewer does not have this many
		// milliseconds after it was written is given up (its stream is reset).
		"delivery-patience-ms": { type: "string" },
		// For measuring: the relay writes a STATS line for each session this
		// often (into `--relay-log`).
		"stats-every-ms": { type: "string" },
		// A file for the relay's lines, for looking into a run.
		"relay-log": { type: "string" },
		// The gatherer's store: one file for one gatherer, which holds it alone.
		"chat-store": { type: "string" },
		port: { type: "string", default: "8080" },
	},
});

const here = import.meta.dir;
const relayBinary =
	options["relay-binary"] ?? join(here, "..", "target", "debug", "xmoq-relay");
/** The key of this run, where it is one with authorization: 32 random bytes. */
const authKeyHex = options.auth
	? Array.from(crypto.getRandomValues(new Uint8Array(32)), (byte) =>
			byte.toString(16).padStart(2, "0"),
		).join("")
	: undefined;
if (!existsSync(relayBinary)) {
	throw new Error(`${relayBinary} is missing: cargo build -p xmoq-relay`);
}

// A certificate of its own, made anew when it has less than a day left.
// WHY 13 days: Chromium takes a certificate by its hash
// (`serverCertificateHashes`) only when it is valid for less than two weeks.
const certDir = join(here, ".cert");
const certFile = join(certDir, "cert.pem");
const keyFile = join(certDir, "key.pem");
const usable =
	existsSync(certFile) &&
	existsSync(keyFile) &&
	new Date(new X509Certificate(readFileSync(certFile)).validTo).getTime() -
		Date.now() >
		24 * 60 * 60 * 1000;
if (!usable) {
	mkdirSync(certDir, { recursive: true });
	// The two steps interop/harness.ts takes, for the reason it gives.
	execFileSync("openssl", [
		"ecparam",
		"-name",
		"prime256v1",
		"-genkey",
		"-noout",
		"-out",
		keyFile,
	]);
	execFileSync("openssl", [
		"req",
		"-x509",
		"-new",
		"-key",
		keyFile,
		"-out",
		certFile,
		"-days",
		"13",
		"-subj",
		"/CN=localhost",
		"-addext",
		"subjectAltName=DNS:localhost,IP:127.0.0.1",
	]);
}
const certSha256Hex = createHash("sha256")
	.update(new X509Certificate(readFileSync(certFile)).raw)
	.digest("hex");

const relay = spawn(
	relayBinary,
	[
		"--port",
		options["relay-port"],
		"--cert-file",
		certFile,
		"--key-file",
		keyFile,
		...(authKeyHex === undefined ? [] : ["--auth-key-hex", authKeyHex]),
		...(options["relay-log"] === undefined ? [] : ["--slow-open-ms", "0"]),
		...(options.congestion === undefined
			? []
			: ["--congestion", options.congestion]),
		...(options["delivery-patience-ms"] === undefined
			? []
			: ["--delivery-patience-ms", options["delivery-patience-ms"]]),
		...(options["stats-every-ms"] === undefined
			? []
			: ["--stats-every-ms", options["stats-every-ms"]]),
	],
	// Its lines, one for each thing it does, are not shown; `--relay-log`
	// writes them to a file, with how long every stream took to open.
	{
		stdio: [
			"ignore",
			options["relay-log"] === undefined
				? "ignore"
				: openSync(options["relay-log"], "w"),
			"inherit",
		],
	},
);
relay.on("exit", (code) => {
	console.log(`the relay exited with ${code}`);
	// The gatherer would otherwise stay, connecting again to nobody, and
	// hold its store.
	chat?.kill("SIGINT");
	process.exit(code ?? 1);
});
// The rooms, where the spike is installed.
const roomsDir = join(here, "..", "spikes", "durable-objects-room");
const rooms = existsSync(join(roomsDir, "node_modules", "miniflare"))
	? spawn("node", ["run.mjs", "--port", options["rooms-port"]], {
			cwd: roomsDir,
			stdio: ["ignore", "ignore", "inherit"],
			env: {
				...process.env,
				AUTH_KEY_HEX: authKeyHex ?? "",
				// The relay the rooms name in their hello, with a broadcast
				// and without one.
				RELAY_URL: `https://127.0.0.1:${options["relay-port"]}/`,
				RELAY_CERT_SHA256_HEX: certSha256Hex,
			},
		})
	: undefined;
const roomsUrl =
	rooms === undefined ? null : `http://127.0.0.1:${options["rooms-port"]}/`;
// The gatherer of the chat, where it is built, with the relay's key under
// `--auth`.
const chatBinary = join(here, "..", "target", "debug", "xmoq-chat");
const chatStore = options["chat-store"] ?? join(here, ".chat", "chat.sqlite3");
const chat =
	existsSync(chatBinary)
		? (() => {
				mkdirSync(dirname(chatStore), { recursive: true });
				return spawn(
					chatBinary,
					[
						"--url",
						`https://127.0.0.1:${options["relay-port"]}/`,
						"--cert-sha256",
						certSha256Hex,
						"--store",
						chatStore,
						...(authKeyHex === undefined ? [] : ["--auth-key-hex", authKeyHex]),
					],
					// Its lines, one for each thing it does, are not shown.
					{ stdio: ["ignore", "ignore", "inherit"] },
				);
			})()
		: undefined;
chat?.on("exit", (code) => {
	// The pages have no chat without it; the run goes on.
	console.log(`the gatherer of the chat exited with ${code}`);
});
/** Why the pages have no chat, where they have none. */
const noChat = `no chat: ${chatBinary} is missing (cargo build -p xmoq-chat)`;
const stop = (): void => {
	relay.kill("SIGINT");
	rooms?.kill("SIGINT");
	chat?.kill("SIGINT");
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

const PAGES: Record<string, string> = {
	"/publish": "publish.ts",
	"/watch": "watch.ts",
};
const scripts = new Map<string, string>();
for (const [path, entry] of Object.entries(PAGES)) {
	const build = await Bun.build({
		entrypoints: [join(here, entry)],
		target: "browser",
		throw: false,
		define: {
			XMOQ_DEMO: JSON.stringify({
				roomsUrl,
				chatTracks: chat !== undefined,
				relayUrl: `https://127.0.0.1:${options["relay-port"]}/`,
				certSha256Hex,
			}),
		},
	});
	const [bundle] = build.outputs;
	if (!build.success || bundle === undefined) {
		stop();
		throw new Error(`bundling ${entry} failed: ${build.logs.join("\n")}`);
	}
	scripts.set(`${path}.js`, await bundle.text());
}

const html = (path: string): string => `<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<title>XmoQ demo: ${path.slice(1)}</title>
		<link rel="icon" href="data:," />
		<style>
			body { font-family: monospace; background: #111; color: #ddd; margin: 16px; }
			canvas { display: block; max-width: 100%; background: #000; }
			a { color: #f2c14e; }
		</style>
	</head>
	<body>
		<p>XmoQ demo, ${path.slice(1)} · <a href="/publish">publish (a clock)</a> · <a href="/publish?source=camera">publish (camera)</a> · <a href="/watch">watch</a></p>
		<canvas width="640" height="360"></canvas>
		${path === "/watch" ? '<p><button id="sound">sound</button></p>' : ""}
		<pre id="status">loading</pre>
		${
			roomsUrl === null
				? "<p>no rooms: npm install in spikes/durable-objects-room, and start again</p>"
				: `<p id="connected"></p>
		${
			chat === undefined
				? `<p>${noChat}</p>`
				: `<div id="chat" style="height: 8em; overflow-y: auto; border: 1px solid #444; padding: 4px;"></div>
		<form><input size="40" placeholder="say something" /> <button>send</button></form>
		<p id="chat-note"></p>`
		}
		<p>${["👍", "❤️", "😂", "👏"].map((emoji) => `<button data-reaction="${emoji}">${emoji}</button>`).join(" ")} <span id="reactions"></span></p>`
		}
		<script type="module" src="${path}.js"></script>
	</body>
</html>
`;

const server = Bun.serve({
	hostname: "127.0.0.1",
	port: Number(options.port),
	fetch(request) {
		const { pathname } = new URL(request.url);
		const script = scripts.get(pathname);
		if (script !== undefined) {
			return new Response(script, {
				headers: { "content-type": "text/javascript" },
			});
		}
		if (pathname in PAGES) {
			return new Response(html(pathname), {
				headers: { "content-type": "text/html" },
			});
		}
		return new Response(null, { status: 404 });
	},
});
// WHY localhost: it is a secure context, which WebTransport and the camera need.
console.log(`publish: http://localhost:${server.port}/publish`);
console.log(`watch:   http://localhost:${server.port}/watch`);
console.log(roomsUrl === null ? "rooms:   none" : `rooms:   ${roomsUrl}`);
console.log(
	chat !== undefined
		? `chat:    over the tracks; the gatherer keeps the lines in ${chatStore}`
		: `chat:    ${noChat}`,
);
if (authKeyHex !== undefined) {
	console.log(
		rooms === undefined
			? "auth:    the relay asks for tokens, and no rooms run to give them"
			: "auth:    the relay asks for tokens, and the rooms give them",
	);
}
