// What a room answers, read: each reader takes the status and the body of
// one kind of answer and gives its value, or undefined for one it does not
// take: a status the request does not give, a field missing or of another
// type, a relay that is not https or whose hash is not 64 hex digits, a
// namespace with no field or an empty one. The forms are
// `fixtures/room-api/README.md`; the Rust client reads the same table, as
// strictly (decided on 2026-10-26).

/** Where the relay of a room is. */
export interface RelayOfARoom {
	url: string;
	/** The SHA-256 of its certificate, in hex, which a client pins. */
	certSha256Hex: string;
}

/** The room of a broadcast. */
export interface BroadcastRoom {
	/** Null where the room was not told. */
	relay: RelayOfARoom | null;
	/** The namespace of the room's one broadcast, its fields. */
	namespace: string[];
	/** The prefix a transcriber listens under for it, its fields; null for none. */
	transcript: string[] | null;
}

/** The room of a meeting. */
export interface MeetingRoom {
	relay: RelayOfARoom | null;
	/** The prefix of the participants' namespaces, its fields. */
	namespace: string[];
	transcript: string[] | null;
	/** How many seats the meeting has. */
	most: number;
}

/** A seat in a meeting, taken or renewed. */
export interface Seat {
	participant: string;
	/** `pub` in the participant's namespace; null where the room has no key. */
	publishToken: string | null;
	/** `sub` in the namespaces of the meeting. */
	subscribeToken: string | null;
	/** What the seat is renewed and left with. */
	lease: string;
	/** When the seat runs out unless it is renewed, in milliseconds since the Unix epoch. */
	expiresAt: number;
	/** How often the room asks for it to be renewed, in milliseconds. */
	renewMs: number;
}

/** What asking for a seat, or renewing one, gets. */
export type SeatAnswer =
	| { type: "seat"; seat: Seat }
	| { type: "full"; most: number }
	| { type: "refused"; why: SeatRefusal }
	/** A seat that ran out, or was never there, cannot be renewed. */
	| { type: "gone" };

/** Why a room gives no seat, other than a full meeting. */
export type SeatRefusal = "not-a-participant" | "not-a-meeting-room" | "taken";

const SEAT_REFUSALS: readonly SeatRefusal[] = [
	"not-a-participant",
	"not-a-meeting-room",
	"taken",
];

const isObject = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

/** The fields of a namespace: one at least, none of them empty. */
const fieldsOf = (value: unknown): string[] | undefined =>
	Array.isArray(value) &&
	value.length > 0 &&
	value.every((field) => typeof field === "string" && field !== "")
		? value.map(String)
		: undefined;

const textOrNull = (value: unknown): string | null | undefined =>
	value === null ? null : typeof value === "string" ? value : undefined;

const wholeNumber = (value: unknown): number | undefined =>
	typeof value === "number" && Number.isSafeInteger(value) && value >= 0
		? value
		: undefined;

function relayOf(value: unknown): RelayOfARoom | null | undefined {
	if (value === null) return null;
	if (!isObject(value)) return undefined;
	const { url, certSha256Hex } = value;
	if (typeof url !== "string" || typeof certSha256Hex !== "string") {
		return undefined;
	}
	// WHY https alone: a relay over WebTransport stands on HTTPS, and the
	// client pins the hash of its certificate.
	if (!url.startsWith("https://") || !/^[0-9a-fA-F]{64}$/.test(certSha256Hex)) {
		return undefined;
	}
	return { url, certSha256Hex };
}

function prefixOf(value: unknown): string[] | null | undefined {
	return value === null ? null : fieldsOf(value);
}

/** The answer to `GET /broadcasts/<name>`, which is 200. */
export function readBroadcastRoom(
	status: number,
	body: unknown,
): BroadcastRoom | undefined {
	if (status !== 200 || !isObject(body)) return undefined;
	const relay = relayOf(body.relay);
	const namespace = fieldsOf(body.namespace);
	const transcript = prefixOf(body.transcript);
	if (relay === undefined || namespace === undefined) return undefined;
	if (transcript === undefined) return undefined;
	return { relay, namespace, transcript };
}

/** The answer to `GET /meetings/<name>`, which is 200. */
export function readMeetingRoom(
	status: number,
	body: unknown,
): MeetingRoom | undefined {
	const room = readBroadcastRoom(status, body);
	if (room === undefined || !isObject(body)) return undefined;
	const most = wholeNumber(body.most);
	if (most === undefined) return undefined;
	return { ...room, most };
}

function seatOf(body: Record<string, unknown>): Seat | undefined {
	const { participant, publish, subscribe, lease } = body;
	if (typeof participant !== "string" || typeof lease !== "string") {
		return undefined;
	}
	if (!isObject(publish) || !isObject(subscribe)) return undefined;
	const publishToken = textOrNull(publish.token);
	const subscribeToken = textOrNull(subscribe.token);
	const expiresAt = wholeNumber(body.expiresAt);
	const renewMs = wholeNumber(body.renewMs);
	if (publishToken === undefined || subscribeToken === undefined) {
		return undefined;
	}
	if (expiresAt === undefined || renewMs === undefined) return undefined;
	return {
		participant,
		publishToken,
		subscribeToken,
		lease,
		expiresAt,
		renewMs,
	};
}

/**
 * The answer to asking for a seat (`take`: 200 a seat, 409 full or refused,
 * 400 or 404 refused) or to renewing one (`renew`: 200 a seat, 404 gone).
 */
export function readSeatAnswer(
	request: "take" | "renew",
	status: number,
	body: unknown,
): SeatAnswer | undefined {
	if (!isObject(body)) return undefined;
	if (status === 200) {
		const seat = seatOf(body);
		return seat === undefined ? undefined : { type: "seat", seat };
	}
	if (request === "renew") {
		return status === 404 && body.gone === true ? { type: "gone" } : undefined;
	}
	if (status === 409 && isObject(body.full)) {
		const most = wholeNumber(body.full.most);
		return most === undefined ? undefined : { type: "full", most };
	}
	if (
		(status === 400 || status === 404 || status === 409) &&
		isObject(body.refused)
	) {
		const { why } = body.refused;
		const refusal = SEAT_REFUSALS.find((each) => each === why);
		return refusal === undefined
			? undefined
			: { type: "refused", why: refusal };
	}
	return undefined;
}

/** The answer to asking for a token, which is 200: the token, null for none. */
export function readToken(
	status: number,
	body: unknown,
): { token: string | null } | undefined {
	if (status !== 200 || !isObject(body)) return undefined;
	const token = textOrNull(body.token);
	return token === undefined ? undefined : { token };
}
