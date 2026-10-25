// A room as a Durable Object: where its broadcast is, how many are
// connected, and reactions. A spike: run locally with Miniflare (run.mjs),
// never deployed. The chat is not here: it goes over MoQ tracks, through
// the gatherer `xmoq-chat` (decided on 2026-10-24).
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
// On the WebSocket the room sends {type: "hello", connected, broadcast,
// relay, transcript, meetingTranscript} first, then {type: "connected", connected} when the count changes,
// {type: "broadcast", broadcast} when the publisher registers or leaves,
// and {type: "reaction", emoji} when somebody sent one (with
// REACTION_BATCH_MS, {type: "reactions", counts} for all of that while).
// A client sends {type: "reaction", emoji}, token-for and join-meeting
// (below), and a publisher the message above. Any other message is not
// answered.
//
// **The relay of the rooms** (decided on 2026-10-25): `relay` of the hello
// is {relayUrl, certSha256Hex} where the Worker has `RELAY_URL` and
// `RELAY_CERT_SHA256_HEX`, and null where it has not. It is there with a
// broadcast and without one, so that a participant can take part in the
// chat (which is on the relay) before anybody broadcasts. WHY apart from
// the broadcast: in a meeting every participant has a namespace of its own,
// and no one broadcast says where the room's relay is. WHY NOT a flag of
// each client: whoever uses a room would have to know the relay as well.
//
// **The transcripts of a room** (decided on 2026-10-25): `transcript` of the
// hello is the prefix a transcriber listens under for the room's broadcast,
// its fields, from `TRANSCRIPT_PREFIX` (`xmoq/demo` for the demo's one
// broadcast); `meetingTranscript` the one for the room's meeting, whose
// participants publish under `xmoq/meet/<room>/<id>`, from
// `MEETING_TRANSCRIPT_PREFIX` (`xmoq/meet/{room}` for a transcriber of each
// room). In both `{room}` is put in the room's name, and each is null
// without its setting. A transcript is `speech` of
// `xmoq/transcript/<prefix>` (`crates/transcript-tracks`), which a reader
// follows: a reader of the meeting the one, any other the other. WHY two:
// a room has a broadcast and a meeting, which are under prefixes of their
// own, and a reader of one that took the other's would follow a transcript
// of nobody it reads. WHY NOT the meeting's fixed as `xmoq/meet/<room>`,
// though its namespaces are: which prefix a transcriber listens under is
// how it is run, one a room (`xmoq/meet/<room>`) or one for every room
// (`xmoq/meet`), and the room cannot tell which; fixed, it would name a
// track that is not there for the second. WHY from the
// Worker's settings, beside the relay: one transcriber listens under the
// prefix to every broadcast of the room, and the room says where those are.
// WHY NOT the transcriber telling the room over the WebSocket that it is
// there: a transcriber would join every room it listens to, and one
// listening under `xmoq/meet` hears many. WHY NOT each publisher saying it
// with its broadcast: in a meeting every participant would say the same.
// The room does not know whether a transcriber runs: a reader's SUBSCRIBE
// waits for one (`RENDEZVOUS_TIMEOUT`).

// **Tokens, an experiment** (how the relay authorizes is undecided): where
// the Worker has the key it shares with the relay (`AUTH_KEY_HEX`), a
// client sends {type: "token-for", role: "pub" | "sub", namespace: [...]}
// and gets {type: "token", role, namespace, token}, or token null where
// there is no key; nothing for a namespace with a `*` or a `/` in a field. The token is the text the relay checks
// (crates/relay/src/auth.rs): `v1|<namespace>|<role>|<expiry>|<mac>`, the
// mac HMAC-SHA-256 over what stands before it. **Anybody who asks gets
// one**: who may publish in a room, and who may watch, is what would be
// decided here, and is not. Except in a meeting's namespaces (`xmoq/meet`
// and under it), which token-for leaves unanswered: they are join-meeting's.

// **A meeting** (decided on 2026-10-25): a participant sends
// {type: "join-meeting", participant: "<id>"}, the id a random one the page
// makes; within MOST_IN_A_MEETING the room answers {type: "meeting",
// participant, publish: {namespace: ["xmoq", "meet", <room>, <id>], token},
// subscribe: {namespace: ["xmoq", "meet", <room>, "*"], token}}, token null
// without a key, and beyond it {type: "meeting-full", most}. Those counted
// are the sockets that joined, until they close; the same socket asking
// again with its id is answered again, with new tokens, full or not, since
// it is counted already. Not answered: an id that is empty,
// longer than 64, or has a `/` or a `*`; another id on a socket that joined;
// an id another open socket has; a room whose name has a `/` or a `*`.
// WHY here and not at the relay: the room hands out the tokens, and one it
// does not give is a participant the relay never sees; the relay knows
// namespaces and sessions, not meetings, and would have to be told which
// prefix is one and how many it takes. WHY the sockets and not the
// namespaces the relay announces: a token outlives the socket for its 600 s,
// so a participant that left the room may stay at the relay that long; the
// room counts who it would give a token to now.

