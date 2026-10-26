// What the two pages of the demo share. Which tracks a broadcast has, and
// their codecs, the watching page reads from the MSF catalog the publishing
// page puts out; the namespace, the sample rate of the sound and the mark
// are agreed on here.

import {
	connect,
	DEFAULT_RECONNECT_DELAY_MS,
	type MoqtConnection,
} from "@xmoq/moqt";
import { NAMESPACE } from "./namespace.ts";
import { openRoomSocket, tokenOf } from "./room-socket.ts";
import { startChat } from "./room-chat.ts";

export { NAMESPACE };
/** The pause between sessions, as a page says it: what `keepPublishing` and `keepPlaying` wait where they are given no `againMs`. */
export const AGAIN = `${DEFAULT_RECONNECT_DELAY_MS / 1000} s`;
export const TRACK = "video";
export const AUDIO_TRACK = "audio";
export const SAMPLE_RATE = 48_000;

/**
 * The mark both tracks carry at the same moment of the publisher's clock,
 * for a viewer to measure by: for the first 200 ms of every 2 seconds the
 * picture has a white square in its upper right corner, and the sound is a
 * loud beep.
 */
export const marked = (wallClockMs: number): boolean =>
	wallClockMs % 2000 < 200;
/** Where the square is. */
export const MARK = { x: 540, y: 20, size: 80 };
/** The size the picture is painted in, whatever size it is sent at: the mark and the clock are at the same place of every size. */
export const WIDTH = 640;
export const HEIGHT = 360;
/** What the encoder is configured with, and what the decoder is told: the same profile and level, with the parameter sets in the key frames (`avc3`). */
export const ENCODER_CODEC = "avc1.42E01F";
export const DECODER_CODEC = "avc3.42E01F";

/**
 * The sizes the publishing page sends at, `/publish?size=360|720|1080`
 * (360 when left out), 30 frames a second, 16:9.
 *
 * - **The level** is the lowest of H.264 that has the size at 30 frames a
 *   second (Table A-1 of H.264, read from `libavcodec/h264_levels.c` of
 *   FFmpeg on 2026-10-12): Level 3.1 (`1F`) has frames of up to 3,600
 *   macroblocks and 108,000 a second, which 1280x720 at 30 is exactly
 *   (80x45 = 3,600); 1920x1080 is 8,160 macroblocks (120x68) and 244,800 a
 *   second, which Level 4 (`28`: 8,192 and 245,760) has and 3.1 and 3.2
 *   (5,120) do not. The profile stays Constrained Baseline (`42E0`).
 * - **The bitrate** where `?bitrate=` gives none: **provisional**, twice
 *   and four times that of 360, not from a measurement of what looks well.
 */
export const SIZES = {
	"360": { width: 640, height: 360, level: "1F", bitrate: 1_500_000 },
	"720": { width: 1280, height: 720, level: "1F", bitrate: 3_000_000 },
	"1080": { width: 1920, height: 1080, level: "28", bitrate: 6_000_000 },
} as const;

const utf8 = new TextEncoder();
export const namespace = (fields: string[] = NAMESPACE): Uint8Array[] =>
	fields.map((field) => utf8.encode(field));
export const trackName = (): Uint8Array => utf8.encode(TRACK);
export const audioTrackName = (): Uint8Array => utf8.encode(AUDIO_TRACK);

/** Where a broadcast is: what a viewer needs to connect to it. */
export interface Broadcast {
	relayUrl: string;
	certSha256Hex: string;
	namespace: string[];
}

/** The relay, and the rooms where they run, as `serve.ts` wrote them into the page. */
declare const XMOQ_DEMO: {
	relayUrl: string;
	certSha256Hex: string;
	roomsUrl: string | null;
};
/** The broadcast of the demo where no room says otherwise. */
export const HERE: Broadcast = {
	relayUrl: XMOQ_DEMO.relayUrl,
	certSha256Hex: XMOQ_DEMO.certSha256Hex,
	namespace: NAMESPACE,
};

