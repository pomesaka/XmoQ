// Starts the demo: the own relay, and a web server with the two pages.
//
// Usage: bun demo/serve.ts [--relay-port 4443] [--port 8080] [--rooms-port 8787] [--no-auth] [--relay-binary FILE] [--relay-log FILE] [--congestion cubic|newreno|bbr] [--delivery-patience-ms N] [--stats-every-ms N] [--chat-store FILE] [--transcribe-language auto|ja|en|...] [--lan IP]
//
// Tokens are asked for unless `--no-auth` (decided on 2026-10-26; who may
// get a token is undecided): a key is made for each run and given to the
// relay and to the rooms. The relay refuses a request without a token of the
// rooms, and the pages ask the room for one; `?token=no` on a page asks for
// none. The relay looks at the token in SETUP, and closes a session without
// one before any request. WHY by default: the relay listens on every
// interface, and without tokens anybody who reaches the port could take the
// camera and the microphone of `/publish` and publish in its place.
// `--no-auth` is for debugging and experiments: no key, and the relay takes
// every request.
//
// The rooms (where the relay and the broadcast are, tokens, and the seats
// of a meeting) are packages/room, run locally with Miniflare. They start when the workspace
// has its dependencies (`bun install`); without them the demo runs without a
// room.
//
// The gatherer of the chat (`xmoq-chat`) starts beside the relay, where it
// is built, and keeps the lines in `--chat-store` (demo/.chat/chat.sqlite3,
// which is not committed). The pages chat over its tracks, and have no chat
// without it. Unless `--no-auth`, the gatherer is given the relay's key
// (`--auth-key-hex`) and makes its own tokens, and each page asks the room
// for two whenever it connects for the chat (`room.ts` of each application). WHY the key
// for the gatherer, and not a token of the room: the room gives a token to
// whoever asks, and the gatherer needs one for every namespace under
// `xmoq/chat`. A gatherer built without that flag exits at once with
// "unknown flag", which is shown as any other exit of the gatherer.
//
// The transcriber (`xmoq-transcriber`, a release build) starts beside the
// relay where it is built and its models are in the cache (`mise run
// speech-model`, and `mise run speech-model -- silero-v5.1.2`). It listens
// under the broadcast of the room "demo", and the watching page shows its
// transcript as captions. Without it, or without the models, the demo runs
// without captions and says how to have them. Unless `--no-auth` it is given the
// relay's key, as the gatherer is. The publishing page sends a tone unless
// it is opened with `?audio=mic`; the transcriber tells that no voice is in
// the tone, and writes nothing.
//
// Open http://localhost:8080/publish in Chromium, then /watch in one or
// more other tabs; or /meet in two or more, each a participant of the
// meeting of its room, whose transcript the transcriber reads for the room
// "demo" alone. Ctrl-C stops the relay as well.
//
// `--lan IP` serves the pages over HTTPS from that address of this machine,
// for others on the same network, with a certificate of its own that no
// browser trusts: a browser past its warning ("proceed") still takes the
// page as a secure context, and has WebTransport and the camera (Chromium of
// Playwright 1.64.0, 2026-10-29). The relay already listens on every
// interface, and the pages take it by its hash at that address, though the
// certificate names only localhost and 127.0.0.1. The rooms stay on
// 127.0.0.1, and the pages reach them through this server under /rooms/.

