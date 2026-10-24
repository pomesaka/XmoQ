// A room as a Durable Object: where its broadcast is, how many are
// connected, and reactions. A spike: run locally with Miniflare (run.mjs),
// never deployed. The chat is not here: it goes over MoQ tracks, through
// the gatherer `xmoq-chat` (Q36 and Q49, decided on 2026-10-24).
//
//   GET /rooms/<room>      {broadcast, connected}
//   GET /rooms/<room>/ws   a WebSocket: the count, the broadcast, reactions
//
// **A broadcast is registered on the WebSocket, and lasts as long as that
// WebSocket**: the publisher sends {type: "broadcast", broadcast:
// {relayUrl, namespace, certSha256Hex}}, and when its socket closes the
// room has no broadcast any more. Provisional: nothing is stored that could
// outlive a publisher that vanished.
//
// On the WebSocket the room sends {type: "hello", connected, broadcast}
// first, then {type: "connected", connected} when the count changes,
// {type: "broadcast", broadcast} when the publisher registers or leaves,
// and {type: "reaction", emoji} when somebody sent one (with
// REACTION_BATCH_MS, {type: "reactions", counts} for all of that while).
// A client sends {type: "reaction", emoji}, and a publisher the message
// above. Any other message is not answered.

// **Tokens, an experiment** (how the relay authorizes is undecided): where
// the Worker has the key it shares with the relay (`AUTH_KEY_HEX`), a
// client sends {type: "token-for", role: "pub" | "sub", namespace: [...]}
// and gets {type: "token", role, namespace, token}, or token null where
// there is no key; nothing for a namespace with a `*` or a `/` in a field. The token is the text the relay checks
// (crates/relay/src/auth.rs): `v1|<namespace>|<role>|<expiry>|<mac>`, the
// mac HMAC-SHA-256 over what stands before it. **Anybody who asks gets
// one**: who may publish in a room, and who may watch, is what would be
// decided here, and is not.

/** How long a token is good for, in seconds. */
const TOKEN_SECONDS = 600;

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
			// The broadcast of a publisher is kept with the socket, since
			// nothing in memory is.
			this.ctx.acceptWebSocket(pair[1]);
			pair[1].send(
				JSON.stringify({
					type: "hello",
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
			// The relay reads a last field `*` as every namespace under the
			// ones before it (Q50), and the token joins the fields with `/`,
			// so that ["xmoq/chat", "blue"] would sign xmoq/chat/blue: the
			// room makes neither for anybody who asks, and leaves the asking
			// unanswered.
			if (namespace.some((field) => field.includes("*") || field.includes("/"))) return;
			socket.send(JSON.stringify({ type: "token", role: said.role, namespace, token: await this.token(said.role, namespace) }));
			return;
		}
		if (said?.type === "reaction" && typeof said.emoji === "string") {
			// A reaction is shown and forgotten: it has no number and is not
			// kept for those who come later. At most 8 UTF-16 units: an emoji
			// with its modifiers, and no text.
			const emoji = said.emoji.slice(0, 8);
			const batchMs = Number(this.env.REACTION_BATCH_MS ?? 0);
			if (!(batchMs > 0)) {
				// As it comes, to everybody, the sender among them.
				this.tellAll({ type: "reaction", emoji, sent: said.sent });
				return;
			}
			// `REACTION_BATCH_MS` (a trial, for measuring; 0 without it): the
			// reactions of that long go out as one message, each emoji with
			// how many. WHY a timer and not an alarm: an alarm is a write to
			// storage a reaction is not worth. The object stays awake for the
			// timer, which is that long at most.
			this.pending ??= { counts: {}, sent: [] };
			this.pending.counts[emoji] = (this.pending.counts[emoji] ?? 0) + 1;
			if (said.sent !== undefined) this.pending.sent.push(said.sent);
			this.flushing ??= setTimeout(() => {
				const { counts, sent } = this.pending;
				this.pending = undefined;
				this.flushing = undefined;
				this.tellAll({ type: "reactions", counts, sent });
			}, batchMs);
			return;
		}
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