/** What a room says on its WebSocket (`packages/room/src/worker.ts`). */
type RoomMessage =
	| {
			type: "hello";
			connected: number;
			broadcast: Broadcast | null;
			transcript: string[] | null;
		}
	| { type: "connected"; connected: number }
	| { type: "reaction"; emoji: string }
	| { type: "reactions"; counts: Record<string, number> }
	| { type: "token"; role: string; namespace: string[]; token: string | null }
	| { type: "broadcast"; broadcast: Broadcast | null };
export interface Room {
	/** Resolves with the broadcast the room has when this page comes, or null. */
	readonly broadcast: Promise<Broadcast | null>;
	/** The broadcast the room has now, as it last said; null without one, or before it said anything. */
	current(): Broadcast | null;
	/**
	 * The prefix the room says a transcriber listens under for its broadcast
	 * (`transcript` of its hello), the fields of `xmoq/transcript/<prefix>`
	 * the room gives a token in; null without one, or before the hello.
	 * WHY from the room: it gives tokens in its own prefix alone
	 * (`tokenForNamespace` of `packages/room/src/core/rules.ts`).
	 */
	transcript(): string[] | null;
	/**
	 * Resolves with the broadcast of the room: at once where it has one, and
	 * else when a publisher registers one (the room says so to everybody in
	 * it as it happens). WHY: a viewer that asked `current` every 2 seconds
	 * had its picture up to 2 seconds after the publisher had begun.
	 */
	named(): Promise<Broadcast>;
	/** Says where this page publishes; holds as long as this page is connected to the room. */
	publish(broadcast: Broadcast): void;
	/**
	 * Asks the room for a token for the relay (an experiment in
	 * authorization): the Token structure as `@xmoq/moqt` carries it, or
	 * undefined where the rooms give none, or where the room's socket is
	 * closed (it is opened again). `?token=no` asks for none, to see the
	 * relay refuse.
	 */
	token(role: "pub" | "sub", namespace: string[]): Promise<Uint8Array | undefined>;
}

/**
 * Joins the room `?room=NAME` ("demo" when left out) of the rooms `serve.ts`
 * started, shows how many are connected, and starts the chat of that room
 * (over MoQ tracks, not this WebSocket). Undefined where
 * no rooms run: the demo then works without one.
 */