import { execFileSync, spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { existsSync, mkdirSync, openSync, readFileSync } from "node:fs";
import { isIP } from "node:net";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
// WHY the limits alone: the package itself has the chat of a page, of the
// DOM, which this side does not run.
import { chatLimits } from "@xmoq/chat/limits";
import { BROADCAST_PAGES } from "./broadcast/pages.ts";
import type { Page, Served } from "./lib/shell.ts";
import { MEETING_PAGES } from "./meeting/pages.ts";
import { PET_PAGES } from "./pet/pages.ts";

const { values: options } = parseArgs({
	options: {
		"relay-port": { type: "string", default: "4443" },
		"rooms-port": { type: "string", default: "8787" },
		// No key, and the relay takes every request: for debugging and
		// experiments.
		"no-auth": { type: "boolean", default: false },
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
		// A file for the pet's lines (what it was told, and what it decided).
		"pet-log": { type: "string" },
		// The gatherer's store: one file for one gatherer, which holds it alone.
		"chat-store": { type: "string" },
		// The language the transcriber reads in: `auto` (its default), or a
		// code of whisper's. A code is twice as fast as `auto`, and has every
		// voice read in that language: an English voice read with `ja` came
		// out in Japanese (docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md).
		"transcribe-language": { type: "string", default: "auto" },
		// The model of whisper.cpp the transcriber reads with, and its model of
		// voice activity: the files `mise run speech-model` fetches into the
		// cache when left out.
		"speech-model": { type: "string" },
		"vad-model": { type: "string" },
		// The limits of a line of the chat: the gatherer drops a line over
		// them, and the pages keep it back and say so. 512 bytes and 5 lines
		// a second (provisional, the gatherer's defaults).
		"chat-text-bytes": { type: "string", default: "512" },
		"chat-lines-per-second": { type: "string", default: "5" },
		// How many reactions one person may say in a second: the gatherer
		// drops one over it, and the pages keep it back (provisional,
		// decided on 2026-10-26: 5).
		"chat-reactions-per-second": { type: "string", default: "5" },
		// How long the gatherer counts the reactions of a room before it
		// publishes them as one, and how many emoji one while has at most
		// (provisional, decided on 2026-10-26: 100 ms and 16). The gatherer
		// checks them, and exits on one it does not take.
		"reaction-batch-ms": { type: "string", default: "100" },
		"reaction-kinds": { type: "string", default: "16" },
		// Whose video a page of the meeting receives at once, the first that
		// many as they came (decided on 2026-10-25: 8); 0 for none.
		"meeting-most-watched": { type: "string", default: "8" },
		// The decisions API the pet asks what it does (`xmoq-pet`
		// `--decisions-url`), with the token in the environment variable
		// DECISIONS_API_TOKEN, which the pet reads from this process's
		// environment; without it, the pet reacts by its own table.
		"decisions-url": { type: "string" },
		"decisions-per-minute": { type: "string" },
		port: { type: "string", default: "8080" },
		// An address of this machine on the LAN, to show the demo to others
		// on the same network: the pages are served over HTTPS from it, and
		// given the relay and the rooms there. Without it, localhost alone.
		lan: { type: "string" },
	},
});

const here = import.meta.dir;
const lan = options.lan;
if (lan !== undefined && isIP(lan) === 0) {
	throw new RangeError(`--lan is an address of this machine, not ${lan}`);
}
/** The host the pages reach the relay and the rooms at. */
const pageHost =
	lan === undefined ? "127.0.0.1" : isIP(lan) === 6 ? `[${lan}]` : lan;
/** The relay as the pages and the rooms name it. The roles beside the relay connect over 127.0.0.1. */
const relayUrlOfThePages = `https://${pageHost}:${options["relay-port"]}/`;
// The values given to the gatherer and the pages, checked before anything is
// started: a value that is none stops the demo, as the gatherer would stop.
const chatLimitsOfTheRun = chatLimits({
	textBytes: Number(options["chat-text-bytes"]),
	linesPerSecond: Number(options["chat-lines-per-second"]),
	reactionsPerSecond: Number(options["chat-reactions-per-second"]),
});
const mostWatched = Number(options["meeting-most-watched"]);
if (!Number.isSafeInteger(mostWatched) || mostWatched < 0) {
	throw new RangeError(
		`--meeting-most-watched is a whole number from 0, not ${options["meeting-most-watched"]}`,
	);
}
/** The broadcast the transcriber reads: that of the room `/publish` and `/watch` open when no `?room=` says. */
const BROADCAST_PREFIX = "xmoq/broadcast/demo";
/** The meeting the transcriber reads: that of the room `/meet` opens when no `?room=` says. */
const MEETING_PREFIX = "xmoq/meet/demo";
const relayBinary =
	options["relay-binary"] ?? join(here, "..", "target", "debug", "xmoq-relay");
/** The key of this run, where it is one with authorization: 32 random bytes. */
const authKeyHex = !options["no-auth"]
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
	// hold its store; the transcriber would hold its models.
	chat?.kill("SIGINT");
	transcriber?.kill("SIGINT");
	garden?.kill("SIGINT");
	pet?.kill("SIGINT");
	process.exit(code ?? 1);
});
// The rooms, where their dependencies are installed.
const roomsDir = join(here, "..", "packages", "room");
const rooms = existsSync(join(roomsDir, "node_modules", "miniflare"))
	? spawn("node", ["run.ts", "--port", options["rooms-port"]], {
			cwd: roomsDir,
			stdio: ["ignore", "ignore", "inherit"],
			env: {
				...process.env,
				// Not set with --no-auth: the rooms then have no key (an empty one would stop them).
				...(authKeyHex === undefined ? {} : { AUTH_KEY_HEX: authKeyHex }),
				// The relay the rooms name.
				RELAY_URL: relayUrlOfThePages,
				RELAY_CERT_SHA256_HEX: certSha256Hex,
				// The prefixes the transcriber listens under for a room: the
				// room's broadcast and its meeting, which it reads for the
				// room "demo" alone (`BROADCAST_PREFIX`, `MEETING_PREFIX`).
				TRANSCRIPT_PREFIX: "xmoq/broadcast/{room}",
				MEETING_TRANSCRIPT_PREFIX: "xmoq/meet/{room}",
			},
		})
	: undefined;
