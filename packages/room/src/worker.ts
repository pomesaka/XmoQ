// A room as a Durable Object: where its broadcast is, how many are
// connected, and reactions. Run locally with Miniflare (`run.ts`),
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

import {
	MOST_IN_A_MEETING,
	participantId,
	relayOf,
	seatFor,
	tokenForNamespace,
	transcriptPrefix,
} from "./core/rules.ts";
import { keyOfSetting, type Role, TOKEN_SECONDS, tokenText } from "./token.ts";

/** The settings of the Worker (`run.ts` has what each is). */
interface Env {
	ROOMS: DurableObjectNamespace;
	AUTH_KEY_HEX?: string;
	REACTION_BATCH_MS?: string;
	RELAY_URL?: string;
	RELAY_CERT_SHA256_HEX?: string;
	TRANSCRIPT_PREFIX?: string;
	MEETING_TRANSCRIPT_PREFIX?: string;
}

/** Where a publisher said its broadcast is, and since when. */
interface Broadcast {
	relayUrl: string;
	namespace: string[];
	certSha256Hex: string;
	since: number;
}

/** What a socket of the room keeps, as it may sleep between messages. */
interface Attachment {
	/** The room's name: an object woken for a message is not told the URL its socket came by. */
	room: string;
	/** The participant it joined the meeting as. */
	meeting?: string;
	broadcast?: Broadcast;
}

const attachmentOf = (socket: WebSocket): Attachment => {
	const kept: Attachment | null = socket.deserializeAttachment();
	return kept ?? { room: "" };
};

// The pages of the demo are served from another origin.
const CORS = {
	"access-control-allow-origin": "*",
	"access-control-allow-methods": "GET, PUT, DELETE, OPTIONS",
	"access-control-allow-headers": "content-type",
};
const json = (value: unknown, status = 200): Response =>
	new Response(JSON.stringify(value), {
		status,
		headers: { "content-type": "application/json", ...CORS },
	});

export default {
	async fetch(request: Request, env: Env): Promise<Response> {
		if (request.method === "OPTIONS")
			return new Response(null, { headers: CORS });
		const match = /^\/rooms\/([^/]{1,64})(\/.*)?$/.exec(
			new URL(request.url).pathname,
		);
		const name = match?.[1];
		if (name === undefined) return json({ error: "no such path" }, 404);
		// One Durable Object for a room name, wherever the request comes in.
		const room = env.ROOMS.get(env.ROOMS.idFromName(decodeURIComponent(name)));
		return room.fetch(request);
	},
};

export class Room {
	private readonly ctx: DurableObjectState;
	private readonly env: Env;
	/** With `REACTION_BATCH_MS`: the reactions that wait, and the timer that sends them. */
	private pending:
		| { counts: Record<string, number>; sent: unknown[] }
		| undefined;
	private flushing: ReturnType<typeof setTimeout> | undefined;

	constructor(ctx: DurableObjectState, env: Env) {
		this.ctx = ctx;
		this.env = env;
	}

	/** The text of a token for the role in the namespace, or null without a key. */
	async token(
		role: Role,
		namespace: readonly string[],
	): Promise<string | null> {
		const key = keyOfSetting(this.env.AUTH_KEY_HEX);
		if (key === undefined) return null;
		return tokenText(
			key,
			namespace,
			role,
			Math.floor(Date.now() / 1000) + TOKEN_SECONDS,
		);
	}

