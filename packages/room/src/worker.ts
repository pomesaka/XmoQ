// The rooms, asked over HTTP with JSON (decided on 2026-10-26): where a
// room's relay is, tokens for that relay, and the seats of a meeting. Run
// locally with Miniflare (`run.ts`), never deployed. What happens in a room
// as it goes (a broadcast that begins, the chat, who is there, reactions)
// goes over MoQ, and not through the room (decided on 2026-10-26): WHY, a
// client that is told it by the room would have to keep a second
// connection beside its session, and the relay already says it (a SUBSCRIBE
// waits for its publisher, `RENDEZVOUS_TIMEOUT`). The forms of the
// requests and the answers are `fixtures/room-api/README.md`.
//
// **The room of a broadcast**, under `/broadcasts/<name>`, is the same
// answer for everybody, and has no Durable Object: GET says its relay, the
// namespace of its one broadcast, `xmoq/broadcast/<name>`, and the prefix
// of its transcript; POST `tokens` gives a token. WHY one broadcast a room,
// at a namespace of the room's name (decided on 2026-10-26): a viewer
// knows where to subscribe before anybody publishes, and the room keeps
// nothing a publisher that vanished could leave behind. WHY NOT a
// publisher telling the room where it publishes: the room would believe
// the page, and what it was told would last as long as a connection to
// the room, which every client would have to keep open.
//
// **The relay of the rooms** (decided on 2026-10-25): `relay` is {url,
// certSha256Hex} where the Worker has `RELAY_URL` and
// `RELAY_CERT_SHA256_HEX`, and null where it has not. WHY one for the
// rooms, and NOT a flag of each client: whoever uses a room would have to
// know the relay as well.
//
// **The transcripts of a room** (decided on 2026-10-25): `transcript` is the
// prefix a transcriber listens under for the room's broadcast, its fields,
// from `TRANSCRIPT_PREFIX` (`xmoq/broadcast/{room}`), and for a meeting
// from `MEETING_TRANSCRIPT_PREFIX` (`xmoq/meet/{room}`); `{room}` is put
// in the room's name, and each is null without its setting. A transcript
// is `speech` of `xmoq/transcript/<prefix>` (`crates/transcript-tracks`).
// WHY NOT fixed as the room's namespace: which prefix a transcriber listens
// under is how it is run, one a room or one for every room
// (`xmoq/broadcast`), and the room cannot tell which; fixed, it would name
// a track that is not there for the second. The room does not know
// whether a transcriber runs: a reader's SUBSCRIBE waits for one
// (`RENDEZVOUS_TIMEOUT`).

// **Tokens, an experiment** (how the relay authorizes is undecided): where
// the Worker has the key it shares with the relay (`AUTH_KEY_HEX`), POST
// `tokens` with {role: "pub" | "sub", namespace: [...]} is answered with
// {token}, null where there is no key. The token is the text the relay
// checks (crates/relay/src/auth.rs): `v1|<namespace>|<role>|<expiry>|<mac>`,
// the mac HMAC-SHA-256 over what stands before it. **Only in the
// namespaces of the room** (decided on 2026-10-26): its broadcast, its
// chat and its transcript (`tokenForNamespace` has the table and why); any
// other ask is answered with token null, a meeting's namespaces (`xmoq/meet`
// and under it) among them, which its seats' are. **Anybody who asks gets
// one** there: who may publish in a room, and who may watch, is what would
// be decided here, and is not.

// **A meeting** (decided on 2026-10-25; over HTTP since 2026-10-26) is a
// room of its own, `MeetingRoom`, under `/meetings/<name>`: GET for its
// relay, the prefix of its participants' namespaces, the prefix of its
// transcript and how many seats it has (MOST_IN_A_MEETING), POST
// `tokens` for a token in its chat or its transcript, and the seats: POST
// `seats` with {participant} gives a seat, {participant, publish: {namespace:
// ["xmoq", "meet", <name>, <id>], token}, subscribe: {namespace: ["xmoq",
// "meet", <name>, "*"], token}, lease, expiresAt, renewMs}, token null
// without a key; beyond MOST_IN_A_MEETING 409 {full: {most}}, and 400, 404
// or 409 {refused: {why}} (`SeatRefusal`). POST `seats/<id>/renew` with
// {lease} renews it (404 {gone: true} where it ran out), DELETE
// `seats/<id>` with {lease} frees it. The forms are
// `fixtures/room-api/README.md`; why a seat holds by its lease, at
// `seatFor`. WHY the room counts and not the relay: the room hands out the
// tokens, and one it does not give is a participant the relay never sees;
// the relay knows namespaces and sessions, not meetings, and would have to
// be told which prefix is one and how many it takes. Who is in the meeting
// is not the room's to say: the participants find each other by
// SUBSCRIBE_NAMESPACE at the relay.