/**
 * Where the pages reach the rooms through this server under `--lan`: the
 * same origin as the pages. WHY: a page over HTTPS may not fetch over plain
 * HTTP, and a second origin would be a second certificate to pass by.
 */
const ROOMS_PATH = "/rooms/";
const roomsUrl =
	rooms === undefined
		? null
		: lan === undefined
			? `http://127.0.0.1:${options["rooms-port"]}/`
			: `https://${pageHost}:${options.port}${ROOMS_PATH}`;
// The gatherer of the chat, where it is built, with the relay's key unless
// `--no-auth`.
const chatBinary = join(here, "..", "target", "debug", "xmoq-chat");
const chatStore = options["chat-store"] ?? join(here, ".chat", "chat.sqlite3");
const chat = existsSync(chatBinary)
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
					"--text-bytes",
					String(chatLimitsOfTheRun.textBytes),
					"--lines-per-second",
					String(chatLimitsOfTheRun.linesPerSecond),
					"--reactions-per-second",
					String(chatLimitsOfTheRun.reactionsPerSecond),
					"--reaction-batch-ms",
					options["reaction-batch-ms"],
					"--reaction-kinds",
					options["reaction-kinds"],
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
// The transcriber, where it is built and its models are in the cache.
const transcriberBinary = join(
	here,
	"..",
	"target",
	"release",
	"xmoq-transcriber",
);
const whisperCache = join(
	process.env.XDG_CACHE_HOME || join(homedir(), ".cache"),
	"xmoq",
	"whisper",
);
// The model the transcriber reads with and its model of voice activity:
// those `mise run speech-model` fetches, unless the flags name others.
const speechModel =
	options["speech-model"] ?? join(whisperCache, "ggml-small-q5_1.bin");
const vadModel =
	options["vad-model"] ?? join(whisperCache, "ggml-silero-v5.1.2.bin");
const transcriberModels = [speechModel, vadModel];
/** Why the watching page has no captions, where it has none. */
const noCaptions = !existsSync(transcriberBinary)
	? `no captions: ${transcriberBinary} is missing (cargo build --release -p xmoq-transcriber)`
	: transcriberModels.some((model) => !existsSync(model))
		? "no captions: a model of the transcriber is missing (mise run speech-model, and mise run speech-model -- silero-v5.1.2)"
		: undefined;
const transcriber =
	noCaptions === undefined
		? spawn(
				transcriberBinary,
				[
					"--url",
					`https://127.0.0.1:${options["relay-port"]}/`,
					"--cert-sha256",
					certSha256Hex,
					// The broadcast and the meeting of the room "demo" alone:
					// the transcript is one for each prefix, and a room names
					// that of its own, `xmoq/broadcast/<room>` and
					// `xmoq/meet/<room>`.
					"--namespace",
					BROADCAST_PREFIX,
					"--namespace",
					MEETING_PREFIX,
					"--language",
					options["transcribe-language"],
					"--model",
					speechModel,
					"--vad-model",
					vadModel,
					...(authKeyHex === undefined ? [] : ["--auth-key-hex", authKeyHex]),
				],
				// Its lines, one for each thing it does, are not shown.
				{ stdio: ["ignore", "ignore", "inherit"] },
			)
		: undefined;