/** How long a token is good for, in seconds. */
const TOKEN_SECONDS = 600;

/** The participants of a meeting, the agents among them (decided on 2026-10-25). */
const MOST_IN_A_MEETING = 49;

/** Whether the namespace is a meeting's: `xmoq/meet` or under it. */
const meetingNamespace = (namespace) => namespace[0] === "xmoq" && namespace[1] === "meet";

/** Whether `id` can be the last field of a participant's namespace. */
const participantId = (id) => typeof id === "string" && id !== "" && id.length <= 64 && !id.includes("/") && !id.includes("*");

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
			const room = decodeURIComponent(url.pathname.split("/")[2] ?? "");
			// The room's name, for join-meeting: an object woken for a message
			// is not told the URL its socket came by.
			pair[1].serializeAttachment({ room });
			pair[1].send(
				JSON.stringify({
					type: "hello",
					connected: this.ctx.getWebSockets().length,
					broadcast: this.broadcast(),
					relay: this.relay(),
					transcript: this.transcript(this.env.TRANSCRIPT_PREFIX, room),
					meetingTranscript: this.transcript(this.env.MEETING_TRANSCRIPT_PREFIX, room),
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

	/** Where the relay of the rooms is, or null where the Worker was not told. */
	relay() {
		const relayUrl = this.env.RELAY_URL;
		const certSha256Hex = this.env.RELAY_CERT_SHA256_HEX;
		if (typeof relayUrl !== "string" || relayUrl === "" || typeof certSha256Hex !== "string" || certSha256Hex === "") return null;
		return { relayUrl, certSha256Hex };
	}

	/**
	 * The prefix a transcriber listens under for the room named `room`, its
	 * fields, from the setting `prefix`, or null where the Worker was not
	 * told. Null as well where a
	 * field would be empty or have a `*`, which the relay reads as every
	 * namespace under the ones before it: a room's name with a `/` or a
	 * `*` makes no prefix.
	 */
	transcript(prefix, room) {
		if (typeof prefix !== "string" || prefix === "") return null;
		if (room.includes("/") || room.includes("*")) return null;
		const fields = prefix.replaceAll("{room}", room).split("/");
		if (fields.some((field) => field === "" || field.includes("*"))) return null;
		return fields;
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
			// ones before it, and the token joins the fields with `/`,
			// so that ["xmoq/chat", "blue"] would sign xmoq/chat/blue: the
			// room makes neither for anybody who asks, and leaves the asking
			// unanswered.
			if (namespace.some((field) => field.includes("*") || field.includes("/"))) return;
			if (meetingNamespace(namespace)) return;
			socket.send(JSON.stringify({ type: "token", role: said.role, namespace, token: await this.token(said.role, namespace) }));
			return;
		}
		if (said?.type === "join-meeting") {
			await this.joinMeeting(socket, said.participant);
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

	/** Answers join-meeting from `socket` for the participant `id`, or leaves it unanswered (at the top). */
	async joinMeeting(socket, id) {
		const attachment = socket.deserializeAttachment() ?? {};
		const room = attachment.room ?? "";
		if (!participantId(id) || room === "" || room.includes("/") || room.includes("*")) return;
		if (attachment.meeting !== undefined && attachment.meeting !== id) return;
		const others = this.ctx.getWebSockets().filter((each) => each !== socket);
		if (others.some((each) => each.deserializeAttachment()?.meeting === id)) return;
		if (attachment.meeting === undefined) {
			const counted = others.filter((each) => each.deserializeAttachment()?.meeting !== undefined).length;
			if (counted >= MOST_IN_A_MEETING) {
				socket.send(JSON.stringify({ type: "meeting-full", most: MOST_IN_A_MEETING }));
				return;
			}
			// WHY before the tokens are made: making one waits, and another
			// join-meeting can be read meanwhile; it has to count this one.
			socket.serializeAttachment({ ...attachment, meeting: id });
		}
		const publish = ["xmoq", "meet", room, id];
		const subscribe = ["xmoq", "meet", room, "*"];
		socket.send(
			JSON.stringify({
				type: "meeting",
				participant: id,
				publish: { namespace: publish, token: await this.token("pub", publish) },
				subscribe: { namespace: subscribe, token: await this.token("sub", subscribe) },
			}),
		);
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