export function joinRoom(as: string): Room | undefined {
	if (XMOQ_DEMO.roomsUrl === null) return undefined;
	const name = new URLSearchParams(location.search).get("room") ?? "demo";
	const count = document.getElementById("connected");
	// Reactions: an emoji somebody pressed is shown to everybody for two
	// seconds, and counted for a script that looks at the page.
	const reactions = document.getElementById("reactions");
	let reacted = 0;
	const react = (emoji: string, count: number): void => {
		reacted += count;
		document.body.dataset.reactions = String(reacted);
		if (reactions === null) return;
		const shown = document.createElement("span");
		shown.textContent = count === 1 ? emoji : `${emoji}×${count}`;
		reactions.append(shown);
		// The newest 50: how many come in two seconds is the room's to decide.
		while (reactions.childElementCount > 50) reactions.firstElementChild?.remove();
		setTimeout(() => shown.remove(), 2000);
	};
	let latest: Broadcast | null = null;
	let transcript: string[] | null = null;
	/** Where this page publishes, said again each time the socket opens: the room forgets it with the socket. */
	let published: Broadcast | undefined;
	/** The callers of `named` that wait for a publisher to register. */
	const waiting: ((broadcast: Broadcast) => void)[] = [];
	let first: (broadcast: Broadcast | null) => void = () => {};
	const broadcast = new Promise<Broadcast | null>((resolve) => {
		first = resolve;
	});
	const socket = openRoomSocket<RoomMessage>(
		`${XMOQ_DEMO.roomsUrl.replace("http", "ws")}rooms/${encodeURIComponent(name)}/ws`,
		{
			open() {
				if (published !== undefined) {
					socket.send({ type: "broadcast", broadcast: published });
				}
			},
			message(message) {
				if (message.type === "hello") {
					transcript = message.transcript;
					first(message.broadcast);
				}
				if (message.type === "hello" || message.type === "broadcast") {
					latest = message.broadcast;
					if (latest !== null) {
						for (const tell of waiting.splice(0)) tell(latest);
					}
				}
				if (message.type === "reaction") react(message.emoji, 1);
				if (message.type === "reactions") {
					for (const [emoji, count] of Object.entries(message.counts)) {
						react(emoji, count);
					}
				}
				if ((message.type === "hello" || message.type === "connected") && count !== null) {
					count.textContent = `room “${name}”: ${message.connected} connected`;
					document.body.dataset.connected = String(message.connected);
				}
			},
			close() {
				first(null);
				if (count !== null) count.textContent = `room “${name}”: joining again`;
				delete document.body.dataset.connected;
			},
		},
	);
	for (const button of document.querySelectorAll<HTMLButtonElement>(
		"button[data-reaction]",
	)) {
		button.addEventListener("click", () => {
			socket.send({ type: "reaction", emoji: button.dataset.reaction });
		});
	}
	const token: Room["token"] = async (role, namespace) => {
		if (new URLSearchParams(location.search).get("token") === "no") {
			return undefined;
		}
		// The room says nothing for a namespace with a `*` or a `/` in a
		// field (`tokenForNamespace` of `packages/room/src/core/rules.ts`), and the answers
		// are matched to the askers by their order: one asked for and never
		// answered would have every later asker take the answer of the one
		// before it.
		if (namespace.some((field) => field.includes("*") || field.includes("/"))) {
			return undefined;
		}
		const answer = await socket.ask({ type: "token-for", role, namespace }, ["token"]);
		return answer?.type === "token" ? tokenOf(answer.token) : undefined;
	};
	// Two tokens each time it connects for the chat, as the participant
	// needs them (`publishNamespaceToken` of `joinChat`): `sub` for the
	// room's `lines`, which the connection carries, and `pub` for its own
	// namespace, which only its PUBLISH_NAMESPACE carries. WHY asked again
	// each time: a token runs out (10 minutes), and the relay checks the
	// SETUP of the new connection. Without --auth the room gives none, and
	// both are undefined. After a GOAWAY the next session goes to its URI
	// with the same certificate's hash, as the broadcast's does (the relay
	// that sends it away, `--when-full`, names one with the same certificate).
	startChat(name, as, async (goAway) => {
		const [sub, pub] = await Promise.all([
			token("sub", ["xmoq", "chat", name]),
			token("pub", ["xmoq", "chat", name, as]),
		]);
		const to =
			goAway === undefined || goAway.newSessionUri === ""
				? HERE
				: { ...HERE, relayUrl: goAway.newSessionUri };
		return {
			connection: await connectToRelay(to, sub),
			...(pub === undefined ? {} : { publishNamespaceToken: pub }),
		};
	});
	return {
		broadcast,
		current: () => latest,
		transcript: () => transcript,
		named: () =>
			latest === null
				? new Promise((resolve) => {
						waiting.push(resolve);
					})
				: Promise.resolve(latest),
		publish(where) {
			published = where;
			socket.send({ type: "broadcast", broadcast: where });
		},
		token,
	};
}

/**
 * `?via=PORT`: the relay through another port of this machine, where
 * something stands in between (`spikes/lossy-link`).
 */
export function through(broadcast: Broadcast): Broadcast {
	const via = new URLSearchParams(location.search).get("via");
	if (via === null) return broadcast;
	const url = new URL(broadcast.relayUrl);
	url.port = via;
	return { ...broadcast, relayUrl: url.href };
}