transcriber?.on("exit", (code) => {
	// The watching page has no captions without it; the run goes on.
	console.log(`the transcriber exited with ${code}`);
});
// The garden of the rooms (`xmoq-garden`), where it is built, with the
// relay's key unless `--no-auth`, as the gatherer: it keeps what lies on
// each room's floor, which the pages and the pet act on.
const gardenBinary = join(here, "..", "target", "debug", "xmoq-garden");
const garden = existsSync(gardenBinary)
	? spawn(
			gardenBinary,
			[
				"--url",
				`https://127.0.0.1:${options["relay-port"]}/`,
				"--cert-sha256",
				certSha256Hex,
				...(authKeyHex === undefined ? [] : ["--auth-key-hex", authKeyHex]),
			],
			// Its lines, one for each thing it does, are not shown.
			{ stdio: ["ignore", "ignore", "inherit"] },
		)
	: undefined;
garden?.on("exit", (code) => {
	// The page of the pet has an empty floor without it; the run goes on.
	console.log(`the garden exited with ${code}`);
});
// The pet of the rooms (`xmoq-pet`), where it is built, with the relay's
// key unless `--no-auth`, as the gatherer.
const petBinary = join(here, "..", "target", "debug", "xmoq-pet");
const pet = existsSync(petBinary)
	? spawn(
			petBinary,
			[
				"--url",
				`https://127.0.0.1:${options["relay-port"]}/`,
				"--cert-sha256",
				certSha256Hex,
				...(authKeyHex === undefined ? [] : ["--auth-key-hex", authKeyHex]),
				...(options["decisions-url"] === undefined
					? []
					: ["--decisions-url", options["decisions-url"]]),
				...(options["decisions-per-minute"] === undefined
					? []
					: ["--decisions-per-minute", options["decisions-per-minute"]]),
			],
			// Its lines, one for each thing it does, are not shown, unless
			// `--pet-log` keeps them.
			{
				stdio: [
					"ignore",
					options["pet-log"] === undefined
						? "ignore"
						: openSync(options["pet-log"], "w"),
					"inherit",
				],
			},
		)
	: undefined;
