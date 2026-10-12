// What the two pages of the demo share. Which tracks a broadcast has, and
// their codecs, the watching page reads from the MSF catalog the publishing
// page puts out; the namespace, the sample rate of the sound and the mark
// are agreed on here.

import { connect, type MoqtConnection } from "@xmoq/moqt";

export const NAMESPACE = ["xmoq", "demo"];
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
export const WIDTH = 640;
export const HEIGHT = 360;
/** What the encoder is configured with, and what the decoder is told: the same profile and level, with the parameter sets in the key frames (`avc3`). */
export const ENCODER_CODEC = "avc1.42E01F";
export const DECODER_CODEC = "avc3.42E01F";

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

/** What a room says on its WebSocket (`spikes/durable-objects-room/worker.js`). */
type RoomMessage =
	| { type: "hello"; history: Chat[]; connected: number; broadcast: Broadcast | null }
	| ({ type: "chat" } & Chat)
	| { type: "connected"; connected: number }
	| { type: "reaction"; emoji: string }
	| { type: "reactions"; counts: Record<string, number> }
	| { type: "token"; role: string; namespace: string[]; token: string | null }
	| { type: "broadcast"; broadcast: Broadcast | null };
interface Chat {
	seq: number;
	from: string;
	text: string;
}

export interface Room {
	/** Resolves with the broadcast the room has when this page comes, or null. */
	readonly broadcast: Promise<Broadcast | null>;
	/** The broadcast the room has now, as it last said; null without one, or before it said anything. */
	current(): Broadcast | null;
	/** Says where this page publishes; holds as long as this page is connected to the room. */
	publish(broadcast: Broadcast): void;
	/**
	 * Asks the room for a token for the relay (an experiment in
	 * authorization): the Token structure as `@xmoq/moqt` carries it, or
	 * undefined where the rooms give none. `?token=no` asks for none, to see
	 * the relay refuse.
	 */
	token(role: "pub" | "sub", namespace: string[]): Promise<Uint8Array | undefined>;
}

/**
 * Joins the room `?room=NAME` ("demo" when left out) of the rooms `serve.ts`
 * started, and shows its chat and how many are connected. Undefined where
 * no rooms run: the demo then works without one.
 */
export function joinRoom(as: string): Room | undefined {
	if (XMOQ_DEMO.roomsUrl === null) return undefined;
	const name = new URLSearchParams(location.search).get("room") ?? "demo";
	const socket = new WebSocket(
		`${XMOQ_DEMO.roomsUrl.replace("http", "ws")}rooms/${encodeURIComponent(name)}/ws?name=${encodeURIComponent(as)}`,
	);
	const log = document.getElementById("chat");
	const count = document.getElementById("connected");
	const show = (chat: Chat): void => {
		if (log === null) return;
		const line = document.createElement("div");
		line.textContent = `${chat.seq} ${chat.from}: ${chat.text}`;
		log.append(line);
		log.scrollTop = log.scrollHeight;
	};
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
		setTimeout(() => shown.remove(), 2000);
	};
	for (const button of document.querySelectorAll<HTMLButtonElement>(
		"button[data-reaction]",
	)) {
		button.addEventListener("click", () => {
			socket.send(
				JSON.stringify({ type: "reaction", emoji: button.dataset.reaction }),
			);
		});
	}
	let latest: Broadcast | null = null;
	/** Who waits for a token, in the order they asked: the room answers in that order. */
	const tokens: ((token: string | null) => void)[] = [];
	const whenOpen = (send: () => void): void => {
		if (socket.readyState === WebSocket.OPEN) send();
		else socket.addEventListener("open", send, { once: true });
	};
	let first: (broadcast: Broadcast | null) => void = () => {};
	const broadcast = new Promise<Broadcast | null>((resolve) => {
		first = resolve;
	});
	socket.addEventListener("message", (event) => {
		const message: RoomMessage = JSON.parse(String(event.data));
		if (message.type === "hello") {
			message.history.forEach(show);
			first(message.broadcast);
		}
		if (message.type === "hello" || message.type === "broadcast") {
			latest = message.broadcast;
		}
		if (message.type === "chat") show(message);
		if (message.type === "reaction") react(message.emoji, 1);
		if (message.type === "reactions") {
			for (const [emoji, count] of Object.entries(message.counts)) {
				react(emoji, count);
			}
		}
		if (message.type === "token") {
			const waiting = tokens.shift();
			waiting?.(message.token);
		}
		if ((message.type === "hello" || message.type === "connected") && count !== null) {
			count.textContent = `room “${name}”: ${message.connected} connected`;
			document.body.dataset.connected = String(message.connected);
		}
	});
	socket.addEventListener("error", () => first(null));
	const form = document.querySelector("form");
	form?.addEventListener("submit", (event) => {
		event.preventDefault();
		const input = form.querySelector("input");
		if (input === null || input.value === "") return;
		socket.send(JSON.stringify({ type: "chat", text: input.value }));
		input.value = "";
	});
	return {
		broadcast,
		current: () => latest,
		publish(where) {
			whenOpen(() =>
				socket.send(JSON.stringify({ type: "broadcast", broadcast: where })),
			);
		},
		async token(role, namespace) {
			if (new URLSearchParams(location.search).get("token") === "no") {
				return undefined;
			}
			const text = await new Promise<string | null>((resolve) => {
				tokens.push(resolve);
				whenOpen(() =>
					socket.send(JSON.stringify({ type: "token-for", role, namespace })),
				);
			});
			if (text === null) return undefined;
			// §10.2.2: Alias Type USE_VALUE (0x3), Token Type 0, the value.
			return new Uint8Array([0x03, 0x00, ...new TextEncoder().encode(text)]);
		},
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