/** With `?diag=1`: when the first bytes of each stream of the peer came, by this page's clock, and how many they were. */
export const firstBytes = new Map<string, { at: number; length: number }>();

/**
 * Connects to the relay of a broadcast. With a token, every request of the
 * connection carries it (`authorizationToken` of `connect`).
 */
export function connectToRelay(
	to: Broadcast = HERE,
	token?: Uint8Array,
): Promise<MoqtConnection> {
	const hash = Uint8Array.from(to.certSha256Hex.match(/../g) ?? [], (byte) =>
		Number.parseInt(byte, 16),
	);
	return connect(to.relayUrl, {
		implementation: "xmoq-demo/0.0.0",
		...(token === undefined ? {} : { authorizationToken: token }),
		// WHY: the relay's certificate is self-signed. Chromium takes it when
		// its hash is given, and it is valid for less than two weeks.
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
		// What the README of @xmoq/media asks of a viewer.
		limits: { maxObjectPayloadLength: 1024 * 1024 },
		// `?uni=N`: how many streams of the peer are read at once, for
		// looking into where something waits; the library's when left out.
		...(new URLSearchParams(location.search).get("uni") === null
			? {}
			: {
					maxPeerUniStreams: Number(
						new URLSearchParams(location.search).get("uni"),
					),
				}),
		// `?diag=1`: when the first bytes of every stream of the peer came,
		// for looking into where something waits.
		...(new URLSearchParams(location.search).get("diag") === null
			? {}
			: {
					diagnostics: {
						onBytes(direction: string, stream: string, data: Uint8Array) {
							if (direction !== "received" || firstBytes.has(stream)) return;
							firstBytes.set(stream, { at: Date.now(), length: data.length });
						},
					},
				}),
	});
}

/** What went wrong, for the line under the picture: a refusal for want of a token is said as that. */
export const why = (error: unknown): string =>
	/REQUEST_ERROR 0x1:/.test(String(error))
		? "the relay refused: not authorized (no token of the room, or one it does not take)"
		: /closed the session: code=(2|22|24) /.test(String(error))
			? `the relay closed the session: not authorized (${String(error).slice(0, 90)})`
			: String(error).slice(0, 120);

/**
 * Calls `tick` every `ms` milliseconds, also while the page is a tab in the
 * background.
 *
 * WHY not `setInterval` of the page: a browser slows the timers of a page
 * nobody looks at (Chromium: to one a second, and less after a while), and
 * stops `requestAnimationFrame` there. A publisher in a background tab
 * then sends a frame a second. The timers of a worker are not slowed with
 * the page, and a message from a worker is handled at once: so the clock
 * runs in a worker, and the work stays here. `?timer=page` is the page's
 * own timer, and `?timer=slow` that timer as a background tab has it, to
 * see the difference.
 */
export function every(ms: number, tick: () => void): void {
	const timer = new URLSearchParams(location.search).get("timer");
	if (timer === "page") {
		setInterval(tick, ms);
		return;
	}
	if (timer === "slow") {
		// What a browser makes of a page's timer in a background tab: once
		// a second at most. For seeing what that does, without a tab.
		setInterval(tick, Math.max(ms, 1000));
		return;
	}
	const clock = new Worker(
		URL.createObjectURL(
			new Blob([`setInterval(() => postMessage(0), ${ms});`], {
				type: "text/javascript",
			}),
		),
	);
	clock.addEventListener("message", tick);
}

/** How many times something happened in the last second: `count()` notes one, `rate()` says how many. */
export function perSecond(): { count(): void; rate(): number } {
	let times: number[] = [];
	return {
		count() {
			times.push(performance.now());
		},
		rate() {
			const since = performance.now() - 1000;
			times = times.filter((time) => time > since);
			return times.length;
		},
	};
}

export const say = (text: string): void => {
	const status = document.getElementById("status");
	if (status !== null) status.textContent = text;
};