import {
	meetingPrefix,
	participantNamespace,
	seatSubscribeNamespace,
} from "@xmoq/meeting/names";
import {
	type HeldSeat,
	holdsSeat,
	MOST_IN_A_MEETING,
	type RoomNamespaces,
	relayOf,
	seatFor,
	seatTimesOf,
	tokenForNamespace,
	transcriptPrefix,
} from "./core/rules.ts";
import { keyOfSetting, type Role, TOKEN_SECONDS, tokenText } from "./token.ts";

/** The settings of the Worker (`run.ts` has what each is). */
interface Env {
	MEETINGS: DurableObjectNamespace;
	AUTH_KEY_HEX?: string;
	RELAY_URL?: string;
	RELAY_CERT_SHA256_HEX?: string;
	TRANSCRIPT_PREFIX?: string;
	MEETING_TRANSCRIPT_PREFIX?: string;
	MEETING_SEAT_MS?: string;
	MEETING_RENEW_MS?: string;
}

// The pages of the demo are served from another origin.
const CORS = {
	"access-control-allow-origin": "*",
	"access-control-allow-methods": "GET, POST, DELETE, OPTIONS",
	"access-control-allow-headers": "content-type",
};
const json = (value: unknown, status = 200): Response =>
	new Response(JSON.stringify(value), {
		status,
		headers: { "content-type": "application/json", ...CORS },
	});

/** The relay of the rooms, as an answer has it. */
const relayAnswer = (
	env: Env,
): { url: string; certSha256Hex: string } | null => {
	const relay = relayOf(env.RELAY_URL, env.RELAY_CERT_SHA256_HEX);
	return relay === null
		? null
		: { url: relay.relayUrl, certSha256Hex: relay.certSha256Hex };
};

export default {
	async fetch(request: Request, env: Env): Promise<Response> {
		if (request.method === "OPTIONS")
			return new Response(null, { headers: CORS });
		const match = /^\/(broadcasts|meetings)\/([^/]{1,64})(\/.*)?$/.exec(
			new URL(request.url).pathname,
		);
		const kind = match?.[1];
		const name = match?.[2];
		if (kind === undefined || name === undefined) {
			return json({ error: "no such path" }, 404);
		}
		if (kind === "broadcasts") {
			return broadcastRoom(
				request,
				env,
				decodeURIComponent(name),
				match?.[3] ?? "",
			);
		}
		// One Durable Object for a meeting's name, wherever the request comes in.
		const meeting = env.MEETINGS.get(
			env.MEETINGS.idFromName(decodeURIComponent(name)),
		);
		return meeting.fetch(request);
	},
};

/** The room of a broadcast (the top of this file): the same answer for everybody. */
async function broadcastRoom(
	request: Request,
	env: Env,
	name: string,
	rest: string,
): Promise<Response> {
	const broadcast = ["xmoq", "broadcast", name];
	const transcript = transcriptPrefix(env.TRANSCRIPT_PREFIX, name);
	if (request.method === "GET" && rest === "") {
		return json({ relay: relayAnswer(env), namespace: broadcast, transcript });
	}
	if (request.method === "POST" && rest === "/tokens") {
		return tokenAnswer(env, await bodyOf(request), {
			chat: { kind: "broadcast", name },
			broadcast,
			transcripts: transcript === null ? [] : [transcript],
		});
	}
	return json({ error: "no such path" }, 404);
}

/** The answer to POST `tokens` of a room with `namespaces`. */
async function tokenAnswer(
	env: Env,
	body: unknown,
	namespaces: RoomNamespaces,
): Promise<Response> {
	if (
		!isObject(body) ||
		(body.role !== "pub" && body.role !== "sub") ||
		!Array.isArray(body.namespace)
	) {
		return json({ error: "a role and a namespace are expected" }, 400);
	}
	const namespace = tokenForNamespace(body.role, body.namespace, namespaces);
	return json({
		token:
			namespace === undefined ? null : await tokenOf(env, body.role, namespace),
	});
}

const isObject = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

/** The body of a request, read as JSON; undefined for none, or for one that is no JSON. */
async function bodyOf(request: Request): Promise<unknown> {
	try {
		return await request.json();
	} catch {
		return undefined;
	}
}

/** A new lease: 16 random bytes in hex, which nobody can guess. */
const newLease = (): string =>
	Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");

/** The text of a token for the role in the namespace, or null without a key. */
async function tokenOf(
	env: Env,
	role: Role,
	namespace: readonly string[],
): Promise<string | null> {
	const key = keyOfSetting(env.AUTH_KEY_HEX);
	if (key === undefined) return null;
	return tokenText(
		key,
		namespace,
		role,
		Math.floor(Date.now() / 1000) + TOKEN_SECONDS,
	);
}