pet?.on("exit", (code) => {
	// The page of the pet shows an empty garden without it; the run goes on.
	console.log(`the pet exited with ${code}`);
});
/** Why the pages have no chat, where they have none. */
const noChat = `no chat: ${chatBinary} is missing (cargo build -p xmoq-chat)`;
const stop = (): void => {
	relay.kill("SIGINT");
	rooms?.kill("SIGINT");
	chat?.kill("SIGINT");
	transcriber?.kill("SIGINT");
	garden?.kill("SIGINT");
	pet?.kill("SIGINT");
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

// Which path is of which application: each assembles its own pages
// (`pages.ts`), and this server only serves them, so that one could be run
// apart from the other.
const PAGES: Record<string, Page> = {
	...BROADCAST_PAGES,
	...MEETING_PAGES,
	...PET_PAGES,
};
const served: Served = {
	roomsUrl,
	noChat: chat === undefined ? noChat : undefined,
	noCaptions,
};
const scripts = new Map<string, string>();
for (const [path, page] of Object.entries(PAGES)) {
	const build = await Bun.build({
		entrypoints: [page.entry],
		target: "browser",
		throw: false,
		define: {
			XMOQ_DEMO: JSON.stringify({
				roomsUrl,
				chatLimits: chatLimitsOfTheRun,
				mostWatched,
				chatTracks: chat !== undefined,
				transcript: transcriber !== undefined,
				relayUrl: relayUrlOfThePages,
				certSha256Hex,
			}),
		},
	});
	const [bundle] = build.outputs;
	if (!build.success || bundle === undefined) {
		stop();
		throw new Error(`bundling ${page.entry} failed: ${build.logs.join("\n")}`);
	}
	scripts.set(`${path}.js`, await bundle.text());
}

/**
 * The certificate of the pages under `--lan`, one for each address, kept
 * for a year: nobody trusts it, and a browser asks once to go on past it.
 * WHY not the relay's: that one is made anew within 13 days, and each new
 * one would be asked about again. WHY RSA: Bun's TLS did not take an EC key
 * of openssl (Bun 1.3.14).
 */
const pageTls =
	lan === undefined
		? undefined
		: (() => {
				const pageCertFile = join(certDir, `page-${lan}.pem`);
				const pageKeyFile = join(certDir, `page-${lan}-key.pem`);
				if (!existsSync(pageCertFile) || !existsSync(pageKeyFile)) {
					execFileSync("openssl", [
						"req",
						"-x509",
						"-newkey",
						"rsa:2048",
						"-nodes",
						"-keyout",
						pageKeyFile,
						"-out",
						pageCertFile,
						"-days",
						"365",
						"-subj",
						`/CN=${lan}`,
						"-addext",
						`subjectAltName=IP:${lan}`,
					]);
				}
				return {
					cert: readFileSync(pageCertFile),
					key: readFileSync(pageKeyFile),
				};
			})();

/** A request of a page to the rooms under `--lan`, passed on to them. */
async function passToTheRooms(request: Request, url: URL): Promise<Response> {
	const answer = await fetch(
		`http://127.0.0.1:${options["rooms-port"]}/${url.pathname.slice(ROOMS_PATH.length)}${url.search}`,
		{ method: request.method, headers: request.headers, body: request.body },
	);
	// WHY: Bun's fetch gives the body uncompressed and keeps the header that
	// says it is compressed, which a browser then fails to read (Bun 1.3.14).
	const headers = new Headers(answer.headers);
	headers.delete("content-encoding");
	headers.delete("content-length");
	return new Response(answer.body, { status: answer.status, headers });
}

const server = Bun.serve({
	hostname: lan ?? "127.0.0.1",
	port: Number(options.port),
	tls: pageTls,
	fetch(request) {
		const url = new URL(request.url);
		const { pathname } = url;
		if (lan !== undefined && pathname.startsWith(ROOMS_PATH)) {
			return passToTheRooms(request, url);
		}
		const script = scripts.get(pathname);
		if (script !== undefined) {
			return new Response(script, {
				headers: { "content-type": "text/javascript" },
			});
		}
		const page = PAGES[pathname];
		if (page !== undefined) {
			return new Response(page.html(served), {
				headers: { "content-type": "text/html" },
			});
		}
		return new Response(null, { status: 404 });
	},
});
// WHY localhost, or HTTPS under `--lan`: a secure context, which
// WebTransport and the camera need.
const origin =
	lan === undefined
		? `http://localhost:${server.port}`
		: `https://${pageHost}:${server.port}`;
console.log(`publish: ${origin}/publish`);
console.log(`watch:   ${origin}/watch`);
console.log(`meet:    ${origin}/meet`);
console.log(
	pet === undefined || garden === undefined
		? `pet:     none: ${pet === undefined ? petBinary : gardenBinary} is missing (cargo build -p xmoq-pet -p xmoq-garden)`
		: `pet:     ${origin}/pet${options["decisions-url"] === undefined ? " (reacting by its own table)" : ""}`,
);
console.log(roomsUrl === null ? "rooms:   none" : `rooms:   ${roomsUrl}`);
console.log(
	chat !== undefined
		? `chat:    over the tracks; the gatherer keeps the lines in ${chatStore}`
		: `chat:    ${noChat}`,
);
console.log(
	noCaptions ??
		`captions: the transcriber reads ${BROADCAST_PREFIX} and ${MEETING_PREFIX} in ${options["transcribe-language"]}; open /publish?audio=mic to be heard`,
);
if (authKeyHex !== undefined) {
	console.log(
		rooms === undefined
			? "auth:    the relay asks for tokens, and no rooms run to give them"
			: "auth:    the relay asks for tokens, and the rooms give them",
	);
} else {
	console.log("auth:    off (--no-auth): the relay takes every request");
}
