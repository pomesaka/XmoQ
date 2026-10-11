// A room as a Durable Object: where its broadcast is, a chat, and how many
// are connected. A spike: run locally with Miniflare (run.mjs), never
// deployed.
//
//   GET /rooms/<room>             {broadcast, connected}
//   GET /rooms/<room>/ws?name=N   a WebSocket: the chat and the count
//
// **A broadcast is registered on the WebSocket, and lasts as long as that
// WebSocket**: the publisher sends {type: "broadcast", broadcast:
// {relayUrl, namespace, certSha256Hex}}, and when its socket closes the
// room has no broadcast any more. Provisional: nothing is stored that could
// outlive a publisher that vanished.
//
// On the WebSocket the room sends {type: "hello", history, connected,
// broadcast} first, then {type: "chat", seq, from, text, at} for every
// message of anybody, {type: "connected", connected} when the count
// changes, and {type: "broadcast", broadcast} when the publisher registers
// or leaves. A client sends {type: "chat", text}, and a publisher the
// message above.

// **Tokens, an experiment** (how the relay authorizes is undecided): where
// the Worker has the key it shares with the relay (`AUTH_KEY_HEX`), a
// client sends {type: "token-for", role: "pub" | "sub", namespace: [...]}
// and gets {type: "token", role, namespace, token}, or token null where
// there is no key. The token is the text the relay checks
// (crates/relay/src/auth.rs): `v1|<namespace>|<role>|<expiry>|<mac>`, the
// mac HMAC-SHA-256 over what stands before it. **Anybody who asks gets
// one**: who may publish in a room, and who may watch, is what would be
// decided here, and is not.

/** How long a token is good for, in seconds. */
const TOKEN_SECONDS = 600;

/** How many chat messages a room keeps for those who come later. */
const HISTORY = 50;

// The pages of the demo are served from another origin.
const CORS = {
	"access-control-allow-origin": "*",
	"access-control-allow-methods": "GET, PUT, DELETE, OPTIONS",
	"access-control-allow-headers": "content-type",
};
const json = (value, status = 200) =>
	new Response(JSON.stringify(value), {
		status,
		headers: { "content-type": "application/json", ...CORS },
	});

export default {
	async fetch(request, env) {
		if (request.method === "OPTIONS") return new Response(null, { headers: CORS });
		const match = /^\/rooms\/([^/]{1,64})(\/.*)?$/.exec(new URL(request.url).pathname);
		if (match === null) return json({ error: "no such path" }, 404);
		// One Durable Object for a room name, wherever the request comes in.
		const room = env.ROOMS.get(env.ROOMS.idFromName(decodeURIComponent(match[1])));
		return room.fetch(request);
	},
};

export class Room {
	constructor(ctx, env) {
		this.ctx = ctx;
		this.env = env;
	}

	/** The text of a token for the role in the namespace, or null without a key. */
	async token(role, namespace) {
		const hex = this.env.AUTH_KEY_HEX;
		if (typeof hex !== "string" || hex === "") return null;
		const key = await crypto.subtle.importKey(
			"raw",
			Uint8Array.from(hex.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16)),
			{ name: "HMAC", hash: "SHA-256" },
			false,
			["sign"],
		);
		const signed = `v1|${namespace.join("/")}|${role}|${Math.floor(Date.now() / 1000) + TOKEN_SECONDS}`;
		const mac = new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(signed)));
		return `${signed}|${Array.from(mac, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
	}

	async fetch(request) {
		const url = new URL(request.url);
		const rest = url.pathname.replace(/^\/rooms\/[^/]+/, "");
		if (rest === "/ws") {
			if (request.headers.get("upgrade") !== "websocket") {
				return json({ error: "a WebSocket is expected" }, 426);
			}
			const pair = new WebSocketPair();
			// WHY the hibernation API: the object can be put to sleep between
			// messages with its sockets open, and is not billed for waiting.
			// The name, and the broadcast of a publisher, are kept with the
			// socket, since nothing in memory is.
			this.ctx.acceptWebSocket(pair[1]);
			pair[1].serializeAttachment({ name: (url.searchParams.get("name") ?? "anonymous").slice(0, 32) });
			pair[1].send(
				JSON.stringify({
					type: "hello",
					history: (await this.ctx.storage.get("history")) ?? [],
					connected: this.ctx.getWebSockets().length,
					broadcast: this.broadcast(),
				}),
			);
			this.tellAll({ type: "connected", connected: this.ctx.getWebSockets().length });
			return new Response(null, { status: 101, webSocket: pair[0] });
		}
		if (rest === "" && request.method === "GET") {
			return json({
				broadcast: this.broadcast(),
				connected: this.ctx.getWebSockets().length,
			});
		}
		return json({ error: "no such path" }, 404);
	}

	/** The broadcast of the publisher that is connected, the latest where there are several; null without one. */
	broadcast(except) {
		const all = this.ctx
			.getWebSockets()
			.filter((socket) => socket !== except)
			.map((socket) => socket.deserializeAttachment()?.broadcast)
			.filter((broadcast) => broadcast != null);
		return all.sort((a, b) => b.since - a.since)[0] ?? null;
	}

	async webSocketMessage(socket, message) {
		let said;
		try {
			said = JSON.parse(typeof message === "string" ? message : "");
		} catch {
			return;
		}
		if (said?.type === "broadcast" && said.broadcast != null) {
			const broadcast = {
				relayUrl: String(said.broadcast.relayUrl),
				namespace: Array.isArray(said.broadcast.namespace) ? said.broadcast.namespace.map(String) : [],
				certSha256Hex: String(said.broadcast.certSha256Hex),
				since: Date.now(),
			};
			socket.serializeAttachment({ ...socket.deserializeAttachment(), broadcast });
			this.tellAll({ type: "broadcast", broadcast: this.broadcast() });
			return;
		}
		if (said?.type === "token-for" && (said.role === "pub" || said.role === "sub") && Array.isArray(said.namespace)) {
			const namespace = said.namespace.map(String);
			socket.send(JSON.stringify({ type: "token", role: said.role, namespace, token: await this.token(said.role, namespace) }));
			return;
		}
		if (said?.type !== "chat" || typeof said.text !== "string") return;
		// The object handles one event at a time, and its storage holds
		// other events back while a write is under way, so the numbers are
		// given in the order the messages are taken, without a gap.
		const seq = ((await this.ctx.storage.get("seq")) ?? 0) + 1;
		const chat = {
			type: "chat",
			seq,
			from: socket.deserializeAttachment()?.name ?? "anonymous",
			text: said.text.slice(0, 500),
			at: Date.now(),
		};
		const history = [...((await this.ctx.storage.get("history")) ?? []), chat].slice(-HISTORY);
		await this.ctx.storage.put({ seq, history });
		this.tellAll(chat);
	}

	webSocketClose(socket) {
		socket.close();
		// The socket that closes is still among them here.
		const connected = this.ctx.getWebSockets().filter((each) => each !== socket).length;
		this.tellAll({ type: "connected", connected }, socket);
		// A publisher that goes takes its broadcast with it.
		if (socket.deserializeAttachment()?.broadcast != null) {
			this.tellAll({ type: "broadcast", broadcast: this.broadcast(socket) }, socket);
		}
	}

	webSocketError(socket) {
		this.webSocketClose(socket);
	}

	tellAll(message, except) {
		const text = JSON.stringify(message);
		for (const socket of this.ctx.getWebSockets()) {
			if (socket === except) continue;
			try {
				socket.send(text);
			} catch {
				// It has gone; its close is on its way.
			}
		}
	}
}