	async fetch(request: Request): Promise<Response> {
		const url = new URL(request.url);
		const rest = url.pathname.replace(/^\/rooms\/[^/]+/, "");
		if (rest === "/ws") {
			if (request.headers.get("upgrade") !== "websocket") {
				return json({ error: "a WebSocket is expected" }, 426);
			}
			const [client, server] = Object.values(new WebSocketPair());
			if (client === undefined || server === undefined) {
				return json({ error: "no WebSocket" }, 500);
			}
			// WHY the hibernation API: the object can be put to sleep between
			// messages with its sockets open, and is not billed for waiting.
			// The broadcast of a publisher is kept with the socket, since
			// nothing in memory is.
			this.ctx.acceptWebSocket(server);
			const room = decodeURIComponent(url.pathname.split("/")[2] ?? "");
			server.serializeAttachment({ room } satisfies Attachment);
			server.send(
				JSON.stringify({
					type: "hello",
					connected: this.ctx.getWebSockets().length,
					broadcast: this.broadcast(),
					relay: relayOf(this.env.RELAY_URL, this.env.RELAY_CERT_SHA256_HEX),
					transcript: transcriptPrefix(this.env.TRANSCRIPT_PREFIX, room),
					meetingTranscript: transcriptPrefix(
						this.env.MEETING_TRANSCRIPT_PREFIX,
						room,
					),
				}),
			);
			this.tellAll({
				type: "connected",
				connected: this.ctx.getWebSockets().length,
			});
			return new Response(null, { status: 101, webSocket: client });
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
	broadcast(except?: WebSocket): Broadcast | null {
		const all = this.ctx
			.getWebSockets()
			.filter((socket) => socket !== except)
			.map((socket) => attachmentOf(socket).broadcast)
			.filter((broadcast) => broadcast !== undefined);
		return all.sort((a, b) => b.since - a.since)[0] ?? null;
	}

	async webSocketMessage(
		socket: WebSocket,
		message: string | ArrayBuffer,
	): Promise<void> {
		let said: unknown;
		try {
			said = JSON.parse(typeof message === "string" ? message : "");
		} catch {
			return;
		}
		if (typeof said !== "object" || said === null || !("type" in said)) return;
		if (said.type === "broadcast" && "broadcast" in said) {
			const given = said.broadcast;
			if (typeof given !== "object" || given === null) return;
			const field = (name: string): unknown =>
				name in given
					? Object.getOwnPropertyDescriptor(given, name)?.value
					: undefined;
			const namespace = field("namespace");
			const broadcast: Broadcast = {
				relayUrl: String(field("relayUrl")),
				namespace: Array.isArray(namespace) ? namespace.map(String) : [],
				certSha256Hex: String(field("certSha256Hex")),
				since: Date.now(),
			};
			socket.serializeAttachment({
				...attachmentOf(socket),
				broadcast,
			} satisfies Attachment);
			this.tellAll({ type: "broadcast", broadcast: this.broadcast() });
			return;
		}
		if (
			said.type === "token-for" &&
			"role" in said &&
			(said.role === "pub" || said.role === "sub") &&
			"namespace" in said &&
			Array.isArray(said.namespace)
		) {
			const namespace = tokenForNamespace(said.namespace);
			if (namespace === undefined) return;
			socket.send(
				JSON.stringify({
					type: "token",
					role: said.role,
					namespace,
					token: await this.token(said.role, namespace),
				}),
			);
			return;
		}
		if (said.type === "join-meeting") {
			await this.joinMeeting(
				socket,
				"participant" in said ? said.participant : undefined,
			);
			return;
		}
		if (
			said.type === "reaction" &&
			"emoji" in said &&
			typeof said.emoji === "string"
		) {
			// A reaction is shown and forgotten: it has no number and is not
			// kept for those who come later. At most 8 UTF-16 units: an emoji
			// with its modifiers, and no text.
			const emoji = said.emoji.slice(0, 8);
			const sent = "sent" in said ? said.sent : undefined;
			const batchMs = Number(this.env.REACTION_BATCH_MS ?? 0);
			if (!(batchMs > 0)) {
				// As it comes, to everybody, the sender among them.
				this.tellAll({ type: "reaction", emoji, sent });
				return;
			}
			// `REACTION_BATCH_MS` (a trial, for measuring; 0 without it): the
			// reactions of that long go out as one message, each emoji with
			// how many. WHY a timer and not an alarm: an alarm is a write to
			// storage a reaction is not worth. The object stays awake for the
			// timer, which is that long at most.
			this.pending ??= { counts: {}, sent: [] };
			this.pending.counts[emoji] = (this.pending.counts[emoji] ?? 0) + 1;
			if (sent !== undefined) this.pending.sent.push(sent);
			this.flushing ??= setTimeout(() => {
				const waited = this.pending;
				this.pending = undefined;
				this.flushing = undefined;
				if (waited === undefined) return;
				this.tellAll({
					type: "reactions",
					counts: waited.counts,
					sent: waited.sent,
				});
			}, batchMs);
		}
	}

	/** Answers join-meeting from `socket` for the participant `id`, or leaves it unanswered (`seatFor`). */
	async joinMeeting(socket: WebSocket, id: unknown): Promise<void> {
		const attachment = attachmentOf(socket);
		const others = this.ctx
			.getWebSockets()
			.filter((each) => each !== socket)
			.map((each) => attachmentOf(each).meeting);
		const answer = seatFor(id, attachment.room, attachment.meeting, others);
		switch (answer.type) {
			case "unanswered":
				return;
			case "full":
				socket.send(
					JSON.stringify({ type: "meeting-full", most: MOST_IN_A_MEETING }),
				);
				return;
			case "seat": {
				// WHY before the tokens are made: making one waits, and another
				// join-meeting can be read meanwhile; it has to count this one.
				if (answer.counted && participantId(id)) {
					socket.serializeAttachment({
						...attachment,
						meeting: id,
					} satisfies Attachment);
				}
				socket.send(
					JSON.stringify({
						type: "meeting",
						participant: id,
						publish: {
							namespace: answer.publish,
							token: await this.token("pub", answer.publish),
						},
						subscribe: {
							namespace: answer.subscribe,
							token: await this.token("sub", answer.subscribe),
						},
					}),
				);
			}
		}
	}

	webSocketClose(socket: WebSocket): void {
		socket.close();
		// The socket that closes is still among them here.
		const connected = this.ctx
			.getWebSockets()
			.filter((each) => each !== socket).length;
		this.tellAll({ type: "connected", connected }, socket);
		// A publisher that goes takes its broadcast with it.
		if (attachmentOf(socket).broadcast !== undefined) {
			this.tellAll(
				{ type: "broadcast", broadcast: this.broadcast(socket) },
				socket,
			);
		}
	}

	webSocketError(socket: WebSocket): void {
		this.webSocketClose(socket);
	}

	tellAll(message: unknown, except?: WebSocket): void {
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