/** The status of each refusal of a seat. */
const REFUSAL_STATUS = {
	"not-a-participant": 400,
	"not-a-meeting-room": 404,
	taken: 409,
} as const;

/** The key of a seat in the storage of the object. */
const SEAT_KEY = "seat:";

/**
 * The room of a meeting, asked over HTTP (the forms at the top of this
 * file). WHY the seats in the object's storage and not in its memory: an
 * object is put to sleep between requests that are seconds apart, its
 * memory with it, and a seat holds for a minute across renewals 20 s
 * apart (in Miniflare, every renewal of the meeting's pages was answered
 * `gone` while the seats were in its memory). A write for every seat and every renewal is the price.
 */
export class MeetingRoom {
	private readonly ctx: DurableObjectState;
	private readonly env: Env;

	constructor(ctx: DurableObjectState, env: Env) {
		this.ctx = ctx;
		this.env = env;
	}

	async fetch(request: Request): Promise<Response> {
		const url = new URL(request.url);
		const [name = "", ...rest] = url.pathname
			.split("/")
			.slice(2)
			.map(decodeURIComponent);
		const now = Date.now();
		const seats = new Map<string, HeldSeat>();
		const runOut: string[] = [];
		for (const [key, seat] of await this.ctx.storage.list<HeldSeat>({
			prefix: SEAT_KEY,
		})) {
			if (seat.expiresAt > now) seats.set(key.slice(SEAT_KEY.length), seat);
			else runOut.push(key);
		}
		if (runOut.length > 0) await this.ctx.storage.delete(runOut);
		const transcript = transcriptPrefix(
			this.env.MEETING_TRANSCRIPT_PREFIX,
			name,
		);
		const route = `${request.method} ${rest.length === 0 ? "" : rest[0]}${rest.length === 3 ? `/${rest[2]}` : ""}`;
		if (route === "GET " && rest.length === 0) {
			return json({
				relay: relayAnswer(this.env),
				namespace: meetingPrefix(name),
				transcript,
				most: MOST_IN_A_MEETING,
			});
		}
		const body = await bodyOf(request);
		if (route === "POST tokens" && rest.length === 1) {
			return tokenAnswer(this.env, body, {
				chat: { kind: "meeting", name },
				transcripts: transcript === null ? [] : [transcript],
			});
		}
		const times = seatTimesOf(
			this.env.MEETING_SEAT_MS,
			this.env.MEETING_RENEW_MS,
		);
		if (route === "POST seats" && rest.length === 1) {
			const id = isObject(body) ? body.participant : undefined;
			const answer = seatFor(id, name, seats, now);
			if (answer.type === "refused") {
				return json(
					{ refused: { why: answer.why } },
					REFUSAL_STATUS[answer.why],
				);
			}
			if (answer.type === "full") {
				return json({ full: { most: MOST_IN_A_MEETING } }, 409);
			}
			const seat = { lease: newLease(), expiresAt: now + times.seatMs };
			// WHY before the tokens are made: making one waits, and another
			// ask for a seat can be read meanwhile; it has to count this one.
			await this.ctx.storage.put(`${SEAT_KEY}${answer.publish[3] ?? ""}`, seat);
			return this.seatAnswer(
				answer.publish,
				answer.subscribe,
				seat,
				times.renewMs,
			);
		}
		const id = rest[1] ?? "";
		const lease = isObject(body) ? body.lease : undefined;
		if (route === "POST seats/renew" && rest.length === 3) {
			if (!holdsSeat(id, lease, seats, now)) return json({ gone: true }, 404);
			const seat = { lease: String(lease), expiresAt: now + times.seatMs };
			await this.ctx.storage.put(`${SEAT_KEY}${id}`, seat);
			return this.seatAnswer(
				participantNamespace(name, id),
				seatSubscribeNamespace(name),
				seat,
				times.renewMs,
			);
		}
		if (route === "DELETE seats" && rest.length === 2) {
			if (holdsSeat(id, lease, seats, now)) {
				await this.ctx.storage.delete(`${SEAT_KEY}${id}`);
			}
			// WHY 204 also for a seat that is gone: leaving is done either way.
			return new Response(null, { status: 204, headers: CORS });
		}
		return json({ error: "no such path" }, 404);
	}

	private async seatAnswer(
		publish: string[],
		subscribe: string[],
		seat: HeldSeat,
		renewMs: number,
	): Promise<Response> {
		return json({
			participant: publish[3],
			publish: {
				namespace: publish,
				token: await tokenOf(this.env, "pub", publish),
			},
			subscribe: {
				namespace: subscribe,
				token: await tokenOf(this.env, "sub", subscribe),
			},
			lease: seat.lease,
			expiresAt: seat.expiresAt,
			renewMs,
		});
	}
}
